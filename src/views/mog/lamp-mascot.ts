/**
 * The lamp mascot (#286): the MOG Genie lives in its lamp and moves in and out of it.
 *
 * It draws three art planes from `mog-rive/fallback/layers/` (lamp mouth, Genie, lamp body) and
 * moves only the Genie, inside a fixed clip at the rim, so the Genie slides behind the rim. All
 * motion is CSS (`mog.css`, `.lamp`), on transform and opacity. This class picks the state, keeps
 * the Genie's height continuous when the pose changes (FLIP), and schedules the one-shot life
 * signs with timers: a blink in every pose with open eyes, and a small peek-up at rest.
 *
 * CPU: in the rest, peek, error and sleeping states no CSS loop and no animation frame runs.
 * Only listening, working and answer-ready loop, and only while they last.
 *
 * Like every renderer it only shows what the brain sends; it never picks a mood (UX-01b).
 */
import type { Mood } from '../../shared/interfaces';
import { ATTENTION_PNG, FIRST_FRAME, MOODS, type MascotRenderer } from './mascot';
import lampBackPng from '../../../mog-rive/fallback/layers/lamp-back.png?url';
import lampFrontPng from '../../../mog-rive/fallback/layers/lamp-front.png?url';
import answeringPng from '../../../mog-rive/fallback/layers/genie-answering.png?url';
import errorPng from '../../../mog-rive/fallback/layers/genie-error.png?url';
import idlePng from '../../../mog-rive/fallback/layers/genie-idle.png?url';
import listeningPng from '../../../mog-rive/fallback/layers/genie-listening.png?url';
import processingPng from '../../../mog-rive/fallback/layers/genie-processing.png?url';
import sleepingPng from '../../../mog-rive/fallback/layers/genie-sleeping.png?url';
import successPng from '../../../mog-rive/fallback/layers/genie-success.png?url';

/** The Genie plane of each mood. */
export const GENIE_PNG: Readonly<Record<Mood, string>> = {
  idle: idlePng,
  listening: listeningPng,
  processing: processingPng,
  answering: answeringPng,
  success: successPng,
  error: errorPng,
  sleeping: sleepingPng,
};
export const LAMP_BACK_PNG: string = lampBackPng;
export const LAMP_FRONT_PNG: string = lampFrontPng;

/** What the lamp shows. `rest` is the Genie back in its lamp; `peek` is it looking out. */
export type LampState = 'rest' | 'peek' | 'listening' | 'working' | 'ready' | 'error' | 'sleeping';

/** The states that loop while they last. Every other state is still between one-shots. */
export const ACTIVE_STATES: readonly LampState[] = ['listening', 'working', 'ready'];

/** The state for a mood and the pointer, cue and rest inputs. Pure. */
export function lampState(i: {
  mood: Mood;
  hover: boolean;
  attention: boolean;
  rested: boolean;
}): LampState {
  switch (i.mood) {
    case 'listening':
      return 'listening';
    case 'processing':
      return 'working';
    case 'answering':
    case 'success':
      return 'ready';
    case 'error':
      return 'error';
    case 'sleeping':
      return 'sleeping';
    case 'idle':
      return i.hover || i.attention || !i.rested ? 'peek' : 'rest';
  }
}

/**
 * How far below its standing height each pose draws the Genie: `RISE` of
 * `tools/gen_mog_rml.py` (art units) times the rig scale 1.1, in % of the 400 px art.
 */
const RISE: Readonly<Record<Mood, number>> = {
  sleeping: 230,
  idle: 76,
  processing: 52,
  error: 40,
  listening: 30,
  answering: -62,
  success: -62,
};
const ART_PCT = (1.1 / 400) * 100;
export const poseOffset = (from: Mood, to: Mood): number => (RISE[from] - RISE[to]) * ART_PCT;

/** The rest state sinks the idle pose this far (% of the art): the eyes just clear the rim. */
export const REST_SINK_PCT = 6;

/** Timing (ms). The motion itself is in mog.css; these are the scheduled one-shots. */
export const TIMING = {
  /** Idle with no pointer and no cue for this long: the Genie goes back into the lamp. */
  restAfter: 4000,
  /** The blink wait, a fresh random value each time. Listening and working blink a bit faster. */
  blinkMin: 3500,
  blinkMax: 7000,
  blinkFastMin: 2500,
  blinkFastMax: 5000,
  blinkLength: 160,
  peekUpMin: 25_000,
  peekUpMax: 45_000,
  peekUpLength: 2000,
  /** The answer ring burst: it starts as the Genie lands and ends inside this window. */
  glintLength: 900,
  /** The "?" cue exit, before its node goes. */
  cueOut: 200,
} as const;

/**
 * The eyes of each pose (art px, 400 px square), measured from `mog-rive/fallback/layers`.
 * One lid covers each eye for a blink. Sleeping has no open eyes, so it never blinks.
 */
export const EYES: Readonly<Partial<Record<Mood, readonly [number, number, number, number][]>>> = {
  idle: [
    [151, 230, 32, 42],
    [217, 230, 34, 42],
  ],
  listening: [
    [145, 175, 36, 48],
    [221, 175, 36, 48],
  ],
  processing: [
    [167, 207, 30, 25],
    [227, 207, 32, 25],
  ],
  answering: [
    [151, 78, 32, 43],
    [217, 82, 34, 39],
  ],
  success: [
    [153, 80, 28, 24],
    [221, 80, 26, 24],
  ],
  error: [
    [156, 195, 30, 32],
    [222, 198, 30, 32],
  ],
};

/** The level ring redraws only for a change this big (0..1). */
export const LEVEL_STEP = 0.02;

/** Move durations (ms) by where the Genie goes; arrivals are slower than exits. */
const MOVE_MS: Readonly<Record<LampState, number>> = {
  rest: 520,
  sleeping: 520,
  peek: 360,
  listening: 300,
  working: 360,
  ready: 560,
  error: 300,
};

export interface LampClock {
  setTimeout(fn: () => void, ms: number): number;
  clearTimeout(id: number): void;
}

export interface LampMascotOptions {
  clock?: LampClock;
  /** 0 <= r < 1. */
  random?: () => number;
  /** `prefers-reduced-motion: reduce`. */
  reducedMotion?: () => boolean;
}

const defaultClock: LampClock = {
  setTimeout: (fn, ms) => window.setTimeout(fn, ms),
  clearTimeout: (id) => window.clearTimeout(id),
};

const prefersReducedMotion = (): boolean =>
  typeof window !== 'undefined' &&
  typeof window.matchMedia === 'function' &&
  window.matchMedia('(prefers-reduced-motion: reduce)').matches;

const SVG = 'http://www.w3.org/2000/svg';

function svg(viewBox: string, cls: string, children: string): SVGSVGElement {
  const el = document.createElementNS(SVG, 'svg');
  el.setAttribute('viewBox', viewBox);
  el.setAttribute('class', cls);
  el.setAttribute('aria-hidden', 'true');
  // Static, trusted markup only (no input reaches it).
  el.innerHTML = children;
  return el;
}

function img(cls: string, src: string): HTMLImageElement {
  const el = document.createElement('img');
  el.className = cls;
  el.alt = '';
  el.draggable = false;
  el.decoding = 'async';
  el.src = src;
  return el;
}

/** The translate Y of an element now, in % of its height (it may be mid-transition). */
function currentY(el: HTMLElement, fallback: number): number {
  const h = el.offsetHeight;
  const t = getComputedStyle(el).translate;
  if (!h || !t || t === 'none') return fallback;
  const parts = t.trim().split(/\s+/);
  const y = parts[1] ?? '0px';
  if (y.endsWith('%')) return parseFloat(y);
  const px = parseFloat(y);
  return Number.isNaN(px) ? fallback : (px / h) * 100;
}

type TimerKey = 'rest' | 'blink' | 'blinkEnd' | 'peekUp' | 'peekUpEnd' | 'glint' | 'cueOut';

export class LampMascot implements MascotRenderer {
  readonly #clock: LampClock;
  readonly #random: () => number;
  readonly #reduced: () => boolean;
  #root: HTMLDivElement | undefined;
  #genie: HTMLDivElement | undefined;
  #poses: Partial<Record<Mood, HTMLImageElement>> = {};
  #cue: HTMLImageElement | undefined;
  #glow: SVGSVGElement | undefined;
  #mood: Mood = FIRST_FRAME.mood;
  #hover = false;
  #attention = false;
  #rested = false;
  #state: LampState | undefined;
  #level = -1;
  #timers = new Map<TimerKey, number>();
  /** The Genie's last target height (% of the art). */
  #lastY = 0;

  constructor(opts: LampMascotOptions = {}) {
    this.#clock = opts.clock ?? defaultClock;
    this.#random = opts.random ?? Math.random;
    this.#reduced = opts.reducedMotion ?? prefersReducedMotion;
  }

  mount(el: HTMLElement): void {
    const root = document.createElement('div');
    root.className = 'lamp';
    root.dataset['mascot'] = 'lamp';
    root.append(img('lamp-back', LAMP_BACK_PNG));
    // A warm halo behind the head while Genie works. It rises from the lamp mouth, so most of
    // it shows above the rim and round the head.
    const glow = svg(
      '0 0 100 100',
      'lamp-glow',
      '<ellipse cx="50" cy="50" rx="46" ry="44"></ellipse>',
    );
    root.append(glow);
    this.#glow = glow;
    // The level ring sits behind the Genie and the lamp body, like the rings of the art.
    const ring = document.createElement('span');
    ring.className = 'level-ring';
    root.append(ring);
    // The "response is here" moment: a gold ring bursts out from behind the standing head as
    // it lands. Behind the Genie, so it never crosses the face.
    const answer = document.createElement('span');
    answer.className = 'answer-ring';
    root.append(answer);
    const clip = document.createElement('div');
    clip.className = 'genie-clip';
    const genie = document.createElement('div');
    genie.className = 'genie';
    // The body carries the loops (breathe, bob, sway, shake), so the lids move with the eyes.
    const body = document.createElement('div');
    body.className = 'genie-body';
    for (const m of MOODS) {
      const pose = img('genie-pose', GENIE_PNG[m]);
      pose.dataset['pose'] = m;
      this.#poses[m] = pose;
      body.append(pose);
    }
    // One shared lid layer in the screen colour of the art: a lid pair per pose.
    const lids = MOODS.map((m) =>
      (EYES[m] ?? [])
        .map(([x, y, w, h]) => `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="10"></rect>`)
        .join(''),
    )
      .map((rects, i) => `<g data-lids="${MOODS[i]}">${rects}</g>`)
      .join('');
    body.append(svg('0 0 400 400', 'eyelids', lids));
    genie.append(body);
    clip.append(genie);
    root.append(clip, img('lamp-front', LAMP_FRONT_PNG));
    this.#root = root;
    this.#genie = genie;
    el.append(root);
    this.#apply(true);
  }

  setMood(mood: Mood): void {
    if (mood === this.#mood) return;
    this.#mood = mood;
    if (mood !== 'idle') this.#rested = false;
    this.#apply();
  }

  setAttention(on: boolean): void {
    if (on === this.#attention) return;
    this.#attention = on;
    if (on) this.#rested = false;
    this.#apply();
  }

  setHover(on: boolean): void {
    if (on === this.#hover) return;
    this.#hover = on;
    if (on) this.#rested = false;
    this.#apply();
  }

  /** The mic level drives the listening ring (UX-06a). Other states ignore it. */
  setLevel(level: number): void {
    const root = this.#root;
    if (!root || this.#state !== 'listening') return;
    const v = Number.isNaN(level) ? 0 : Math.min(1, Math.max(0, level));
    if (Math.abs(v - this.#level) < LEVEL_STEP && !(v === 0 && this.#level !== 0)) return;
    this.#level = v;
    root.style.setProperty('--level', v.toFixed(3));
  }

  /** A poke is a peek: the Genie looks out. It never changes the mood (UX-02a). */
  poke(): void {
    if (this.#state !== 'rest') return;
    this.#rested = false;
    this.#apply();
  }

  destroy(): void {
    for (const id of this.#timers.values()) this.#clock.clearTimeout(id);
    this.#timers.clear();
    this.#root?.remove();
    this.#root = this.#genie = this.#cue = this.#glow = undefined;
    this.#poses = {};
    this.#state = undefined;
  }

  /** For tests and the frame strip: the state the lamp shows now. */
  get state(): LampState | undefined {
    return this.#state;
  }

  #after(key: TimerKey, ms: number, fn: () => void): void {
    this.#cancel(key);
    this.#timers.set(
      key,
      this.#clock.setTimeout(() => {
        this.#timers.delete(key);
        fn();
      }, ms),
    );
  }

  #cancel(...keys: TimerKey[]): void {
    for (const k of keys) {
      const id = this.#timers.get(k);
      if (id !== undefined) this.#clock.clearTimeout(id);
      this.#timers.delete(k);
    }
  }

  #between(min: number, max: number): number {
    return Math.round(min + this.#random() * (max - min));
  }

  #apply(first = false): void {
    const root = this.#root;
    const genie = this.#genie;
    if (!root || !genie) return;
    const reduced = this.#reduced();
    root.dataset['motion'] = reduced ? 'reduced' : 'full';
    const prevState = this.#state;
    const prevMood = root.dataset['mood'] as Mood | undefined;
    const next = lampState({
      mood: this.#mood,
      hover: this.#hover,
      attention: this.#attention,
      rested: this.#rested,
    });
    this.#state = next;

    // The glow loops in working (and, with reduced motion, in listening). When that loop
    // ends, hand its current opacity to the CSS transition, so it fades out and never snaps.
    const glow = this.#glow;
    const glowFrom =
      glow && prevState !== next && (prevState === 'working' || prevState === 'listening')
        ? parseFloat(getComputedStyle(glow).opacity)
        : Number.NaN;

    // The pose: one image per mood, all decoded up front, so a swap never flashes. A swap at
    // the same height (answering to success) fades the old pose out; a swap with a height
    // change is a cut that the FLIP move carries, so no ghost of the old pose moves with it.
    const sameHeight =
      prevMood !== undefined && prevMood !== this.#mood && poseOffset(prevMood, this.#mood) === 0;
    genie.toggleAttribute('data-fade', sameHeight);
    for (const m of MOODS) this.#poses[m]?.toggleAttribute('data-on', m === this.#mood);
    root.dataset['mood'] = this.#mood;
    root.dataset['state'] = next;
    if (glow && glowFrom > 0) {
      glow.style.opacity = String(glowFrom);
      void getComputedStyle(glow).opacity;
      glow.style.opacity = '';
    }

    // The Genie's height: continuous across a pose swap (FLIP), then a move to the target.
    const target = next === 'rest' && !reduced ? REST_SINK_PCT : 0;
    if (!reduced) {
      const before = first ? target : currentY(genie, this.#lastY);
      const swap = prevMood !== undefined && prevMood !== this.#mood;
      const from = swap ? before + poseOffset(prevMood, this.#mood) : before;
      const up = target < from;
      genie.style.setProperty('--move-ms', `${MOVE_MS[next]}ms`);
      genie.dataset['dir'] = up ? 'up' : 'down';
      if (from !== target) {
        genie.dataset['moving'] = '';
        genie.style.translate = `0 ${from.toFixed(3)}%`;
        // Commit the start position before the move (one forced layout, only on a change).
        void genie.offsetHeight;
        delete genie.dataset['moving'];
      }
      genie.style.translate = `0 ${target.toFixed(3)}%`;
    } else {
      genie.style.translate = '';
    }
    this.#lastY = target;

    if (next !== 'listening') {
      this.#level = -1;
      root.style.removeProperty('--level');
    }
    this.#renderCue();
    this.#schedule(prevState, next, reduced);
  }

  #renderCue(): void {
    const root = this.#root;
    if (!root) return;
    const cue = this.#cue;
    if (this.#attention) {
      if (cue) {
        // Back during its exit: keep the same bubble.
        this.#cancel('cueOut');
        delete cue.dataset['out'];
        return;
      }
      const el = img('lamp-cue', ATTENTION_PNG);
      el.dataset['mascot'] = 'attention';
      this.#cue = el;
      root.append(el);
    } else if (cue && cue.dataset['out'] === undefined) {
      // A short exit, then the node goes.
      cue.dataset['out'] = '';
      this.#after('cueOut', TIMING.cueOut, () => {
        cue.remove();
        if (this.#cue === cue) this.#cue = undefined;
      });
    }
  }

  #schedule(prev: LampState | undefined, next: LampState, reduced: boolean): void {
    const root = this.#root;
    const genie = this.#genie;
    if (!root || !genie) return;
    // Back into the lamp after a calm moment.
    if (next === 'peek' && this.#mood === 'idle' && !this.#hover && !this.#attention) {
      this.#after('rest', TIMING.restAfter, () => {
        this.#rested = true;
        this.#apply();
      });
    } else {
      this.#cancel('rest');
    }
    // Life signs: a blink in every pose with open eyes, also with reduced motion (there the
    // lid swaps by opacity). A new state starts a fresh wait, which is longer than the glint
    // and the head shake, so a blink never meets them.
    if (next === 'sleeping' || prev !== next) {
      this.#cancel('blink', 'blinkEnd');
      delete genie.dataset['blink'];
    }
    if (next !== 'sleeping' && !this.#timers.has('blink') && !this.#timers.has('blinkEnd')) {
      this.#nextBlink();
    }
    if (next === 'rest' && !reduced) {
      if (!this.#timers.has('peekUp') && !this.#timers.has('peekUpEnd')) this.#nextPeekUp();
    } else {
      this.#cancel('peekUp', 'peekUpEnd');
      delete genie.dataset['peekUp'];
    }
    // The "response is here" ring burst, once, as the Genie comes out with the answer.
    if (next === 'ready' && prev !== 'ready') {
      root.dataset['glint'] = '';
      this.#after('glint', TIMING.glintLength, () => {
        if (this.#root) delete this.#root.dataset['glint'];
      });
    } else if (next !== 'ready') {
      this.#cancel('glint');
      delete root.dataset['glint'];
    }
  }

  #nextBlink(): void {
    const fast = this.#state === 'listening' || this.#state === 'working';
    const wait = fast
      ? this.#between(TIMING.blinkFastMin, TIMING.blinkFastMax)
      : this.#between(TIMING.blinkMin, TIMING.blinkMax);
    this.#after('blink', wait, () => {
      const genie = this.#genie;
      if (!genie) return;
      genie.dataset['blink'] = '';
      this.#after('blinkEnd', TIMING.blinkLength, () => {
        if (this.#genie) delete this.#genie.dataset['blink'];
        this.#nextBlink();
      });
    });
  }

  #nextPeekUp(): void {
    this.#after('peekUp', this.#between(TIMING.peekUpMin, TIMING.peekUpMax), () => {
      const genie = this.#genie;
      if (!genie) return;
      genie.dataset['peekUp'] = '';
      this.#after('peekUpEnd', TIMING.peekUpLength, () => {
        if (this.#genie) delete this.#genie.dataset['peekUp'];
        this.#nextPeekUp();
      });
    });
  }
}
