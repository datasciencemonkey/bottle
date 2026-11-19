// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { act } from 'react';
import { FrameProbe } from './frame-probe';
import type { MascotRenderer } from './mascot';
import type { MicLevelFrame, MicLevelSource } from './level-meter';
import { RiveMascot } from './rive-mascot';
import { render, tauri, viewEmits } from '../test-tauri';
import { DOUBLE_CLICK_MS, MogView, defaultMascot } from './MogView';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const ID = '00000000-0000-4000-8000-000000000001';
const OTHER = '00000000-0000-4000-8000-000000000002';

function fakeMascot() {
  const calls: string[] = [];
  const m: MascotRenderer = {
    mount: () => calls.push('mount'),
    setMood: (mood) => calls.push(`mood:${mood}`),
    setLevel: (l) => calls.push(`level:${l}`),
    setAttention: (a) => calls.push(`attention:${a}`),
    setHover: (h) => calls.push(`hover:${h}`),
    poke: () => calls.push('poke'),
    destroy: () => calls.push('destroy'),
  };
  return { m, calls };
}

const overlays = { attention: false, recording: false, offline: false, localOnly: false };

describe('V2 MOG view', () => {
  beforeEach(() => tauri.reset());

  it('[V2] mounts exactly one mascot instance and destroys it on unmount', async () => {
    const { m, calls } = fakeMascot();
    const factory = vi.fn(() => m);
    const { root } = await render(<MogView mogId={ID} createMascot={factory} />);
    expect(factory).toHaveBeenCalledTimes(1);
    expect(calls).toEqual(['mount']);
    act(() => root.unmount());
    expect(calls).toEqual(['mount', 'destroy']);
  });

  it('[V2] subscribes to mog:view_state and renders the mood the brain sends', async () => {
    const { m, calls } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    expect(tauri.listeners('mog:view_state')).toBe(1);
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'listening', overlays, level: 0.5 });
    expect(calls).toContain('mood:listening');
    // UX-06a: the view computes the level from the mic frames, not from the brain's value.
    expect(calls).not.toContain('level:0.5');
    expect(el.querySelector('[data-overlay="attention"]')).toBeNull();
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'error',
      overlays: { ...overlays, attention: true },
    });
    expect(el.querySelector('[data-overlay="attention"]')).not.toBeNull();
  });

  it('[V2] ignores view state for another MOG and malformed payloads', async () => {
    const { m, calls } = fakeMascot();
    await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:view_state', { mogId: OTHER, mood: 'error', overlays });
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'not-a-mood', overlays });
    await tauri.emit('mog:view_state', null);
    expect(calls).toEqual(['mount']);
  });

  it('[V2] [ARC-2] [UX-08a] sends pointer input only through view_emit', async () => {
    const { m, calls } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    const target = el.querySelector('[aria-label="MOG"]') as HTMLElement;
    vi.useFakeTimers();
    await act(async () => target.click());
    await act(async () => target.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })));
    // #573: the click goes after the double-click time.
    await act(async () => void vi.advanceTimersByTime(DOUBLE_CLICK_MS));
    vi.useRealTimers();
    expect(calls).toEqual(['mount', 'poke', 'hover:true']);
    expect(viewEmits()).toEqual([
      // MB-06a (#73): the view tells the brain that it is open. TODO(#439): a `ready` kind.
      { mogId: ID, kind: 'click', payload: { action: 'view.ready' } },
      { mogId: ID, kind: 'hover' },
      // #573: the pointer on the MOG, once per enter, for the Genie card hold.
      { mogId: ID, kind: 'hover', payload: { on: true } },
      { mogId: ID, kind: 'click' },
    ]);
    expect(tauri.invoke.mock.calls.every(([c]) => c === 'view_emit')).toBe(true);
  });

  it('[#573] a double-click opens Conversations and sends no single click', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    const target = el.querySelector('[aria-label="MOG"]') as HTMLElement;
    vi.useFakeTimers();
    const click = (detail: number) =>
      act(
        async () => void target.dispatchEvent(new MouseEvent('click', { bubbles: true, detail })),
      );
    await click(1);
    await click(2);
    await act(
      async () =>
        void target.dispatchEvent(new MouseEvent('dblclick', { bubbles: true, detail: 2 })),
    );
    await act(async () => void vi.advanceTimersByTime(DOUBLE_CLICK_MS * 2));
    vi.useRealTimers();
    const clicks = viewEmits().filter(
      (e) =>
        (e as { kind: string }).kind === 'click' &&
        !('payload' in (e as object) && (e as { payload: { action?: string } }).payload.action),
    );
    expect(clicks).toEqual([
      { mogId: ID, kind: 'click', payload: { target: 'open_conversations' } },
    ]);
  });

  it('[#573] the pointer leave tells the brain once', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    const root = el.firstElementChild as HTMLElement;
    const target = el.querySelector('[aria-label="MOG"]') as HTMLElement;
    await act(async () => target.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })));
    await act(async () => target.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })));
    await act(async () =>
      root.dispatchEvent(
        new MouseEvent('mouseout', { bubbles: true, relatedTarget: document.body }),
      ),
    );
    const holds = viewEmits().filter(
      (e) =>
        (e as { kind: string; payload?: unknown }).kind === 'hover' &&
        (e as { payload?: unknown }).payload,
    );
    expect(holds).toEqual([
      { mogId: ID, kind: 'hover', payload: { on: true } },
      { mogId: ID, kind: 'hover', payload: { on: false } },
    ]);
  });

  it('[V2] stops listening on unmount', async () => {
    const { m } = fakeMascot();
    const { root } = await render(<MogView mogId={ID} createMascot={() => m} />);
    act(() => root.unmount());
    expect(tauri.listeners('mog:view_state')).toBe(0);
  });

  it('[UX-09a] the default renderer is the Rive mascot', () => {
    expect(defaultMascot()).toBeInstanceOf(RiveMascot);
  });

  it('[UX-03] [AT-37] binds the attention overlay of mog.view_state to the mascot cue', async () => {
    const { m, calls } = fakeMascot();
    await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'idle',
      overlays: { ...overlays, attention: true },
    });
    expect(calls).toEqual(['mount', 'mood:idle', 'attention:true']);
  });

  it('[UX-08a] pointer leave turns the mascot hover off', async () => {
    const { m, calls } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    const target = el.querySelector('[aria-label="MOG"]') as HTMLElement;
    await act(async () => target.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })));
    await act(async () =>
      target.dispatchEvent(
        new MouseEvent('mouseout', { bubbles: true, relatedTarget: document.body }),
      ),
    );
    expect(calls).toEqual(['mount', 'hover:true', 'hover:false']);
  });

  it('[UX-06a] [UX-06b] [AT-14] the level comes from the mic frames this MOG owns', async () => {
    const { m, calls } = fakeMascot();
    let push: (f: MicLevelFrame) => void = () => undefined;
    let subs = 0;
    const levels: MicLevelSource = {
      subscribe: (fn) => {
        subs++;
        push = fn;
        return () => {
          subs--;
        };
      },
    };
    const { root } = await render(<MogView mogId={ID} createMascot={() => m} levels={levels} />);
    expect(subs).toBe(1);
    act(() => {
      for (let t = 0; t <= 300; t += 10) push({ owner: OTHER, dbfs: -10, t });
    });
    expect(calls.filter((c) => c.startsWith('level:'))).toEqual([]);
    act(() => {
      for (let t = 400; t <= 700; t += 10) push({ owner: ID, dbfs: -10, t });
    });
    const shown = calls.filter((c) => c.startsWith('level:')).map((c) => Number(c.slice(6)));
    expect(shown.length).toBeGreaterThan(3);
    expect(shown.at(-1)).toBeGreaterThan(0.9);
    act(() => push({ owner: null, dbfs: -10, t: 710 }));
    expect(calls.at(-1)).toBe('level:0');
    act(() => root.unmount());
    expect(subs).toBe(0);
  });

  it('[UX-06a] [UX-06b] [CAP-16a] with no levels prop, the meter reads the mog.level frames sent to this window', async () => {
    tauri.label = `mog-${ID}`;
    const { m, calls } = fakeMascot();
    const { root } = await render(<MogView mogId={ID} createMascot={() => m} />);
    expect(tauri.listeners('mog:level')).toBe(1);
    const levels = () => calls.filter((c) => c.startsWith('level:')).map((c) => Number(c.slice(6)));
    // A frame sent to another MOG window never reaches this one.
    await tauri.emitTo(`mog-${OTHER}`, 'mog:level', { owner: OTHER, dbfs: -10, t: 0 });
    expect(levels()).toEqual([]);
    for (let t = 0; t <= 300; t += 10) {
      await tauri.emitTo(`mog-${ID}`, 'mog:level', { owner: ID, dbfs: -10, t });
    }
    expect(levels().length).toBeGreaterThan(3);
    expect(levels().at(-1)).toBeGreaterThan(0.9);
    // UX-06b: the brain's `owner: null` frame on the stop sets 0 at once.
    await tauri.emitTo(`mog-${ID}`, 'mog:level', { owner: null, dbfs: -100, t: 310 });
    expect(calls.at(-1)).toBe('level:0');
    act(() => root.unmount());
    await act(async () => undefined);
    expect(tauri.listeners('mog:level')).toBe(0);
    tauri.label = 'hub';
  });

  it('[UX-06a] [UX-06b] a new renderer gets level writes from a new meter during an owned mic stream', async () => {
    const a = fakeMascot();
    const b = fakeMascot();
    let push: (f: MicLevelFrame) => void = () => undefined;
    const levels: MicLevelSource = {
      subscribe: (fn) => {
        push = fn;
        return () => undefined;
      },
    };
    const first = () => a.m;
    const second = () => b.m;
    const { root } = await render(<MogView mogId={ID} createMascot={first} levels={levels} />);
    act(() => {
      for (let t = 0; t <= 300; t += 10) push({ owner: ID, dbfs: -10, t });
    });
    expect(a.calls.some((c) => c.startsWith('level:'))).toBe(true);
    await act(async () =>
      root.render(<MogView mogId={ID} createMascot={second} levels={levels} />),
    );
    expect(b.calls.some((c) => c.startsWith('level:'))).toBe(false);
    // The stream goes on with the same owner. The new meter starts from 0, so it writes at once.
    act(() => push({ owner: ID, dbfs: -10, t: 310 }));
    act(() => push({ owner: ID, dbfs: -10, t: 400 }));
    expect(b.calls.some((c) => c.startsWith('level:'))).toBe(true);
    expect(a.calls.filter((c) => c.startsWith('level:')).length).toBeGreaterThan(0);
    expect(a.calls.at(-1)).toBe('destroy');
  });

  it('[UX-03] [UX-02b] a new renderer starts from the last state the brain sent', async () => {
    const a = fakeMascot();
    const b = fakeMascot();
    const first = () => a.m;
    const second = () => b.m;
    const { root } = await render(<MogView mogId={ID} createMascot={first} />);
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'error',
      overlays: { ...overlays, attention: true },
    });
    await act(async () => root.render(<MogView mogId={ID} createMascot={second} />));
    expect(a.calls.at(-1)).toBe('destroy');
    expect(b.calls).toEqual(['mount', 'mood:error', 'attention:true']);
  });

  /** What the brain's `mapBadges` sends (src/brain/components/mog-badges.ts). */
  const badges = [
    {
      kind: 'identity',
      text: 'prod-finance · Genie One',
      label:
        'prod-finance · Genie One. Profile prod-finance on adb-111.azuredatabricks.net, backend Genie One',
      detail: 'adb-111.azuredatabricks.net',
      hoverOnly: true,
    },
    {
      kind: 'stt',
      text: 'STT: local',
      label: 'STT: local. Speech to text runs on this device',
      hoverOnly: true,
      action: 'open_data_disclosures',
    },
    {
      kind: 'hop',
      text: 'Hop: sales-ws (this session)',
      label: 'Hop: sales-ws (this session). Asking in sales-ws for this session',
      hoverOnly: false,
    },
    {
      kind: 'offline',
      text: 'Offline',
      label: 'Offline. Databricks is not reachable',
      hoverOnly: false,
    },
    {
      kind: 'sync',
      text: 'Sync paused',
      label: 'Sync paused. All data stays on this device',
      hoverOnly: false,
      action: 'open_setup',
    },
  ];
  const shown = (el: HTMLElement) =>
    [...el.querySelectorAll('[data-badge]')].map((b) => b.getAttribute('data-badge'));
  const hoverOn = (t: HTMLElement) =>
    act(async () => t.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })));
  const hoverOff = (t: HTMLElement) =>
    act(async () =>
      t.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: document.body })),
    );

  it('[MOG-07] [UX-38] [AT-12] the profile/backend and STT badges show only while the pointer is on the MOG', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'idle',
      overlays,
      badges: badges.slice(0, 2),
    });
    expect(shown(el)).toEqual([]);
    const target = el.querySelector('[aria-label="MOG"]') as HTMLElement;
    await hoverOn(target);
    expect(shown(el)).toEqual(['identity', 'stt']);
    const identity = el.querySelector('[data-badge="identity"]') as HTMLElement;
    // MOG-07: the host shows as visible text next to "<profile> · <backend label>".
    expect(identity.querySelector('[data-badge-text]')?.textContent).toBe(
      'prod-finance · Genie One',
    );
    expect(identity.querySelector('[data-badge-detail]')?.textContent).toBe(
      'adb-111.azuredatabricks.net',
    );
    expect(identity.getAttribute('aria-label')).toContain('adb-111.azuredatabricks.net');
    expect(el.querySelector('[data-badge="stt"] [data-badge-text]')?.textContent).toBe(
      'STT: local',
    );
    await hoverOff(target);
    expect(shown(el)).toEqual([]);
  });

  it('[UX-39a] [UX-33] [AT-12] hop, offline and sync badges show with text and a label while they apply', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'idle', overlays, badges });
    expect(shown(el)).toEqual(['hop', 'offline', 'sync']);
    // With the offline badge, the 4.7 offline overlay text does not show twice.
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'idle',
      overlays: { ...overlays, offline: true },
      badges,
    });
    expect(el.querySelectorAll('[data-overlay="offline"]')).toHaveLength(0);
    expect(el.textContent?.match(/Offline/g)).toHaveLength(1);
    for (const b of badges.slice(2)) {
      const node = el.querySelector(`[data-badge="${b.kind}"]`) as HTMLElement;
      expect(node.querySelector('[data-badge-text]')?.textContent).toBe(b.text);
      expect(node.getAttribute('aria-label')).toBe(b.label);
    }
    // The state no longer applies: the brain sends no badge, and the view shows none.
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'idle', overlays, badges: [] });
    expect(shown(el)).toEqual([]);
  });

  it('[UX-39b] [AT-12] badges never change the mood the view renders', async () => {
    const { m, calls } = fakeMascot();
    await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'processing', overlays });
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'processing', overlays, badges });
    expect(calls.filter((c) => c.startsWith('mood:'))).toEqual([
      'mood:processing',
      'mood:processing',
    ]);
  });

  it('[UX-39a] drops a view state with malformed badges', async () => {
    const { m, calls } = fakeMascot();
    await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'idle', overlays, badges: 'x' });
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'idle',
      overlays,
      badges: [{ kind: 'hop', text: 1, label: 'l', hoverOnly: false }],
    });
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'idle',
      overlays,
      badges: [{ kind: 'hop', text: 't', label: 'l', hoverOnly: 'no' }],
    });
    expect(calls).toEqual(['mount']);
  });

  it('[UX-38] [UX-39a] a badge with an action is a button that tells the brain which badge', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'idle', overlays, badges });
    const target = el.querySelector('[aria-label="MOG"]') as HTMLElement;
    await hoverOn(target);
    const stt = el.querySelector('button[data-badge="stt"]') as HTMLElement;
    const sync = el.querySelector('button[data-badge="sync"]') as HTMLElement;
    expect(el.querySelector('button[data-badge="hop"]')).toBeNull();
    await act(async () => stt.click());
    await act(async () => sync.click());
    expect(viewEmits().slice(-2)).toEqual([
      { mogId: ID, kind: 'click', payload: { target: 'badge', badge: 'stt' } },
      { mogId: ID, kind: 'click', payload: { target: 'badge', badge: 'sync' } },
    ]);
  });

  it('[UX-39a] when the brain sends badges, the old offline overlay text does not show', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'idle',
      overlays: { ...overlays, offline: true },
      badges: [],
    });
    expect(el.querySelector('[data-overlay="offline"]')).toBeNull();
  });

  it('[MOG-07] moving the pointer from the mascot onto a badge keeps the hover badges', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'idle', overlays, badges });
    const target = el.querySelector('[aria-label="MOG"]') as HTMLElement;
    await hoverOn(target);
    const stt = el.querySelector('[data-badge="stt"]') as HTMLElement;
    await act(async () =>
      target.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: stt })),
    );
    await act(async () => stt.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })));
    expect(shown(el)).toContain('identity');
  });
});

describe('V2 MOG view frame budget (#13, ADR 0031)', () => {
  beforeEach(() => tauri.reset());

  it('[UX-36] [UX-05b] applies the mood in the event task and needs one frame per update', async () => {
    const { m, calls } = fakeMascot();
    const frames: (() => void)[] = [];
    let t = 100;
    const probe = new FrameProbe({ now: () => t, requestFrame: (cb) => void frames.push(cb) });
    await render(<MogView mogId={ID} createMascot={() => m} probe={probe} />);
    const trace = { seq: 1, eventAt: 90, sentAt: 95, keyDownAt: 60 };
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'listening', overlays, trace });
    // The mascot has the mood before any frame runs: the next frame draws it.
    expect(calls).toContain('mood:listening');
    expect(frames).toHaveLength(1);
    t = 112;
    frames[0]?.();
    expect(probe.samples()).toEqual([
      {
        seq: 1,
        mood: 'listening',
        eventAt: 90,
        sentAt: 95,
        keyDownAt: 60,
        receivedAt: 100,
        frameAt: 112,
      },
    ]);
  });

  it('[UX-36] a state for another MOG or with no trace asks for no frame', async () => {
    const { m } = fakeMascot();
    const frames: (() => void)[] = [];
    const probe = new FrameProbe({ now: () => 0, requestFrame: (cb) => void frames.push(cb) });
    await render(<MogView mogId={ID} createMascot={() => m} probe={probe} />);
    const trace = { seq: 1, eventAt: 0, sentAt: 0 };
    await tauri.emit('mog:view_state', { mogId: OTHER, mood: 'error', overlays, trace });
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'error', overlays });
    expect(frames).toHaveLength(0);
  });

  it('[UX-36] the MOG entry attaches the probe only with the trace flag', () => {
    const main = readFileSync(resolve(import.meta.dirname, 'main.tsx'), 'utf8');
    expect(main).toMatch(/import\.meta\.env\.VITE_BOTTLE_LATENCY_TRACE === '1'/);
    expect(main).toMatch(/probe=\{probe\}/);
  });
});

describe('V2 MOG view target marker and toast (#73)', () => {
  beforeEach(() => tauri.reset());

  const tv = (over: Record<string, unknown> = {}) => ({
    mogId: ID,
    name: 'Sales',
    target: true,
    toast: false,
    ...over,
  });

  it('[UX-28] draws a triangle marker under the target mascot and adds "target" to its name', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    expect(tauri.listeners('mog:target')).toBe(1);
    expect(el.querySelector('[data-marker="target"]')).toBeNull();
    await tauri.emit('mog:target', tv());
    const mascot = el.querySelector('.mascot') as HTMLElement;
    expect(mascot.getAttribute('aria-label')).toBe('Sales, target');
    const marker = el.querySelector('[data-marker="target"]') as HTMLElement;
    expect(marker).not.toBeNull();
    expect(marker.getAttribute('aria-hidden')).toBe('true');
    // The marker sits under the mascot: after it in the DOM, a CSS triangle at the foot.
    expect(mascot.compareDocumentPosition(marker) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    const css = readFileSync(resolve(import.meta.dirname, 'mog.css'), 'utf8');
    expect(css).toMatch(/\.target-marker\s*\{[^}]*border-top:[^}]*\}/s);
  });

  it('[UX-28] the marker moves away when this MOG stops being the target', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:target', tv());
    await tauri.emit('mog:target', tv({ target: false }));
    expect(el.querySelector('[data-marker="target"]')).toBeNull();
    expect(el.querySelector('.mascot')?.getAttribute('aria-label')).toBe('Sales');
  });

  it('[UX-28] ignores a target view for another MOG or with a bad shape', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:target', tv({ mogId: OTHER }));
    await tauri.emit('mog:target', tv({ target: 'yes' }));
    await tauri.emit('mog:target', tv({ name: 7 }));
    await tauri.emit('mog:target', null);
    expect(el.querySelector('[data-marker="target"]')).toBeNull();
    expect(el.querySelector('.mascot')?.getAttribute('aria-label')).toBe('MOG');
  });

  it('[MB-06b] shows the target toast with the MOG name while the brain says so', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:target', tv({ toast: true }));
    const toast = el.querySelector('[data-toast="target"]') as HTMLElement;
    expect(toast.textContent).toBe('Talking to Sales');
    expect(toast.getAttribute('role')).toBe('status');
    await tauri.emit('mog:target', tv({ toast: false }));
    expect(el.querySelector('[data-toast="target"]')).toBeNull();
  });

  it('[MB-06a] tells the brain once that the MOG view is open, after it listens', async () => {
    const { m } = fakeMascot();
    await render(<MogView mogId={ID} createMascot={() => m} />);
    expect(viewEmits()).toEqual([{ mogId: ID, kind: 'click', payload: { action: 'view.ready' } }]);
  });

  it('[ui-4] always shows a name pill, tinted with the palette colour the brain sends', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    // Before the brain answers, the pill still shows (the fallback name).
    expect(el.querySelector('[data-identity-pill]')?.textContent).toBe('MOG');
    await tauri.emit('mog:target', tv({ colorVariant: 'teal' }));
    expect(el.querySelector('[data-identity-pill]')?.textContent).toBe('Sales');
    const root = el.firstElementChild as HTMLElement;
    expect(root.style.getPropertyValue('--mog-id-color')).toBe('#0f6b66');
  });

  it('[ui-4] an unknown colour variant falls back to a stable hash of the MOG ID', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:target', tv({ colorVariant: 'not-a-colour' }));
    const root = el.firstElementChild as HTMLElement;
    const first = root.style.getPropertyValue('--mog-id-color');
    expect(first).toMatch(/^#[0-9a-f]{6}$/);
    await tauri.emit('mog:target', tv({ colorVariant: 'still-not-a-colour' }));
    expect(root.style.getPropertyValue('--mog-id-color')).toBe(first);
  });

  it('[ui-4] a hover tooltip shows the name and the home workspace, if known', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    const mascot = el.querySelector('.mascot') as HTMLElement;
    expect(mascot.title).toBe('MOG');
    await tauri.emit('mog:target', tv({ home: 'sales-ws' }));
    expect(mascot.title).toBe('Sales · sales-ws');
  });

  it('[ui-4] Hide shrinks the MOG to lamp-only; a click brings it back with no brain call', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    const mascot = el.querySelector('.mascot') as HTMLElement;
    await act(async () => {
      mascot.dispatchEvent(
        new MouseEvent('contextmenu', { bubbles: true, clientX: 10, clientY: 10 }),
      );
    });
    await act(async () => {
      (el.querySelector('[data-menu-item="hide"]') as HTMLElement).click();
    });
    expect(mascot.dataset['hidden']).toBe('true');
    const before = viewEmits().length;
    await act(async () => void mascot.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(mascot.dataset['hidden']).toBeUndefined();
    // Bringing it back is MOG-view-local: it sends nothing more to the brain.
    expect(viewEmits()).toHaveLength(before);
  });

  it('[ui-4] right-click opens a menu with Hide, Close, Open in Bottle and Change home workspace…', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    const mascot = el.querySelector('.mascot') as HTMLElement;
    await act(async () => {
      mascot.dispatchEvent(
        new MouseEvent('contextmenu', { bubbles: true, clientX: 5, clientY: 5 }),
      );
    });
    const menu = el.querySelector('[role="menu"]') as HTMLElement;
    expect([...menu.querySelectorAll('[role="menuitem"]')].map((b) => b.textContent)).toEqual([
      'Hide',
      'Close',
      'Open in Bottle',
      'Change home workspace…',
    ]);
    await act(async () => {
      (el.querySelector('[data-menu-item="close"]') as HTMLElement).click();
    });
    expect(viewEmits()).toEqual([
      { mogId: ID, kind: 'click', payload: { action: 'view.ready' } },
      { mogId: ID, kind: 'click', payload: { target: 'mog_menu_close' } },
    ]);
    expect(el.querySelector('[role="menu"]')).toBeNull();
  });

  it('[ui-4] Esc, and a click outside, close the menu with no action sent', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    const mascot = el.querySelector('.mascot') as HTMLElement;
    await act(async () => {
      mascot.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true }));
    });
    expect(el.querySelector('[role="menu"]')).not.toBeNull();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(el.querySelector('[role="menu"]')).toBeNull();
    await act(async () => {
      mascot.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true }));
    });
    expect(el.querySelector('[role="menu"]')).not.toBeNull();
    await act(async () => {
      document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    });
    expect(el.querySelector('[role="menu"]')).toBeNull();
  });

  it('[ui-4] change home workspace sends the menu change_home target', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    const mascot = el.querySelector('.mascot') as HTMLElement;
    await act(async () => mascot.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true })));
    await act(async () => {
      (el.querySelector('[data-menu-item="change_home"]') as HTMLElement).click();
    });
    expect(viewEmits().at(-1)).toEqual({
      mogId: ID,
      kind: 'click',
      payload: { target: 'mog_menu_change_home' },
    });
  });

  it('[ui-4] open in bottle from the menu sends the same target as a double-click', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    const mascot = el.querySelector('.mascot') as HTMLElement;
    await act(async () => mascot.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true })));
    await act(async () => {
      (el.querySelector('[data-menu-item="open_in_bottle"]') as HTMLElement).click();
    });
    expect(viewEmits().at(-1)).toEqual({
      mogId: ID,
      kind: 'click',
      payload: { target: 'open_conversations' },
    });
  });

  it('[MB-06a] tries view.ready again while no brain listens yet', async () => {
    vi.useFakeTimers();
    try {
      let refusals = 2;
      tauri.invoke.mockImplementation(async () => {
        if (refusals-- > 0) throw { error: 'host_failed' };
        return {};
      });
      const { m } = fakeMascot();
      await render(<MogView mogId={ID} createMascot={() => m} />);
      for (let i = 0; i < 5; i++) await act(async () => vi.advanceTimersByTimeAsync(1000));
      expect(viewEmits()).toHaveLength(3);
    } finally {
      tauri.invoke.mockImplementation(async () => ({}));
      vi.useRealTimers();
    }
  });
});
