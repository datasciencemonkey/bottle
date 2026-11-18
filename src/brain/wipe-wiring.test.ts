/**
 * #124: the brain side of Settings → Wipe (MEM-33a, SEC-15c, G-45b, U9, ARC-5; ADR 0087). The flow
 * is a double; every value here is fake.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import type { WipeDialogModel, WipeScope, WipeSettingsModel } from '../shared/wipe-view';
import { asWipeDialogModel, asWipeSettingsModel, WIPE_VIEW_ID } from '../shared/wipe-view';
import { BrainBus, type ViewInput } from './bus';
import { WipeDialogController, wireWipe } from './wipe-wiring';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const MOG = '0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01';
const CID = '01f0aaaa-test-not-real-0001';
const THREAD = `https://acme.cloud.databricks.com/one/chat/threads/${CID}?o=1001`;

const confirmModel = (scope: WipeScope, alsoGenie: boolean): WipeDialogModel => ({
  scope,
  alsoGenie,
  phase: 'confirm',
  title: 'Wipe everything',
  text: 'Delete everything on this computer? You cannot undo this.',
});

const SETTINGS: WipeSettingsModel = {
  rows: [{ id: 'audio_journal', label: 'Session audio journal', value: '7 days', locked: true }],
};

function rig(over: { confirm?: () => Promise<WipeDialogModel> } = {}) {
  const sent: (WipeDialogModel | null)[] = [];
  const settings: WipeSettingsModel[] = [];
  const opened: string[] = [];
  const confirms: [WipeScope, boolean][] = [];
  const flow = {
    dialog: (s: WipeScope, alsoGenie: boolean) => {
      if (!['older_than_30d', 'older_than_90d', 'everything'].includes(s)) throw new Error('bad');
      return confirmModel(s, alsoGenie);
    },
    confirm:
      over.confirm ??
      (async (s: WipeScope, alsoGenie: boolean) => {
        confirms.push([s, alsoGenie]);
        return {
          ...confirmModel(s, alsoGenie),
          phase: 'done' as const,
          result: {
            local: 9,
            deletedConversations: 0,
            notices: [
              {
                mogId: MOG,
                label: 'Delete in Genie One',
                text: 'Bottle cannot delete Genie One conversations yet.',
                links: [{ conversationId: CID, url: THREAD }],
                noLink: 0,
              },
            ],
          },
        };
      }),
  };
  const c = new WipeDialogController({
    flow,
    settings: async () => SETTINGS,
    send: async (m) => {
      sent.push(m);
    },
    sendSettings: async (m) => {
      settings.push(m);
    },
    openLink: async (u) => {
      opened.push(u);
    },
  });
  const click = (payload: unknown, mogId = WIPE_VIEW_ID, from = 'hub'): Promise<void> =>
    c.onInput({ from, mogId, kind: 'click', payload } as ViewInput);
  return { c, sent, settings, opened, confirms, click };
}

describe('Wipe wiring (#124)', () => {
  it('[MEM-33a] the hub Wipe click opens the confirm dialog with the selected option', async () => {
    const r = rig();
    await r.click({ target: 'wipe_open', scope: 'older_than_30d', alsoGenie: true });
    expect(r.sent).toEqual([confirmModel('older_than_30d', true)]);
    expect(r.confirms).toEqual([]);
  });

  it('[MEM-33a] [U9] confirm runs the Wipe that the brain opened, once, then sends the done dialog', async () => {
    const r = rig();
    await r.click({ target: 'wipe_open', scope: 'everything', alsoGenie: false });
    await r.click({ target: 'wipe_confirm', scope: 'everything' });
    expect(r.confirms).toEqual([['everything', false]]);
    expect(r.sent[1]?.phase).toBe('done');
    expect(r.sent[1]?.result?.local).toBe(9);
    await r.click({ target: 'wipe_confirm', scope: 'everything' });
    expect(r.confirms).toHaveLength(1);
  });

  it('[MEM-33a] [U9] nothing is wiped without an open dialog, the same option, a hub click and the Wipe id', async () => {
    const r = rig();
    await r.click({ target: 'wipe_confirm', scope: 'everything' });
    await r.click(
      { target: 'wipe_open', scope: 'everything', alsoGenie: false },
      WIPE_VIEW_ID,
      `mog-${MOG}`,
    );
    await r.click({ target: 'wipe_open', scope: 'everything', alsoGenie: false }, MOG);
    await r.click({ target: 'wipe_open', scope: 'older_than_7d', alsoGenie: false });
    await r.click({ target: 'wipe_open', scope: 'everything', alsoGenie: 'yes' });
    expect(r.sent).toEqual([]);
    await r.click({ target: 'wipe_open', scope: 'older_than_90d', alsoGenie: false });
    // The view cannot widen the Wipe: a confirm for another option does nothing.
    await r.click({ target: 'wipe_confirm', scope: 'everything' });
    expect(r.confirms).toEqual([]);
    await r.click({ target: 'wipe_confirm', scope: 'older_than_90d' });
    expect(r.confirms).toEqual([['older_than_90d', false]]);
  });

  it('[MEM-33a] cancel closes the dialog; a failed Wipe says nothing changed', async () => {
    const r = rig({
      confirm: async () => {
        throw new Error('host_failed');
      },
    });
    await r.click({ target: 'wipe_open', scope: 'everything', alsoGenie: false });
    await r.click({ target: 'wipe_confirm', scope: 'everything' });
    expect(r.sent[1]?.phase).toBe('confirm');
    expect(r.sent[1]?.text).toBe(
      'Bottle could not wipe your data. Nothing on this computer changed. Try again.',
    );
    await r.click({ target: 'wipe_cancel' });
    expect(r.sent[2]).toBeNull();
  });

  it('[G-45b] the Delete in Genie One click opens that thread link of the done dialog only', async () => {
    const r = rig();
    await r.click({ target: 'genie_one_delete', conversationId: CID }, MOG);
    expect(r.opened).toEqual([]);
    await r.click({ target: 'wipe_open', scope: 'everything', alsoGenie: true });
    await r.click({ target: 'wipe_confirm', scope: 'everything' });
    await r.click({ target: 'genie_one_delete', conversationId: 'not-in-the-notice' }, MOG);
    await r.click(
      { target: 'genie_one_delete', conversationId: CID, url: 'https://evil.test' },
      MOG,
    );
    expect(r.opened).toEqual([THREAD]);
  });

  it('[SEC-15c] the Settings panel asks, and the brain sends the retention rows with the admin locks', async () => {
    const r = rig();
    await r.click({ target: 'wipe_settings' });
    expect(r.settings).toEqual([SETTINGS]);
    await r.click({ target: 'wipe_settings' }, WIPE_VIEW_ID, `mog-${MOG}`);
    expect(r.settings).toHaveLength(1);
  });

  it('[MEM-33a] wireWipe routes view:input clicks from the bus', async () => {
    const bus = new BrainBus();
    const sent: unknown[] = [];
    const { off } = wireWipe(bus, {
      flow: {
        dialog: () => confirmModel('everything', false),
        confirm: async () => confirmModel('everything', false),
      },
      settings: async () => SETTINGS,
      send: async (m) => {
        sent.push(m);
      },
      sendSettings: async () => undefined,
    });
    bus.dispatch({
      event: 'view:input',
      payload: {
        from: 'hub',
        input: {
          mogId: WIPE_VIEW_ID,
          kind: 'click',
          payload: { target: 'wipe_open', scope: 'everything', alsoGenie: false },
        },
      },
    });
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    off();
  });

  it('[ARC-2] the hub shape checks keep a valid model and refuse a bad one', () => {
    const m = confirmModel('everything', true);
    expect(asWipeDialogModel(m)).toEqual(m);
    expect(asWipeDialogModel(null)).toBeNull();
    expect(asWipeDialogModel({ ...m, scope: 'all' })).toBeUndefined();
    expect(asWipeDialogModel({ ...m, alsoGenie: 'no' })).toBeUndefined();
    expect(
      asWipeDialogModel({
        ...m,
        phase: 'done',
        result: { local: -1, deletedConversations: 0, notices: [] },
      }),
    ).toBeUndefined();
    expect(asWipeSettingsModel(SETTINGS)).toEqual(SETTINGS);
    expect(asWipeSettingsModel({ rows: [{ id: 'x', label: 'y', value: 'z' }] })).toBeUndefined();
  });

  it('[U9] [MEM-33a] only the confirm click reaches the Wipe: one caller, no timer', () => {
    const files = (dir: string): string[] =>
      readdirSync(dir).flatMap((n) => {
        const p = join(dir, n);
        if (statSync(p).isDirectory()) return files(p);
        return /\.tsx?$/.test(n) && !/\.test\.tsx?$/.test(n) ? [p] : [];
      });
    const src = resolve(import.meta.dirname, '..');
    // The R14 command and the core seam appear in the flow only, and the flow's confirm is
    // called from the controller's confirm click only.
    // Comments are removed first: only code counts.
    const code = (f: string) =>
      readFileSync(f, 'utf8')
        .replace(/\/\*[\s\S]*?\*\//g, '')
        .replace(/^\s*\/\/.*$/gm, '');
    const callers = files(src).filter((f) => /\bmem_wipe\b|\.wipe\(/.test(code(f)));
    expect(callers.map((f) => f.slice(src.length + 1)).sort()).toEqual([
      'brain/components/wipe-flow.ts',
    ]);
    const wiring = readFileSync(resolve(import.meta.dirname, 'wipe-wiring.ts'), 'utf8');
    expect(wiring.match(/flow\.confirm\(/g)).toHaveLength(1);
    expect(wiring).not.toMatch(/setInterval|setTimeout|requestIdleCallback/);
    // startBrain wires the dialog; it does not run a Wipe.
    const index = readFileSync(resolve(import.meta.dirname, 'components', 'index.ts'), 'utf8');
    expect(index).toContain('wireWipe(');
    expect(index).not.toMatch(/\.confirm\(|\.wipe\(/);
  });
});
