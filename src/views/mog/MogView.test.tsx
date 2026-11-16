// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { act } from 'react';
import { FrameProbe } from './frame-probe';
import type { MascotRenderer } from './mascot';
import type { MicLevelFrame, MicLevelSource } from './level-meter';
import { RiveMascot } from './rive-mascot';
import { render, tauri, viewEmits } from '../test-tauri';
import { MogView, defaultMascot } from './MogView';

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
    await act(async () => target.click());
    await act(async () => target.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })));
    expect(calls).toEqual(['mount', 'poke', 'hover:true']);
    expect(viewEmits()).toEqual([
      { mogId: ID, kind: 'click' },
      { mogId: ID, kind: 'hover' },
    ]);
    expect(tauri.invoke.mock.calls.every(([c]) => c === 'view_emit')).toBe(true);
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
