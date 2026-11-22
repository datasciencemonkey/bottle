/**
 * The brain side of the hub Memory area (local memory slice 1, ADR 0112, ADR 0113). A fake store
 * records each `mem_local_exec` call with its ticket. The hub gets every event, and no other
 * window does.
 */
import { BrainBus } from './bus';
import { LocalMemoryService } from './components/memory-service';
import type { LocalMemoryStore, LocalStmtId } from './components/memory-store';
import { asMemoryAction, readMemory, wireHubMemory } from './hub-memory-wiring';
import type { JsonObject } from '../shared/ipc';
import { core } from './test-core';
import { readFileSync } from 'node:fs';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const MOG = '00000000-0000-4000-8000-00000000000a';
const MOG2 = '00000000-0000-4000-8000-00000000000b';
const ITEM = '11111111-2222-4333-8444-555555555555';
const NEW = '22222222-2222-4333-8444-555555555555';
const T = 1_767_225_600_000;
const SECRET_BODY = 'sentinel-body-text-9f3';

type Sent = { event: string; payload: Record<string, unknown> };

interface Call {
  stmtId: LocalStmtId;
  params: JsonObject;
  ticket: string | undefined;
}

const listRow = (over: Record<string, unknown> = {}) => ({
  itemId: ITEM,
  kind: 'fact',
  layer: 'daily',
  file: '/memories/sales/memory/2026-11-22.md',
  path: '/memories/sales/memory/2026-11-22.md#renewal',
  title: 'Renewal',
  summary: 'Renews in March',
  status: 'active',
  pinned: true,
  source: 'you',
  origin: 'added',
  tags: [],
  rating: 1,
  createdAt: T,
  updatedAt: T,
  ...over,
});

function setup(opts: { fail?: unknown; changes?: number; status?: string } = {}) {
  const calls: Call[] = [];
  // A MOG has a store row only after `mem.mog.upsert` (its first exchange, or ensureMog).
  const registered = new Set<string>();
  const store: LocalMemoryStore = {
    exec: async (stmtId, params, ticket) => {
      calls.push({ stmtId, params, ticket });
      if (
        opts.fail !== undefined &&
        stmtId !== 'mem.item.get' &&
        stmtId !== 'mem.item.list' &&
        stmtId !== 'mem.mog.upsert'
      ) {
        throw opts.fail;
      }
      switch (stmtId) {
        case 'mem.mog.upsert':
          registered.add(params['mogId'] as string);
          return { rows: [{ changes: 1 }] };
        case 'mem.item.list':
          if (!registered.has(params['mogId'] as string)) throw { error: 'invalid_args' };
          return {
            rows: params['mogId'] === MOG ? [listRow()] : [],
            folder: params['mogId'] === MOG ? 'sales' : 'ops-team',
          };
        case 'mem.item.get':
          return {
            rows: [
              {
                itemId: ITEM,
                type: 'fact',
                title: 'Renewal',
                bodyMd: 'Renews in March',
                status: opts.status ?? 'active',
                pinned: 1,
              },
            ],
          };
        case 'mem.file.get':
          return { rows: [{ path: params['path'], text: '# file' }] };
        case 'mem.item.write':
          return {
            rows: [
              { changes: registered.has(params['mogId'] as string) ? (opts.changes ?? 1) : 0 },
              { changes: 1 },
            ],
          };
        case 'mem.item.edit':
        case 'mem.item.rate':
          return { rows: [{ changes: opts.changes ?? 1 }, { changes: 1 }] };
        default:
          return { rows: [{ changes: 1 }, { changes: 0 }, { changes: 1 }] };
      }
    },
  };
  const memory = new LocalMemoryService(store, {
    now: () => T,
    newId: () => NEW,
    deviceId: () => 'device-1',
  });
  const sent: Sent[] = [];
  const bus = new BrainBus();
  const wiring = wireHubMemory(bus, {
    memory,
    mogs: async () => [
      { mogId: MOG, name: 'Sales', colorVariant: 'coral', backendKind: 'k' },
      { mogId: MOG2, name: 'Ops Team', backendKind: 'k' },
    ],
    send: async (event, payload) =>
      void sent.push({ event, payload: payload as Record<string, unknown> }),
  });
  const click = (payload: unknown, ticket?: string, from = 'hub', mogId = 'memory') => {
    bus.dispatch({
      event: 'view:input',
      payload: { from, input: { mogId, kind: 'click', payload }, actionTicket: ticket },
    });
    return new Promise((r) => setTimeout(r, 0));
  };
  return {
    calls,
    sent,
    click,
    wiring,
    forgetMogRow: (id: string) => void registered.delete(id),
    writes: () =>
      calls.filter(
        (c) =>
          c.stmtId !== 'mem.item.get' &&
          c.stmtId !== 'mem.item.list' &&
          c.stmtId !== 'mem.mog.upsert',
      ),
  };
}

const result = (sent: Sent[]) =>
  sent.filter((s) => s.event === 'hub:memory.result').map((s) => s.payload);

describe('hub Memory wiring: reads', () => {
  it('memory.ready lists each MOG and sends hub:memory with folders', async () => {
    const s = setup();
    await s.click({ action: 'memory.ready' });
    expect(s.calls.map((c) => [c.stmtId, c.params['mogId']])).toEqual([
      ['mem.mog.upsert', MOG],
      ['mem.item.list', MOG],
      ['mem.mog.upsert', MOG2],
      ['mem.item.list', MOG2],
    ]);
    const m = s.sent.find((e) => e.event === 'hub:memory')?.payload as {
      folders: { mogId: string; folder: string; rows: unknown[]; mogColorVariant?: string }[];
    };
    expect(
      m.folders.map((f: { mogId: string; folder: string; rows: unknown[] }) => [
        f.mogId,
        f.folder,
        f.rows.length,
      ]),
    ).toEqual([
      [MOG, 'sales', 1],
      [MOG2, 'ops-team', 0],
    ]);
    expect(m.folders[0]?.mogColorVariant).toBe('coral');
  });

  it('a MOG with no exchange yet shows an empty folder, and a hub add works', async () => {
    const s = setup();
    await s.click({ action: 'memory.ready' });
    await s.click({ action: 'memory.ready' });
    // No cache: each list registers each MOG again, so a Forget of the MOG row heals itself.
    expect(s.calls.filter((c) => c.stmtId === 'mem.mog.upsert')).toHaveLength(4);
    const ops = (
      s.sent[0]?.payload as { folders: { mogId: string; folder: string; rows: unknown[] }[] }
    ).folders[1];
    expect(ops).toMatchObject({ mogId: MOG2, folder: 'ops-team', rows: [] });
    await s.click(
      { action: 'memory.add', requestId: 'r9', mogId: MOG2, kind: 'fact', title: 'T', body: 'B' },
      'tkt',
    );
    expect(result(s.sent)).toEqual([{ requestId: 'r9', ok: true, itemId: NEW, status: 'active' }]);
  });

  it('after the MOG row is forgotten, the list and a hub add work again', async () => {
    const s = setup();
    await s.click({ action: 'memory.ready' });
    s.forgetMogRow(MOG);
    await s.click({ action: 'memory.ready' });
    const last = s.sent.filter((e) => e.event === 'hub:memory').at(-1)?.payload as {
      folders: { mogId: string }[];
    };
    expect(last.folders.map((f) => f.mogId)).toContain(MOG);
    await s.click(
      { action: 'memory.add', requestId: 'r8', mogId: MOG, kind: 'fact', title: 'T', body: 'B' },
      'tkt',
    );
    expect(result(s.sent)).toEqual([{ requestId: 'r8', ok: true, itemId: NEW, status: 'active' }]);
  });

  it('one MOG that fails to list does not hide the others', async () => {
    const memory = {
      ensureMog: async () => undefined,
      listItems: async (id: string) => {
        if (id === MOG2) throw { error: 'invalid_args' };
        return { rows: [listRow()], folder: 'sales' };
      },
    };
    const m = await readMemory(memory, async () => [
      { mogId: MOG, name: 'Sales', backendKind: 'k' },
      { mogId: MOG2, name: 'Ops', backendKind: 'k' },
    ]);
    expect(m.unavailable).toBeUndefined();
    expect(m.folders.map((f) => f.mogId)).toEqual([MOG]);
  });

  it('every MOG failing, or no folder name from Rust, is unavailable', async () => {
    const bad = {
      ensureMog: async () => undefined,
      listItems: async () => ({ rows: [] }),
    };
    expect(
      (await readMemory(bad, async () => [{ mogId: MOG, name: 'Sales', backendKind: 'k' }]))
        .unavailable,
    ).toBe(true);
    expect((await readMemory(bad, async () => [])).unavailable).toBeUndefined();
  });

  it('a failed list sends an unavailable list with no error text', async () => {
    const memory = {
      ensureMog: async () => undefined,
      listItems: async () => Promise.reject({ error: 'host_failed' }),
    };
    expect(
      await readMemory(memory, async () => [{ mogId: MOG, name: 'Sales', backendKind: 'k' }]),
    ).toEqual({
      userItems: 0,
      folders: [],
      unavailable: true,
    });
  });

  it('memory.file runs mem.file.get and sends hub:memory.file', async () => {
    const s = setup();
    const path = '/memories/sales/MEMORY.md';
    await s.click({ action: 'memory.file', mogId: MOG, path });
    expect(s.calls).toEqual([
      { stmtId: 'mem.file.get', params: { mogId: MOG, path }, ticket: undefined },
    ]);
    expect(s.sent).toEqual([
      { event: 'hub:memory.file', payload: { mogId: MOG, path, text: '# file' } },
    ]);
  });

  it('memory.item runs mem.item.get and sends the editor fields', async () => {
    const s = setup();
    await s.click({ action: 'memory.item', mogId: MOG, itemId: ITEM });
    expect(s.calls.map((c) => [c.stmtId, c.params])).toEqual([
      ['mem.item.get', { mogId: MOG, itemId: ITEM }],
    ]);
    expect(s.sent).toEqual([
      {
        event: 'hub:memory.item',
        payload: {
          mogId: MOG,
          itemId: ITEM,
          kind: 'fact',
          title: 'Renewal',
          body: 'Renews in March',
          status: 'active',
          pinned: true,
        },
      },
    ]);
  });

  it('only the hub Memory view is heard', async () => {
    const s = setup();
    await s.click({ action: 'memory.ready' }, undefined, `card-${MOG}`, MOG);
    await s.click({ action: 'memory.ready' }, undefined, 'hub', 'conversations');
    await s.click({ action: 'nope' });
    expect(s.calls).toEqual([]);
    expect(s.sent).toEqual([]);
  });

  it('the default sender goes to the hub window only', async () => {
    core.reset();
    const bus = new BrainBus();
    const memory = new LocalMemoryService({ exec: async () => ({ rows: [] }) });
    wireHubMemory(bus, { memory, mogs: async () => [] });
    bus.dispatch({
      event: 'view:input',
      payload: {
        from: 'hub',
        input: { mogId: 'memory', kind: 'click', payload: { action: 'memory.ready' } },
      },
    });
    await new Promise((r) => setTimeout(r, 0));
    expect(core.emitTo.mock.calls.map((c) => [c[0], c[1]])).toEqual([['hub', 'hub:memory']]);
  });
});

describe('hub Memory wiring: writes', () => {
  it('memory.add writes a pinned, added item with no text and passes the ticket', async () => {
    const s = setup();
    await s.click(
      {
        action: 'memory.add',
        requestId: 'r1',
        mogId: MOG,
        kind: 'decision',
        title: 'T',
        body: 'B',
      },
      'tkt-add',
    );
    const w = s.writes();
    expect(w).toHaveLength(1);
    expect(w[0]).toEqual({
      stmtId: 'mem.item.write',
      ticket: 'tkt-add',
      params: {
        mogId: MOG,
        itemId: NEW,
        itemType: 'decision',
        source: 'you',
        confidence: 1,
        pinned: true,
        origin: 'added',
        title: 'T',
        body: 'B',
        seenAt: T,
        rowHlc: T + 1,
        deviceId: 'device-1',
        now: T,
      },
    });
    expect(w[0]?.params).not.toHaveProperty('text');
    expect(result(s.sent)).toEqual([{ requestId: 'r1', ok: true, itemId: NEW, status: 'active' }]);
    // The refresh after the write.
    expect(s.sent.at(-1)?.event).toBe('hub:memory');
  });

  it('memory.edit runs mem.item.edit with the ticket and refreshes', async () => {
    const s = setup({ status: 'quarantined' });
    await s.click(
      {
        action: 'memory.edit',
        requestId: 'r2',
        mogId: MOG,
        itemId: ITEM,
        kind: 'fact',
        title: 'T2',
        body: 'B2',
      },
      'tkt-edit',
    );
    expect(s.writes()).toEqual([
      {
        stmtId: 'mem.item.edit',
        ticket: 'tkt-edit',
        params: {
          mogId: MOG,
          itemId: ITEM,
          itemType: 'fact',
          title: 'T2',
          body: 'B2',
          now: T,
          rowHlc: T,
        },
      },
    ]);
    expect(result(s.sent)).toEqual([
      { requestId: 'r2', ok: true, itemId: ITEM, status: 'quarantined' },
    ]);
    expect(s.sent.at(-1)?.event).toBe('hub:memory');
  });

  it('memory.rate runs mem.item.rate with the ticket and refreshes', async () => {
    const s = setup();
    await s.click({ action: 'memory.rate', requestId: 'r3', mogId: MOG, itemId: ITEM }, 'tkt-rate');
    expect(s.writes()).toEqual([
      {
        stmtId: 'mem.item.rate',
        ticket: 'tkt-rate',
        params: { mogId: MOG, itemId: ITEM, now: T, rowHlc: T },
      },
    ]);
    expect(result(s.sent)).toEqual([{ requestId: 'r3', ok: true, itemId: ITEM, status: 'active' }]);
    expect(s.sent.at(-1)?.event).toBe('hub:memory');
  });

  it('memory.delete runs mem.forget.items and refreshes', async () => {
    const s = setup();
    await s.click({ action: 'memory.delete', mogId: MOG, itemId: ITEM });
    const w = s.writes();
    expect(w).toHaveLength(1);
    expect(w[0]).toMatchObject({
      stmtId: 'mem.forget.items',
      ticket: undefined,
      params: { mogId: MOG, itemIds: [ITEM] },
    });
    expect(s.sent.map((e) => e.event)).toEqual(['hub:memory']);
  });

  it.each([
    [
      'add',
      { action: 'memory.add', requestId: 'r', mogId: MOG, kind: 'fact', title: 'T', body: 'B' },
    ],
    [
      'edit',
      {
        action: 'memory.edit',
        requestId: 'r',
        mogId: MOG,
        itemId: ITEM,
        kind: 'fact',
        title: 'T',
        body: 'B',
      },
    ],
    ['rate', { action: 'memory.rate', requestId: 'r', mogId: MOG, itemId: ITEM }],
  ])('%s with no ticket makes no call and reports failed', async (_n, payload) => {
    const s = setup();
    await s.click(payload);
    expect(s.calls).toEqual([]);
    expect(result(s.sent)).toEqual([{ requestId: 'r', ok: false, code: 'failed' }]);
    expect(s.sent).toHaveLength(1);
  });

  it.each([
    ['secret_refused', { error: 'secret_refused' }, 'secret_refused'],
    ['invalid_args', { error: 'invalid_args' }, 'invalid'],
    ['host_failed', { error: 'host_failed' }, 'failed'],
    ['a thrown string', 'boom', 'failed'],
  ])('maps %s to %s and does not refresh', async (_n, fail, code) => {
    const s = setup({ fail });
    await s.click({ action: 'memory.rate', requestId: 'r', mogId: MOG, itemId: ITEM }, 't');
    expect(result(s.sent)).toEqual([{ requestId: 'r', ok: false, code }]);
    expect(s.sent).toHaveLength(1);
  });

  it('zero changes is invalid', async () => {
    const s = setup({ changes: 0 });
    await s.click(
      { action: 'memory.add', requestId: 'r', mogId: MOG, kind: 'fact', title: '', body: 'B' },
      't',
    );
    expect(result(s.sent)).toEqual([{ requestId: 'r', ok: false, code: 'invalid' }]);
  });

  it('a bad payload is dropped, and an over-long body is invalid with no call', async () => {
    const s = setup();
    await s.click(
      {
        action: 'memory.add',
        requestId: 'r',
        mogId: 'not-a-uuid',
        kind: 'fact',
        title: '',
        body: 'B',
      },
      't',
    );
    await s.click(
      {
        action: 'memory.add',
        requestId: 'r',
        mogId: MOG,
        kind: 'preference',
        title: '',
        body: 'B',
      },
      't',
    );
    expect(s.calls).toEqual([]);
    expect(s.sent).toEqual([]);
    await s.click(
      {
        action: 'memory.add',
        requestId: 'r',
        mogId: MOG,
        kind: 'fact',
        title: '',
        body: 'x'.repeat(4001),
      },
      't',
    );
    expect(s.calls).toEqual([]);
    expect(result(s.sent)).toEqual([{ requestId: 'r', ok: false, code: 'invalid' }]);
  });
});

describe('hub Memory wiring: edit rules', () => {
  const edit = (over: Record<string, unknown>) => ({
    action: 'memory.edit',
    requestId: 'r',
    mogId: MOG,
    itemId: ITEM,
    kind: 'fact',
    title: 'T',
    body: 'B',
    ...over,
  });

  it('refuses preference, and a change of a fact to a Genie answer, with no write', async () => {
    const s = setup();
    await s.click(edit({ kind: 'preference' }), 't');
    await s.click(edit({ kind: 'genie_answer' }), 't');
    expect(s.writes()).toEqual([]);
    expect(result(s.sent)).toEqual([
      { requestId: 'r', ok: false, code: 'invalid' },
      { requestId: 'r', ok: false, code: 'invalid' },
    ]);
  });

  it('caps count code points, not UTF-16 units', async () => {
    const s = setup();
    await s.click(edit({ title: '😀'.repeat(80) }), 't');
    expect(s.writes()).toHaveLength(1);
    await s.click(edit({ title: '😀'.repeat(81) }), 't');
    expect(s.writes()).toHaveLength(1);
  });

  it('memory.item with an unknown status sends nothing', async () => {
    const s = setup({ status: 'bogus' });
    await s.click({ action: 'memory.item', mogId: MOG, itemId: ITEM });
    expect(s.sent).toEqual([]);
  });
});

describe('asMemoryAction', () => {
  it('rejects unknown actions and bad ids', () => {
    expect(asMemoryAction({ action: 'memory.pin' })).toBeUndefined();
    expect(asMemoryAction({ action: 'memory.delete', mogId: MOG, itemId: 'x' })).toBeUndefined();
    expect(asMemoryAction({ action: 'memory.ready' })).toEqual({ action: 'memory.ready' });
  });
});

describe('no memory content in a log', () => {
  it('writes, failures and reads print nothing', async () => {
    const spies = (['log', 'info', 'warn', 'error', 'debug'] as const).map((k) =>
      vi.spyOn(console, k).mockImplementation(() => undefined),
    );
    const s = setup({ fail: new Error(SECRET_BODY) });
    await s.click(
      {
        action: 'memory.add',
        requestId: 'r',
        mogId: MOG,
        kind: 'fact',
        title: SECRET_BODY,
        body: SECRET_BODY,
      },
      't',
    );
    await s.click({ action: 'memory.ready' });
    const printed = spies.flatMap((sp) => sp.mock.calls.flat().map(String));
    spies.forEach((sp) => sp.mockRestore());
    expect(printed.join('\n')).not.toContain(SECRET_BODY);
    expect(JSON.stringify(result(s.sent))).not.toContain(SECRET_BODY);
  });

  it('the source files of the memory flow have no console or telemetry call', () => {
    for (const f of ['hub-memory-wiring.ts', 'remember-flow.ts', 'components/memory-service.ts']) {
      const src = readFileSync(new URL(`./${f}`, import.meta.url), 'utf8');
      expect(src).not.toMatch(/console\.|createBrainLog|\.event\(/);
    }
  });
});
