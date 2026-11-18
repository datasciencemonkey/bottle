/**
 * #123: the brain side of the hub Forget dialog (MEM-32a, MEM-32c, MEM-32d, G-45b, U9, ARC-5).
 * The flow is a double; every value here is fake.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import type { ForgetDialogModel } from '../shared/forget-view';
import { asForgetDialogModel } from '../shared/forget-view';
import { BrainBus, type ViewInput } from './bus';
import type { ForgetScope } from './components/memory-service';
import { ForgetDialogController, wireForget } from './forget-wiring';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const MOG = '0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01';
const OTHER = '6a7f0c2e-91d4-4b8e-8f1e-2c4b5d6e7f02';
const CID = '01f0aaaa-test-not-real-0001';
const REVIEW = 'https://acme.cloud.databricks.com/one?o=1001';
const THREAD = `https://acme.cloud.databricks.com/one/chat/threads/${CID}?o=1001`;

const confirmModel = (mogId: string): ForgetDialogModel => ({
  mogId,
  scope: 'mog',
  phase: 'confirm',
  title: 'Forget this MOG',
  text: 'Delete all of this MOG on this computer? You cannot undo this.',
  memoryReview: {
    label: 'Review Genie One memories',
    text: 'Genie One keeps its own memory.',
    url: REVIEW,
  },
});

function rig(over: { confirm?: (s: ForgetScope) => Promise<ForgetDialogModel> } = {}) {
  const sent: (ForgetDialogModel | null)[] = [];
  const opened: string[] = [];
  const confirms: ForgetScope[] = [];
  const flow = {
    dialog: (s: ForgetScope) => {
      if (s.mogId === undefined || !/^[0-9a-f-]{36}$/.test(s.mogId)) throw new Error('bad scope');
      return confirmModel(s.mogId);
    },
    confirm:
      over.confirm ??
      (async (s: ForgetScope) => {
        confirms.push(s);
        return {
          ...confirmModel(s.mogId ?? ''),
          phase: 'done' as const,
          result: {
            local: 9,
            deletedConversations: 0,
            notice: {
              mogId: s.mogId ?? '',
              label: 'Delete in Genie One',
              text: 'Bottle cannot delete Genie One conversations yet.',
              links: [{ conversationId: CID, url: THREAD }],
              noLink: 0,
            },
          },
        };
      }),
  };
  const c = new ForgetDialogController({
    flow,
    send: async (m) => {
      sent.push(m);
    },
    openLink: async (u) => {
      opened.push(u);
    },
  });
  const click = (payload: unknown, mogId = MOG, from = 'hub'): Promise<void> =>
    c.onInput({ from, mogId, kind: 'click', payload } as ViewInput);
  return { c, sent, opened, confirms, click };
}

describe('Forget dialog wiring (#123)', () => {
  it('[MEM-32a] the hub Forget memory click opens the confirm dialog for that MOG', async () => {
    const r = rig();
    await r.click({ target: 'forget_open' });
    expect(r.sent).toEqual([confirmModel(MOG)]);
    expect(r.confirms).toEqual([]);
  });

  it('[MEM-32a] [U9] confirm runs Forget on the scope the brain opened, then sends the done dialog', async () => {
    const r = rig();
    await r.click({ target: 'forget_open' });
    await r.click({ target: 'forget_confirm', scope: 'mog' });
    expect(r.confirms).toEqual([{ mogId: MOG }]);
    expect(r.sent[1]?.phase).toBe('done');
    expect(r.sent[1]?.result?.local).toBe(9);
    // A second confirm on the done dialog does nothing.
    await r.click({ target: 'forget_confirm', scope: 'mog' });
    expect(r.confirms).toHaveLength(1);
  });

  it('[MEM-32a] [U9] nothing is forgotten without an open dialog, a matching MOG and scope, or from a MOG view', async () => {
    const r = rig();
    await r.click({ target: 'forget_confirm', scope: 'mog' });
    await r.click({ target: 'forget_open' }, MOG, `mog-${MOG}`);
    await r.click({ target: 'forget_open' }, 'hub');
    expect(r.sent).toEqual([]);
    await r.click({ target: 'forget_open' });
    await r.click({ target: 'forget_confirm', scope: 'session' });
    await r.click({ target: 'forget_confirm', scope: 'mog' }, OTHER);
    expect(r.confirms).toEqual([]);
  });

  it('[MEM-32d] the review click opens the memory review link of the open dialog', async () => {
    const r = rig();
    await r.click({ target: 'genie_one_memory_review' });
    expect(r.opened).toEqual([]);
    await r.click({ target: 'forget_open' });
    // The view sends no URL; a URL in the payload is ignored.
    await r.click({ target: 'genie_one_memory_review', url: 'https://evil.test' });
    expect(r.opened).toEqual([REVIEW]);
  });

  it('[MEM-32c] [G-45b] the Delete in Genie One click opens that thread link only', async () => {
    const r = rig();
    await r.click({ target: 'forget_open' });
    await r.click({ target: 'forget_confirm', scope: 'mog' });
    await r.click({ target: 'genie_one_delete', conversationId: 'not-in-the-notice' });
    await r.click({ target: 'genie_one_delete', conversationId: CID });
    expect(r.opened).toEqual([THREAD]);
  });

  it('[MEM-32a] cancel closes the dialog; a failed Forget says nothing changed', async () => {
    const r = rig({
      confirm: async () => {
        throw new Error('host_failed');
      },
    });
    await r.click({ target: 'forget_open' });
    await r.click({ target: 'forget_confirm', scope: 'mog' });
    expect(r.sent[1]?.phase).toBe('confirm');
    expect(r.sent[1]?.text).toBe(
      'Bottle could not forget this. Nothing on this computer changed. Try again.',
    );
    await r.click({ target: 'forget_cancel' });
    expect(r.sent[2]).toBeNull();
    await r.click({ target: 'genie_one_memory_review' });
    expect(r.opened).toEqual([]);
  });

  it('[MEM-32a] open(scope) shows the dialog for a session; a bad scope shows nothing', async () => {
    const r = rig();
    expect(await r.c.open({ mogId: 'bad' })).toBe(false);
    expect(await r.c.open({ mogId: MOG, sessionId: '21111111-2222-4333-8444-555555555501' })).toBe(
      true,
    );
    expect(r.sent).toHaveLength(1);
  });

  it('[MEM-32a] wireForget routes view:input clicks from the bus', async () => {
    const bus = new BrainBus();
    const sent: unknown[] = [];
    const { off } = wireForget(bus, {
      flow: { dialog: () => confirmModel(MOG), confirm: async () => confirmModel(MOG) },
      send: async (m) => {
        sent.push(m);
      },
    });
    bus.dispatch({
      event: 'view:input',
      payload: {
        from: 'hub',
        input: { mogId: MOG, kind: 'click', payload: { target: 'forget_open' } },
      },
    });
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    off();
  });

  it('[ARC-2] the hub shape check keeps a valid model and refuses a bad one', () => {
    const m = confirmModel(MOG);
    expect(asForgetDialogModel(m)).toEqual(m);
    expect(asForgetDialogModel(null)).toBeNull();
    expect(asForgetDialogModel({ ...m, scope: 'all' })).toBeUndefined();
    expect(
      asForgetDialogModel({ ...m, result: { local: -1, deletedConversations: 0 } }),
    ).toBeUndefined();
  });

  it('[MEM-32a] [U9] no brain source enqueues a v1 sync outbox row yet (ADR 0081, TODO(#195))', () => {
    const files = (dir: string): string[] =>
      readdirSync(dir).flatMap((n) => {
        const p = join(dir, n);
        if (statSync(p).isDirectory()) return files(p);
        return /\.tsx?$/.test(n) && !/\.test\.tsx?$/.test(n) ? [p] : [];
      });
    const enqueuers = files(resolve(import.meta.dirname)).filter((f) =>
      readFileSync(f, 'utf8').includes("'mem.outbox.enqueue'"),
    );
    for (const f of enqueuers) {
      expect(readFileSync(f, 'utf8'), f).not.toMatch(/kind:\s*'sync'/);
    }
  });
});
