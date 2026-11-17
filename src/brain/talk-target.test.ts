import {
  MOG_TARGET_EVENT,
  TARGET_TOAST_MS,
  VIEW_READY,
  type MogTargetView,
} from '../shared/mog-target';
import { BrainBus } from './bus';
import { core } from './test-core';
import {
  configName,
  createTalkTarget,
  sendMogTarget,
  wireTalkTarget,
  type TalkTarget,
} from './talk-target';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const A = '00000000-0000-4000-8000-00000000000a';
const B = '00000000-0000-4000-8000-00000000000b';
const C = '00000000-0000-4000-8000-00000000000c';
const NAMES: Record<string, string> = { [A]: 'Sales', [B]: 'Ops', [C]: 'Finance' };

const flush = () => new Promise((r) => setTimeout(r, 0));

/** A fake clock for `after`: `advance(ms)` runs the callbacks that fall due. */
function fakeTimers() {
  let now = 0;
  let jobs: { at: number; fn: () => void; live: boolean }[] = [];
  return {
    after(ms: number, fn: () => void) {
      const job = { at: now + ms, fn, live: true };
      jobs.push(job);
      return () => {
        job.live = false;
      };
    },
    advance(ms: number) {
      now += ms;
      const due = jobs.filter((j) => j.live && j.at <= now);
      jobs = jobs.filter((j) => !due.includes(j));
      for (const j of due) j.fn();
    },
  };
}

async function setup() {
  const sent: MogTargetView[] = [];
  const active: string[] = [];
  const timers = fakeTimers();
  const target = createTalkTarget({
    send: (v) => void sent.push(v),
    setActive: async (id) => void active.push(id),
    after: timers.after,
    nameOf: async (id) => NAMES[id],
  });
  return { target, sent, active, timers };
}

/** The last view sent to each MOG. */
function lastViews(sent: MogTargetView[]): Record<string, MogTargetView> {
  const out: Record<string, MogTargetView> = {};
  for (const v of sent) out[v.mogId] = v;
  return out;
}

/** The MOGs whose last view shows the marker. */
function marked(sent: MogTargetView[]): string[] {
  return Object.values(lastViews(sent))
    .filter((v) => v.target)
    .map((v) => v.mogId);
}

async function openAll(t: TalkTarget, ...ids: string[]) {
  for (const id of ids) t.viewReady(id);
  await flush();
}

describe('T1 talk-key target in the brain (#73)', () => {
  beforeEach(() => core.reset());

  it('[MB-06a] the first open of a MOG makes it the target; a reload does not', async () => {
    const { target, active } = await setup();
    await openAll(target, A, B);
    expect(target.current()).toBe(B);
    target.interacted(A);
    await flush();
    // #305 review note A: a reload of the MOG B view is not an interaction.
    target.viewReady(B);
    expect(target.current()).toBe(A);
    expect(active).toEqual([A, B, A]);
  });

  it('[MB-06a] a click, poke, drag end or card input moves the target and calls mog_set_active', async () => {
    const { target, active } = await setup();
    await openAll(target, A, B, C);
    active.length = 0;
    target.interacted(A);
    await flush();
    expect(target.current()).toBe(A);
    target.interacted(C, 'card');
    expect(target.current()).toBe(C);
    expect(active).toEqual([A, C]);
  });

  it('[MB-06a] a closed target falls back to the last-interacted open MOG', async () => {
    const { target, sent } = await setup();
    await openAll(target, A, B, C);
    target.interacted(A);
    target.interacted(C);
    await flush();
    target.closed(C);
    expect(target.current()).toBe(A);
    expect(marked(sent)).toEqual([A]);
    target.closed(A);
    expect(target.current()).toBe(B);
    target.closed(B);
    expect(target.current()).toBeUndefined();
  });

  it('[MB-06a] the brain ignores a MOG it does not know as open (#305 review note B)', async () => {
    const { target, active, sent } = await setup();
    await openAll(target, A);
    active.length = 0;
    sent.length = 0;
    expect(target.interacted(B)).toBe(false);
    expect(target.pttDown(B)).toBe(false);
    target.closed(A);
    expect(target.pttDown(A)).toBe(false);
    expect(target.interacted(A)).toBe(false);
    expect(active).toEqual([]);
    expect(sent.some((v) => v.mogId === B)).toBe(false);
  });

  it('[MB-06a] a press for an open MOG moves the brain target to the MOG that Rust latched', async () => {
    const { target, active } = await setup();
    await openAll(target, A, B);
    active.length = 0;
    expect(target.pttDown(A)).toBe(true);
    expect(target.current()).toBe(A);
    // Rust has that target already, so the brain does not send it back.
    expect(active).toEqual([]);
  });

  it('[MB-06b] key-down shows a toast with the target MOG name for 1.2 s', async () => {
    const { target, sent, timers } = await setup();
    await openAll(target, A, B);
    sent.length = 0;
    target.pttDown(A);
    expect(TARGET_TOAST_MS).toBe(1200);
    expect(lastViews(sent)[A]).toEqual({ mogId: A, name: 'Sales', target: true, toast: true });
    timers.advance(TARGET_TOAST_MS - 1);
    expect(lastViews(sent)[A]?.toast).toBe(true);
    timers.advance(1);
    expect(lastViews(sent)[A]).toEqual({ mogId: A, name: 'Sales', target: true, toast: false });
    // The toast is only on the target MOG.
    expect(sent.some((v) => v.mogId !== A && v.toast)).toBe(false);
  });

  it('[MB-06b] a second key-down during the toast shows it for 1.2 s from the new press', async () => {
    const { target, sent, timers } = await setup();
    await openAll(target, A);
    target.pttDown(A);
    timers.advance(1000);
    target.pttDown(A);
    timers.advance(1000);
    expect(lastViews(sent)[A]?.toast).toBe(true);
    timers.advance(200);
    expect(lastViews(sent)[A]?.toast).toBe(false);
  });

  it('[UX-28] the marker moves with the target: one MOG has it, the old target loses it', async () => {
    const { target, sent } = await setup();
    await openAll(target, A, B, C);
    expect(marked(sent)).toEqual([C]);
    target.interacted(A);
    await flush();
    expect(marked(sent)).toEqual([A]);
    expect(lastViews(sent)[C]).toEqual({ mogId: C, name: 'Finance', target: false, toast: false });
    target.pttDown(B);
    expect(marked(sent)).toEqual([B]);
  });

  it('[UX-28] a reloaded MOG view gets its marker state again', async () => {
    const { target, sent } = await setup();
    await openAll(target, A, B);
    sent.length = 0;
    target.viewReady(A);
    target.viewReady(B);
    expect(sent).toEqual([
      { mogId: A, name: 'Sales', target: false, toast: false },
      { mogId: B, name: 'Ops', target: true, toast: false },
    ]);
  });

  it('[MB-06b] a MOG with no name lookup shows the name "MOG"', async () => {
    const sent: MogTargetView[] = [];
    const t = createTalkTarget({
      send: (v) => void sent.push(v),
      setActive: async () => undefined,
      after: () => () => undefined,
      nameOf: async () => {
        throw new Error('no store');
      },
    });
    t.viewReady(A);
    await flush();
    t.pttDown(A);
    expect(lastViews(sent)[A]).toEqual({ mogId: A, name: 'MOG', target: true, toast: true });
  });

  /** A tracker whose `mog_set_active` is refused, as before #439 routes it. */
  function refused() {
    const sent: MogTargetView[] = [];
    const t = createTalkTarget({
      send: (v) => void sent.push(v),
      setActive: async () => {
        throw { error: 'not_implemented' };
      },
      after: () => () => undefined,
    });
    return { t, sent };
  }

  it('[MB-06a] [UX-28] a refused mog_set_active (#439) does not move the marker on a MOG click', async () => {
    // Rust did not move its target, so the marker must not move: it never shows a wrong target.
    const { t, sent } = refused();
    t.viewReady(A);
    t.viewReady(B);
    await flush();
    expect(t.interacted(A)).toBe(true);
    await flush();
    expect(t.current()).toBe(B);
    expect(marked(sent)).toEqual([B]);
    expect(sent.some((v) => v.mogId === A && v.target)).toBe(false);
  });

  it('[MB-06a] [UX-28] a MOG click moves the marker only when mog_set_active answers ok', async () => {
    const sent: MogTargetView[] = [];
    let answer: () => void = () => undefined;
    const t = createTalkTarget({
      send: (v) => void sent.push(v),
      setActive: () => new Promise<void>((ok) => (answer = ok)),
      after: () => () => undefined,
    });
    t.viewReady(A);
    t.viewReady(B);
    await flush();
    t.interacted(A);
    await flush();
    expect(t.current()).toBe(B);
    expect(marked(sent)).toEqual([B]);
    answer();
    await flush();
    expect(t.current()).toBe(A);
    expect(marked(sent)).toEqual([A]);
  });

  it('[MB-06a] [UX-28] card input moves the marker at once: Rust sees the card focus', async () => {
    const { t, sent } = refused();
    t.viewReady(A);
    t.viewReady(B);
    expect(t.interacted(A, 'card')).toBe(true);
    expect(t.current()).toBe(A);
    expect(marked(sent)).toEqual([A]);
  });
});

describe('wiring of the talk-key target (#73)', () => {
  beforeEach(() => core.reset());

  function input(from: string, mogId: string, kind: string, payload?: unknown) {
    return { event: 'view:input', payload: { from, input: { mogId, kind, payload } } };
  }
  const ptt = (mogId: string, phase: string) => ({
    event: 'hotkey:ptt',
    payload: { mogId, phase },
  });

  async function wired() {
    const s = await setup();
    const bus = new BrainBus();
    wireTalkTarget(bus, s.target);
    return { ...s, bus };
  }

  it('[MB-06a] view.ready from a MOG view opens it; a click, poke or drag end targets it', async () => {
    const { bus, target } = await wired();
    bus.dispatch(input(`mog-${A}`, A, 'click', { action: VIEW_READY }));
    bus.dispatch(input(`mog-${B}`, B, 'click', { action: VIEW_READY }));
    await flush();
    expect(target.current()).toBe(B);
    for (const kind of ['click', 'poked', 'drag_end']) {
      bus.dispatch(input(`mog-${A}`, A, kind));
      await flush();
      expect(target.current()).toBe(A);
      bus.dispatch(input(`mog-${B}`, B, kind));
      await flush();
      expect(target.current()).toBe(B);
    }
  });

  it('[MB-06a] card input targets its MOG; hover and input for another MOG do not', async () => {
    const { bus, target } = await wired();
    bus.dispatch(input(`mog-${A}`, A, 'click', { action: VIEW_READY }));
    bus.dispatch(input(`mog-${B}`, B, 'click', { action: VIEW_READY }));
    await flush();
    bus.dispatch(input(`mog-${A}`, A, 'hover'));
    bus.dispatch(input(`card-${A}`, A, 'hover'));
    expect(target.current()).toBe(B);
    // A view speaks only for its own MOG (asViewInput drops this).
    bus.dispatch(input(`mog-${B}`, A, 'click'));
    expect(target.current()).toBe(B);
    bus.dispatch(input(`card-${A}`, A, 'chip_click', { chipId: 'x' }));
    expect(target.current()).toBe(A);
  });

  it('[MB-06a] input from a MOG view the brain missed (brain restart) opens and targets it', async () => {
    const { bus, target } = await wired();
    bus.dispatch(input(`mog-${C}`, C, 'poked'));
    await flush();
    expect(target.current()).toBe(C);
    // A card alone never proves that its MOG window is open (#305 review note B).
    bus.dispatch(input(`card-${A}`, A, 'click'));
    expect(target.current()).toBe(C);
  });

  it('[MB-06a] [MB-06b] hotkey:ptt down shows the toast; up and cancel do not', async () => {
    const { bus, sent } = await wired();
    bus.dispatch(input(`mog-${A}`, A, 'click', { action: VIEW_READY }));
    await flush();
    sent.length = 0;
    bus.dispatch(ptt(A, 'up'));
    bus.dispatch(ptt(A, 'cancel'));
    expect(sent).toEqual([]);
    bus.dispatch(ptt(A, 'down'));
    expect(sent).toEqual([{ mogId: A, name: 'Sales', target: true, toast: true }]);
  });

  it('[MB-06a] [MB-06b] hotkey:ptt down for a MOG the brain missed (brain restart) opens and targets it', async () => {
    // Rust latches only a MOG whose `mog-<id>` window is open, so the press proves it is open.
    const { bus, target, sent } = await wired();
    bus.dispatch(input(`mog-${A}`, A, 'click', { action: VIEW_READY }));
    await flush();
    bus.dispatch(ptt(C, 'down'));
    await flush();
    expect(target.current()).toBe(C);
    expect(target.isOpen(C)).toBe(true);
    expect(lastViews(sent)[C]).toMatchObject({ mogId: C, target: true, toast: true });
    expect(marked(sent)).toEqual([C]);
  });

  it('[UX-28] [ARC-5] sends the target view only to its own MOG window', async () => {
    await sendMogTarget({ mogId: A, name: 'Sales', target: true, toast: false });
    expect(core.emitTo).toHaveBeenCalledWith(`mog-${A}`, MOG_TARGET_EVENT, {
      mogId: A,
      name: 'Sales',
      target: true,
      toast: false,
    });
    expect(core.emit).not.toHaveBeenCalled();
    await expect(
      sendMogTarget({ mogId: 'a b', name: 'x', target: true, toast: false }),
    ).rejects.toThrow();
  });

  it('[MB-06b] the toast name comes from the MOG config (config_read)', async () => {
    core.invoke.mockImplementation(async (cmd: string) =>
      cmd === 'config_read' ? { id: A, json: { name: 'Sales' } } : {},
    );
    await expect(configName(A)).resolves.toBe('Sales');
    expect(core.invoke).toHaveBeenCalledWith('config_read', { args: { id: A } });
    core.invoke.mockImplementation(async () => ({ id: A, json: { name: 7 } }));
    await expect(configName(A)).resolves.toBeUndefined();
  });
});
