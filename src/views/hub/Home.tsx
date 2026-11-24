/**
 * The Home area: the launch screen the hub opens on. A small stage: the lamp mascot built from
 * its art layers (`mog-rive/fallback/layers`, the same planes the MOG draws), the "Bottle"
 * wordmark, the two keys that run Bottle, and the punchline at the bottom. No buttons:
 * navigation is the sidebar, and nothing here takes focus, so the focus order is the hub's own.
 *
 * Motion (ADR 0106: Motion, no second animation library):
 * - The entrance plays once, the first time Home is shown: one `animate` sequence (`INTRO`). The
 *   lamp rises in, the Genie peeks up out of it, the spark ignites with a burst, the wordmark
 *   rises letter by letter, then the key hints and the punchline arrive.
 * - Then the idle life starts (`data-phase="idle"`, all CSS loops in home.css): the lamp breathes,
 *   the Genie bobs over the rim, the spark orbits and twinkles, the glow pulses, the rays turn and
 *   a few specks twinkle. The eyes blink now and then (a timer), and the layers drift a few px
 *   with the pointer, at three depths (Motion springs, no React render per move).
 * - Every loop and the blink stop while Home is hidden or the window is not visible
 *   (`data-paused`). Only transform and opacity move, and every layer has its final size from
 *   the first frame, so nothing shifts.
 * - Reduced motion: no entrance, no loops, no parallax; the final frame is simply there
 *   (`data-motion="reduced"`).
 *
 * The art is one image to assistive tech (`role="img"` with the mascot's name); its layers are
 * aria-hidden.
 *
 * The setup steps (Hub.tsx `SetupRegion`) render above every area, Home included, so a pending
 * step shows above the landing and a first-run user is never blocked. The hub header owns the
 * page's one h1 ("Bottle"); the wordmark here is a decorative paragraph, not a second h1.
 */
import { useEffect, useRef, useState, type CSSProperties } from 'react';
import {
  motion,
  stagger,
  useAnimate,
  useMotionValue,
  useReducedMotion,
  useSpring,
  useTransform,
  type AnimationSequence,
} from 'motion/react';
import { EYES } from '../mog/lamp-mascot';
import lampBackPng from '../../../mog-rive/fallback/layers/lamp-back.png?url';
import lampFrontPng from '../../../mog-rive/fallback/layers/lamp-front.png?url';
import geniePng from '../../../mog-rive/fallback/layers/genie-idle.png?url';
import './home.css';

const WORDMARK = 'Bottle';

/** The gold four-point star of the MOG spark (lamp-mascot.ts), in a 48 x 48 box round 0,0. */
const STAR_PATH =
  'M0 -16 C1.6 -5 5 -1.6 16 0 C5 1.6 1.6 5 0 16 C-1.6 5 -5 1.6 -16 0 C-5 -1.6 -1.6 -5 0 -16Z';

/** The specks round the stage: x and y (% of the stage), size (px), loop (s), delay (s), tint. */
const SPECKS: readonly [number, number, number, number, number, 'spark' | 'eye'][] = [
  [14, 22, 3, 3.4, 0.2, 'spark'],
  [84, 18, 2, 4.1, 1.1, 'eye'],
  [24, 62, 2, 3.8, 2.0, 'eye'],
  [78, 54, 3, 4.6, 0.6, 'spark'],
  [92, 38, 2, 3.1, 1.7, 'spark'],
  [6, 44, 2, 4.4, 2.6, 'eye'],
  [64, 8, 2, 3.6, 0.9, 'spark'],
];

const EASE_OUT = [0.16, 1, 0.3, 1] as const; // --mog-ease-out
const EASE_BACK = [0.34, 1.56, 0.64, 1] as const; // a small overshoot, for the arrivals

/**
 * The entrance, about 2.1 s (seconds, `at` is the start on the timeline). Every element holds its
 * first keyframe until its own start, and the CSS `data-phase="intro"` gate hides it before the
 * first frame, so nothing flashes in its final place.
 */
const INTRO: AnimationSequence = [
  [
    '[data-home-backdrop]',
    { opacity: [0, 1], scale: [0.85, 1] },
    { duration: 1.4, at: 0, ease: EASE_OUT },
  ],
  [
    '[data-home-lamp]',
    { opacity: [0, 1], y: [56, 0], scale: [0.9, 1] },
    { duration: 0.8, at: 0, ease: EASE_BACK },
  ],
  ['[data-home-genie]', { y: ['26%', '0%'] }, { duration: 0.6, at: 0.4, ease: EASE_BACK }],
  [
    '[data-home-spark]',
    { opacity: [0, 1], scale: [0, 1.4, 1], rotate: [-135, 0] },
    { duration: 0.7, at: 0.75, ease: EASE_OUT },
  ],
  [
    '[data-home-burst]',
    { opacity: [0, 0.9, 0], scale: [0.2, 0.9, 2.6] },
    { duration: 0.8, at: 0.78, times: [0, 0.15, 1], ease: EASE_OUT },
  ],
  [
    '[data-home-letter]',
    { opacity: [0, 1], y: ['110%', '0%'] },
    { duration: 0.6, at: 0.85, delay: stagger(0.05), ease: EASE_OUT },
  ],
  [
    '[data-home-rule]',
    { opacity: [0, 1], scaleX: [0, 1] },
    { duration: 0.5, at: 1.25, ease: EASE_OUT },
  ],
  [
    '[data-home-hint]',
    { opacity: [0, 1], y: [10, 0] },
    { duration: 0.5, at: 1.35, delay: stagger(0.12), ease: EASE_OUT },
  ],
  [
    '[data-home-punchline]',
    { opacity: [0, 1], y: [6, 0] },
    { duration: 0.6, at: 1.55, ease: EASE_OUT },
  ],
];

/** The blink wait (ms, a fresh random value each time) and its length (as in the MOG). */
const BLINK_MIN = 3500;
const BLINK_MAX = 7000;
const BLINK_MS = 160;

/** How far each depth drifts with the pointer at the stage edge (px). Back goes the other way. */
const DEPTH = { back: -6, mid: 4, front: 12 } as const;

type Phase = 'intro' | 'idle';

/** True while the window is visible (`document.visibilityState`). */
function usePageVisible(): boolean {
  const [visible, setVisible] = useState(() => document.visibilityState !== 'hidden');
  useEffect(() => {
    const on = () => setVisible(document.visibilityState !== 'hidden');
    document.addEventListener('visibilitychange', on);
    return () => document.removeEventListener('visibilitychange', on);
  }, []);
  return visible;
}

export function Home({ hidden }: { hidden: boolean }) {
  const reduce = useReducedMotion() ?? false;
  const pageVisible = usePageVisible();
  const [played, setPlayed] = useState<Phase>('intro');
  // Reduced motion never plays the entrance: the final frame is the first frame.
  const phase: Phase = reduce ? 'idle' : played;
  const [scope, animate] = useAnimate<HTMLElement>();
  const started = useRef(false);
  const genie = useRef<HTMLDivElement>(null);
  const paused = hidden || !pageVisible;
  const live = !reduce && phase === 'idle' && !paused;

  // The entrance: once, the first time Home is on screen. Reduced motion skips it.
  useEffect(() => {
    if (reduce || hidden || started.current) return;
    started.current = true;
    const run = animate(INTRO);
    let done = false;
    let stopped = false;
    void run.then(() => {
      done = true;
      if (!stopped) setPlayed('idle');
    });
    return () => {
      // Hidden before the end (or a dev double mount): stop, and play it again when shown.
      if (done) return;
      stopped = true;
      run.stop();
      started.current = false;
    };
  }, [animate, hidden, reduce]);

  // The blink: a random wait, then the lids close for BLINK_MS. Only while the idle life runs.
  useEffect(() => {
    const el = genie.current;
    if (!live || !el) return;
    let timer = 0;
    const next = () => {
      timer = window.setTimeout(
        () => {
          el.setAttribute('data-blink', '');
          timer = window.setTimeout(() => {
            el.removeAttribute('data-blink');
            next();
          }, BLINK_MS);
        },
        BLINK_MIN + Math.random() * (BLINK_MAX - BLINK_MIN),
      );
    };
    next();
    return () => {
      window.clearTimeout(timer);
      el.removeAttribute('data-blink');
    };
  }, [live]);

  // The pointer parallax: -1..1 across the section, eased by a spring, three depths.
  const nx = useMotionValue(0);
  const ny = useMotionValue(0);
  const sx = useSpring(nx, { stiffness: 60, damping: 18, mass: 0.8 });
  const sy = useSpring(ny, { stiffness: 60, damping: 18, mass: 0.8 });
  const backX = useTransform(sx, (v) => v * DEPTH.back);
  const backY = useTransform(sy, (v) => v * DEPTH.back);
  const midX = useTransform(sx, (v) => v * DEPTH.mid);
  const midY = useTransform(sy, (v) => v * DEPTH.mid);
  const frontX = useTransform(sx, (v) => v * DEPTH.front);
  const frontY = useTransform(sy, (v) => v * DEPTH.front);
  useEffect(() => {
    const el = scope.current;
    if (!live || !el) return;
    const move = (e: PointerEvent) => {
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) return;
      nx.set(Math.max(-1, Math.min(1, ((e.clientX - r.left) / r.width) * 2 - 1)));
      ny.set(Math.max(-1, Math.min(1, ((e.clientY - r.top) / r.height) * 2 - 1)));
    };
    const leave = () => {
      nx.set(0);
      ny.set(0);
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerleave', leave);
    return () => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerleave', leave);
      leave();
    };
  }, [live, nx, ny, scope]);

  return (
    <section
      ref={scope}
      data-hub-view="home"
      aria-label="Home"
      hidden={hidden}
      className="hub-home"
      data-motion={reduce ? 'reduced' : 'full'}
      data-phase={phase}
      data-paused={paused ? '' : undefined}
    >
      <div className="hub-home-stage">
        <div
          className="home-art"
          data-home-art
          role="img"
          aria-label="The Bottle mascot, a small genie, peeking out of its lamp"
        >
          <motion.div className="home-depth" aria-hidden="true" style={{ x: backX, y: backY }}>
            <div className="home-backdrop" data-home-backdrop>
              <div className="home-rays" />
              <div className="home-glow" />
              {SPECKS.map(([x, y, size, loop, delay, tint], i) => (
                <span
                  key={i}
                  className="home-speck"
                  data-tint={tint}
                  style={
                    {
                      left: `${x}%`,
                      top: `${y}%`,
                      '--speck-size': `${size}px`,
                      '--speck-loop': `${loop}s`,
                      '--speck-delay': `${delay}s`,
                    } as CSSProperties
                  }
                />
              ))}
            </div>
          </motion.div>
          <motion.div className="home-depth" aria-hidden="true" style={{ x: midX, y: midY }}>
            <div className="home-lamp" data-home-lamp>
              <div className="home-breathe">
                <img className="home-layer" src={lampBackPng} alt="" draggable={false} />
                <div className="home-genie-clip">
                  <div className="home-genie" data-home-genie>
                    <div className="home-genie-bob" ref={genie}>
                      <img className="home-layer" src={geniePng} alt="" draggable={false} />
                      <svg className="home-lids" viewBox="0 0 400 400">
                        {(EYES.idle ?? []).map(([x, y, w, h]) => (
                          <rect key={x} x={x - 2} y={y - 2} width={w + 4} height={h + 4} rx={8} />
                        ))}
                      </svg>
                    </div>
                  </div>
                </div>
                <img className="home-layer" src={lampFrontPng} alt="" draggable={false} />
              </div>
            </div>
          </motion.div>
          <motion.div className="home-depth" aria-hidden="true" style={{ x: frontX, y: frontY }}>
            <div className="home-spark-anchor">
              <span className="home-burst" data-home-burst />
              <div className="home-spark" data-home-spark>
                <div className="home-spark-orbit">
                  <div className="home-spark-arm">
                    <div className="home-spark-turn">
                      <svg className="home-spark-star" viewBox="-24 -24 48 48">
                        <defs>
                          <radialGradient id="home-spark-halo">
                            <stop offset="0" className="home-spark-halo-in" />
                            <stop offset="1" className="home-spark-halo-out" />
                          </radialGradient>
                        </defs>
                        <circle r="24" fill="url(#home-spark-halo)" />
                        <path className="home-spark-body" d={STAR_PATH} />
                        <path className="home-spark-core" d={STAR_PATH} transform="scale(0.45)" />
                      </svg>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </motion.div>
        </div>
        <p className="hub-home-wordmark" data-home-wordmark>
          <span className="sr-only">{WORDMARK}</span>
          <span className="home-letters" aria-hidden="true">
            {[...WORDMARK].map((ch, i) => (
              <span key={i} className="home-letter-mask">
                <span className="home-letter" data-home-letter>
                  {ch}
                </span>
              </span>
            ))}
          </span>
        </p>
        <span className="home-rule" data-home-rule aria-hidden="true" />
        <ul className="home-hints" aria-label="Keyboard shortcuts">
          <li className="home-hint" data-home-hint>
            <span className="home-keys" aria-hidden="true">
              <kbd className="home-kbd">Right ⌥</kbd>
            </span>
            <span className="home-hint-label" aria-hidden="true">
              Hold to dictate
            </span>
            <span className="sr-only">Hold Right Option to dictate.</span>
          </li>
          <li className="home-hint" data-home-hint>
            <span className="home-keys" aria-hidden="true">
              <kbd className="home-kbd">Right ⌥</kbd>
              <span className="home-plus">+</span>
              <kbd className="home-kbd home-kbd--square">G</kbd>
            </span>
            <span className="home-hint-label" aria-hidden="true">
              Ask Genie
            </span>
            <span className="sr-only">Hold Right Option and tap G to ask Genie.</span>
          </li>
          <li className="home-hint" data-home-hint>
            <span className="home-keys" aria-hidden="true">
              <kbd className="home-kbd">Right ⌥</kbd>
              <span className="home-plus">+</span>
              <kbd className="home-kbd home-kbd--square">R</kbd>
            </span>
            <span className="home-hint-label" aria-hidden="true">
              Record a call
            </span>
            <span className="sr-only">
              Hold Right Option and tap R to start or stop a call recording.
            </span>
          </li>
        </ul>
      </div>
      <p className="hub-home-punchline" data-home-punchline>
        Your Genie, ready.
      </p>
    </section>
  );
}
