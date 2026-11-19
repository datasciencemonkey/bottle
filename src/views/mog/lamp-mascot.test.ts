// @vitest-environment jsdom
/**
 * The lamp mascot (#286): state mapping, the Genie's height across pose swaps, the rest-state
 * life signs, the level ring, reduced motion, and no loop at idle. The motion itself is CSS
 * (`mog.css`); the CSS tests are at the end.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import type { Mood } from '../../shared/interfaces';
import {
  ACTIVE_STATES,
  EYES,
  LEVEL_STEP,
  LampMascot,
  REST_SINK_PCT,
  TIMING,
  lampState,
  poseOffset,
  type LampClock,
  type LampState,
} from './lamp-mascot';

/** A fake clock: timers run only when the test advances it. */
function fakeClock() {
  let now = 0;
  let next = 1;
  const timers = new Map<number, { at: number; fn: () => void }>();
  const clock: LampClock = {
    setTimeout: (fn, ms) => {
      const id = next++;
      timers.set(id, { at: now + ms, fn });
      return id;
    },
    clearTimeout: (id) => void timers.delete(id),
  };
  const advance = (ms: number) => {
    const end = now + ms;
    for (;;) {
      const due = [...timers.entries()]
        .filter(([, t]) => t.at <= end)
        .sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) break;
      timers.delete(due[0]);
      now = due[1].at;
      due[1].fn();
    }
    now = end;
  };
  return { clock, advance, pending: () => timers.size };
}

function mountLamp(reduced = false) {
  const el = document.createElement('div');
  document.body.append(el);
  const c = fakeClock();
  const m = new LampMascot({ clock: c.clock, random: () => 0, reducedMotion: () => reduced });
  m.mount(el);
  const root = () => el.querySelector<HTMLElement>('[data-mascot="lamp"]');
  const genie = () => el.querySelector<HTMLElement>('.genie');
  /** The Genie's translate Y in %, or undefined when none is set. */
  const y = () => {
    const t = genie()?.style.translate;
    return t ? parseFloat(t.split(/\s+/)[1] ?? '0') : undefined;
  };
  return { el, m, c, root, genie, y };
}

describe('lampState (#286)', () => {
  const base = { hover: false, attention: false, rested: false };
  it.each<[Mood, LampState]>([
    ['listening', 'listening'],
    ['processing', 'working'],
    ['answering', 'ready'],
    ['success', 'ready'],
    ['error', 'error'],
    ['sleeping', 'sleeping'],
  ])('maps the %s mood to %s', (mood, state) => {
    expect(lampState({ ...base, mood })).toBe(state);
    // The pointer and the rest flag do not override an active mood.
    expect(lampState({ mood, hover: true, attention: true, rested: true })).toBe(state);
  });

  it('idle is in the lamp only when rested, with no pointer and no cue', () => {
    expect(lampState({ ...base, mood: 'idle' })).toBe('peek');
    expect(lampState({ ...base, mood: 'idle', rested: true })).toBe('rest');
    expect(lampState({ ...base, mood: 'idle', rested: true, hover: true })).toBe('peek');
    expect(lampState({ ...base, mood: 'idle', rested: true, attention: true })).toBe('peek');
  });

  it('the active states are the ones that loop', () => {
    expect([...ACTIVE_STATES].sort()).toEqual(['listening', 'ready', 'working']);
  });
});

describe('LampMascot (#286)', () => {
  afterEach(() => document.body.replaceChildren());

  it('draws the three planes and one pose per mood, with only the current pose on', () => {
    const { el, root } = mountLamp();
    expect(el.querySelector('img.lamp-back')?.getAttribute('src')).toMatch(/layers\/lamp-back/);
    expect(el.querySelector('img.lamp-front')?.getAttribute('src')).toMatch(/layers\/lamp-front/);
    const poses = el.querySelectorAll<HTMLImageElement>('img.genie-pose');
    expect(poses).toHaveLength(7);
    const on = el.querySelectorAll('img.genie-pose[data-on]');
    expect(on).toHaveLength(1);
    expect(on[0]?.getAttribute('src')).toMatch(/genie-idle/);
    expect(root()?.dataset['state']).toBe('peek');
    // The Genie is inside the fixed rim clip, so it slides behind the rim.
    expect(el.querySelector('.genie-clip > .genie')).not.toBeNull();
  });

  it('goes back into the lamp after a calm moment, and comes out on hover or a poke', () => {
    const { m, c, root, y } = mountLamp();
    c.advance(TIMING.restAfter - 1);
    expect(root()?.dataset['state']).toBe('peek');
    c.advance(1);
    expect(root()?.dataset['state']).toBe('rest');
    expect(y()).toBe(REST_SINK_PCT);
    m.setHover(true);
    expect(root()?.dataset['state']).toBe('peek');
    expect(y()).toBe(0);
    // It stays out while the pointer is on it.
    c.advance(TIMING.restAfter * 3);
    expect(root()?.dataset['state']).toBe('peek');
    m.setHover(false);
    c.advance(TIMING.restAfter);
    expect(root()?.dataset['state']).toBe('rest');
    m.poke();
    expect(root()?.dataset['state']).toBe('peek');
  });

  it('comes out of the lamp when the user speaks', () => {
    const { m, c, root } = mountLamp();
    c.advance(TIMING.restAfter);
    expect(root()?.dataset['state']).toBe('rest');
    m.setMood('listening');
    expect(root()?.dataset['state']).toBe('listening');
    // Back to idle: it looks out first, then it rests again.
    m.setMood('idle');
    expect(root()?.dataset['state']).toBe('peek');
    c.advance(TIMING.restAfter);
    expect(root()?.dataset['state']).toBe('rest');
  });

  it('keeps the Genie at its old height across a pose swap, then moves it (FLIP)', () => {
    const { m, genie } = mountLamp();
    const moves: string[] = [];
    const g = genie() as HTMLElement;
    const style = g.style;
    const set = Object.getOwnPropertyDescriptor(CSSStyleDeclaration.prototype, 'translate');
    // Record every translate write in order (jsdom computes no layout).
    Object.defineProperty(style, 'translate', {
      configurable: true,
      get: () => moves.at(-1) ?? '',
      set: (v: string) => void moves.push(v),
    });
    m.setMood('answering');
    const from = poseOffset('idle', 'answering');
    expect(from).toBeGreaterThan(0); // the standing pose starts low, in the lamp
    expect(moves.map((v) => parseFloat(v.split(' ')[1] ?? ''))).toEqual([
      Number(from.toFixed(3)),
      0,
    ]);
    expect(g.dataset['dir']).toBe('up');
    expect(g.style.getPropertyValue('--move-ms')).toBe('560ms');
    if (set) Object.defineProperty(style, 'translate', set);
  });

  it('blinks and peeks up now and then at rest, with timers only (no animation frames)', () => {
    const raf = vi.spyOn(window, 'requestAnimationFrame');
    const { c, genie, root } = mountLamp();
    // The random source is 0, so each wait is its minimum.
    c.advance(TIMING.blinkMin);
    expect(genie()?.dataset['blink']).toBe('');
    c.advance(TIMING.blinkLength);
    expect(genie()?.dataset['blink']).toBeUndefined();
    c.advance(TIMING.restAfter - TIMING.blinkMin - TIMING.blinkLength);
    expect(root()?.dataset['state']).toBe('rest');
    c.advance(TIMING.peekUpMin);
    expect(genie()?.dataset['peekUp']).toBe('');
    c.advance(TIMING.peekUpLength);
    expect(genie()?.dataset['peekUp']).toBeUndefined();
    expect(raf).not.toHaveBeenCalled();
    raf.mockRestore();
  });

  it('blinks in every pose with open eyes, faster while listening or working', () => {
    const { m, c, el, genie } = mountLamp();
    for (const mood of ['listening', 'processing', 'answering', 'success', 'error'] as Mood[]) {
      m.setMood(mood);
      // One lid pair per pose, in the shared lid layer.
      expect(el.querySelectorAll(`[data-lids="${mood}"] rect`)).toHaveLength(2);
      const fast = mood === 'listening' || mood === 'processing';
      const wait = fast ? TIMING.blinkFastMin : TIMING.blinkMin;
      c.advance(wait - 1);
      expect(genie()?.dataset['blink'], mood).toBeUndefined();
      c.advance(1);
      expect(genie()?.dataset['blink'], mood).toBe('');
      c.advance(TIMING.blinkLength);
      expect(genie()?.dataset['blink'], mood).toBeUndefined();
    }
    expect(EYES.sleeping).toBeUndefined();
    m.setMood('sleeping');
    c.advance(TIMING.blinkMax * 3);
    expect(genie()?.dataset['blink']).toBeUndefined();
    expect(c.pending()).toBe(0);
  });

  it('never blinks during the answer glint or the error shake', () => {
    const { m, c, root, genie } = mountLamp();
    c.advance(TIMING.blinkMin - 1); // a blink was due in 1 ms
    m.setMood('answering');
    expect(root()?.dataset['glint']).toBe('');
    c.advance(TIMING.glintLength);
    expect(root()?.dataset['glint']).toBeUndefined();
    expect(genie()?.dataset['blink']).toBeUndefined();
    expect(TIMING.blinkFastMin).toBeGreaterThan(TIMING.glintLength);
  });

  it('stops the rest timers in active states, and every timer on destroy', () => {
    const { m, c } = mountLamp();
    c.advance(TIMING.restAfter);
    const atRest = c.pending(); // the blink and the peek-up
    expect(atRest).toBe(2);
    m.setMood('processing');
    expect(c.pending()).toBe(1); // the blink only
    m.destroy();
    expect(c.pending()).toBe(0);
  });

  it('drives the level ring from the mic level, only while listening, in steps', () => {
    const { m, root } = mountLamp();
    m.setLevel(0.5);
    expect(root()?.style.getPropertyValue('--level')).toBe('');
    m.setMood('listening');
    m.setLevel(0.5);
    expect(root()?.style.getPropertyValue('--level')).toBe('0.500');
    m.setLevel(0.5 + LEVEL_STEP / 2);
    expect(root()?.style.getPropertyValue('--level')).toBe('0.500');
    m.setLevel(0.8);
    expect(root()?.style.getPropertyValue('--level')).toBe('0.800');
    m.setLevel(Number.NaN);
    expect(root()?.style.getPropertyValue('--level')).toBe('0.000');
    m.setMood('processing');
    expect(root()?.style.getPropertyValue('--level')).toBe('');
  });

  it('marks the answer-ready moment once, with a glint that ends', () => {
    const { m, c, root } = mountLamp();
    m.setMood('processing');
    m.setMood('answering');
    expect(root()?.dataset['glint']).toBe('');
    m.setMood('success'); // still ready: no second glint timer restart
    c.advance(TIMING.glintLength);
    expect(root()?.dataset['glint']).toBeUndefined();
  });

  it('shows the "?" cue over any state (UX-03) and peeks for it', () => {
    const { el, m, c, root } = mountLamp();
    c.advance(TIMING.restAfter);
    m.setAttention(true);
    expect(root()?.dataset['state']).toBe('peek');
    expect(el.querySelector('[data-mascot="attention"]')).not.toBeNull();
    m.setAttention(false);
    // A short exit first, then the node goes.
    const cue = el.querySelector<HTMLElement>('[data-mascot="attention"]');
    expect(cue?.dataset['out']).toBe('');
    m.setAttention(true); // back during the exit: the same bubble stays
    expect(el.querySelector('[data-mascot="attention"]')).toBe(cue);
    expect(cue?.dataset['out']).toBeUndefined();
    m.setAttention(false);
    c.advance(TIMING.cueOut);
    expect(el.querySelector('[data-mascot="attention"]')).toBeNull();
  });

  it('fades the old pose only for a swap at the same height (answering to success)', () => {
    const { m, genie } = mountLamp();
    m.setMood('answering');
    expect(genie()?.hasAttribute('data-fade')).toBe(false); // the FLIP move carries it
    m.setMood('success');
    expect(genie()?.hasAttribute('data-fade')).toBe(true);
    m.setMood('idle');
    expect(genie()?.hasAttribute('data-fade')).toBe(false);
  });

  it('reduced motion: no sink and no FLIP, but the state and the blink still show', () => {
    const { m, c, root, genie, y } = mountLamp(true);
    expect(root()?.dataset['motion']).toBe('reduced');
    c.advance(TIMING.restAfter);
    expect(root()?.dataset['state']).toBe('rest');
    expect(y()).toBeUndefined();
    // The blink still runs: the lid swaps by opacity (CSS), a state change, not motion.
    expect(c.pending()).toBe(1);
    c.advance(TIMING.blinkMin); // the rest state started a fresh wait
    expect(genie()?.dataset['blink']).toBe('');
    m.setMood('listening');
    m.setLevel(0.6);
    expect(root()?.style.getPropertyValue('--level')).toBe('0.600');
  });
});

describe('lamp motion CSS (#286)', () => {
  const css = readFileSync(resolve(import.meta.dirname, 'mog.css'), 'utf8');
  const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((r) => ({
    sel: (r[1] ?? '').trim(),
    body: r[2] ?? '',
  }));

  it('loops only in the active states, so an idle MOG runs no animation', () => {
    const loops = rules.filter((r) => /animation[^;]*infinite/.test(r.body));
    expect(loops.length).toBeGreaterThan(0);
    for (const r of loops) {
      expect(r.sel, r.sel).toMatch(/data-state='(listening|working|ready)'/);
    }
  });

  it('every active state loops in full and in reduced motion (never a frozen pose)', () => {
    const reduced = /@media \(prefers-reduced-motion: reduce\) \{([\s\S]*)\}\s*$/.exec(css)?.[1];
    expect(reduced, 'a reduced-motion block at the end of mog.css').toBeDefined();
    for (const s of ['listening', 'working', 'ready']) {
      const loop = new RegExp(`data-state='${s}'[^{]*\\{[^}]*animation[^;]*infinite`);
      expect(css, s).toMatch(loop);
      // Listening follows the level by opacity; the other two breathe by opacity.
      if (s === 'listening') expect(reduced).toMatch(/level-ring[^{]*\{[^}]*opacity:\s*calc/);
      else expect(reduced, s).toMatch(new RegExp(`data-state='${s}'[^{]*\\{[^}]*animation`));
    }
  });

  it('the working halo shows above the rim, clear of the lamp body', () => {
    const rule = rules.find((r) => /(^|\/|\s)\.lamp-glow$/.test(r.sel));
    const top = parseFloat(/top:\s*([\d.]+)%/.exec(rule?.body ?? '')?.[1] ?? 'NaN');
    const height = parseFloat(/height:\s*([\d.]+)%/.exec(rule?.body ?? '')?.[1] ?? 'NaN');
    // The rim is at about 69% of the art: most of the halo must be above it.
    expect(top + height / 2).toBeLessThan(60);
    expect(top).toBeGreaterThanOrEqual(0);
  });

  it('the answer ring stays inside the window at its largest', () => {
    const rule = rules.find((r) => /(^|\/|\s)\.answer-ring$/.test(r.sel));
    const pct = (k: string) =>
      parseFloat(new RegExp(`${k}:\\s*([\\d.]+)%`).exec(rule?.body ?? '')?.[1] ?? 'NaN');
    const peak = 1.1;
    const r = (pct('width') * peak) / 2;
    const cx = pct('left') + pct('width') / 2;
    const cy = pct('top') + pct('height') / 2;
    expect(cy - r).toBeGreaterThanOrEqual(0);
    expect(cx - r).toBeGreaterThanOrEqual(0);
    expect(cx + r).toBeLessThanOrEqual(100);
  });

  it('moves only transform-type properties and opacity', () => {
    const keyframes = [...css.matchAll(/@keyframes [\w-]+ \{([\s\S]*?)\n\}/g)].map((k) => k[1]);
    expect(keyframes.length).toBeGreaterThan(0);
    for (const k of keyframes) {
      const props = [...(k ?? '').matchAll(/([a-z-]+):/g)].map((p) => p[1]);
      for (const p of props) expect(['transform', 'translate', 'opacity', 'scale']).toContain(p);
    }
  });
});
