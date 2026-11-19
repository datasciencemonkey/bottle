/**
 * The Rive mascot renderer (ADR 0027): one Rive instance per MOG view, built offline through
 * `createOfflineRive` (SEC-20a, ADR 0021).
 *
 * - The Rive listeners are off (UX-08b); the DOM drives hover and poke (UX-08a).
 * - `DrawOnChanged` (UX-09a): Rive draws only when an input changes, and this renderer writes a
 *   view-model value only when it differs, so an idle MOG draws no frames.
 * - On load it sets `demo=false`, `level=0`, `hover=false`, `mood=idle` (UX-02b), then the state
 *   the brain has sent so far. It never sets `demo=true`, so a poke never cycles the moods (UX-02a).
 * - If the file does not load, the runtime throws, or the view model is not the one the art
 *   defines (exactly the 7 moods, UX-02a), it shows `LampMascot` instead (UX-09b, #286): the
 *   fallback art in layers, with the lamp motion in CSS.
 */
import type { Mood } from '../../shared/interfaces';
import { LampMascot } from './lamp-mascot';
import { FIRST_FRAME, MOODS, type MascotRenderer } from './mascot';
import { DrawOptimizationOptions, createOfflineRive, type OfflineRiveOptions } from './rive-loader';

/** The bundled `.riv` (emitted by the `bottle-mog-riv` Vite plugin, ADR 0027). */
export const MOG_RIV_URL = '/mascot/mog.riv';

/** The view-model calls this renderer makes (the `Mog` view model in `mog-rive/scene.rml`). */
export interface RiveViewModel {
  enum(path: string): { value: string; readonly values: string[] } | null;
  number(path: string): { value: number } | null;
  boolean(path: string): { value: boolean } | null;
  trigger(path: string): { trigger(): void } | null;
}

/** The part of a Rive instance this renderer uses. */
export interface RiveLike {
  readonly viewModelInstance: RiveViewModel | null;
  resizeDrawingSurfaceToCanvas(): void;
  cleanup(): void;
}

export type RiveFactory = (opts: OfflineRiveOptions) => RiveLike;

interface Bound {
  mood: NonNullable<ReturnType<RiveViewModel['enum']>>;
  level: NonNullable<ReturnType<RiveViewModel['number']>>;
  hover: NonNullable<ReturnType<RiveViewModel['boolean']>>;
  demo: NonNullable<ReturnType<RiveViewModel['boolean']>>;
  attention: NonNullable<ReturnType<RiveViewModel['boolean']>>;
  poke: NonNullable<ReturnType<RiveViewModel['trigger']>>;
  /** The Rive enum entry for each mood (the runtime may report keys or display names). */
  entries: Record<Mood, string>;
}

const clamp01 = (v: number): number => (Number.isNaN(v) ? 0 : Math.min(1, Math.max(0, v)));

/** Bind the view model, or undefined when it is not the mascot's. */
function bind(vm: RiveViewModel | null): Bound | undefined {
  if (!vm) return undefined;
  const mood = vm.enum('mood');
  const level = vm.number('level');
  const hover = vm.boolean('hover');
  const demo = vm.boolean('demo');
  const attention = vm.boolean('attention');
  const poke = vm.trigger('poke');
  if (!mood || !level || !hover || !demo || !attention || !poke) return undefined;
  const values = mood.values;
  if (values.length !== MOODS.length) return undefined;
  const entries = {} as Record<Mood, string>;
  for (const m of MOODS) {
    const entry = values.find((v) => v.toLowerCase() === m);
    if (entry === undefined) return undefined;
    entries[m] = entry;
  }
  return { mood, level, hover, demo, attention, poke, entries };
}

function set<T>(prop: { value: T }, v: T): void {
  if (prop.value !== v) prop.value = v;
}

export class RiveMascot implements MascotRenderer {
  readonly #create: RiveFactory;
  readonly #fallback: () => MascotRenderer;
  #el: HTMLElement | undefined;
  #canvas: HTMLCanvasElement | undefined;
  #rive: RiveLike | undefined;
  #bound: Bound | undefined;
  #png: MascotRenderer | undefined;
  #state: { mood: Mood; level: number; hover: boolean; attention: boolean } = {
    mood: FIRST_FRAME.mood,
    level: FIRST_FRAME.level,
    hover: FIRST_FRAME.hover,
    attention: false,
  };

  constructor(
    create: RiveFactory = createOfflineRive,
    fallback: () => MascotRenderer = () => new LampMascot(),
  ) {
    this.#create = create;
    this.#fallback = fallback;
  }

  mount(el: HTMLElement): void {
    this.#el = el;
    const canvas = document.createElement('canvas');
    canvas.dataset['mascot'] = 'rive';
    this.#canvas = canvas;
    el.append(canvas);
    try {
      this.#rive = this.#create({
        src: MOG_RIV_URL,
        canvas,
        autoplay: true,
        autoBind: true,
        shouldDisableRiveListeners: true,
        drawingOptions: DrawOptimizationOptions.DrawOnChanged,
        onLoad: () => this.#onLoad(),
        onLoadError: () => this.#toPng(),
      });
    } catch {
      this.#toPng();
    }
  }

  setMood(mood: Mood): void {
    const s = this.#state;
    s.mood = mood;
    this.#png?.setMood(s.mood);
    if (this.#bound) set(this.#bound.mood, this.#bound.entries[s.mood]);
  }

  setLevel(level: number): void {
    this.#state.level = clamp01(level);
    this.#png?.setLevel(this.#state.level);
    if (this.#bound) set(this.#bound.level, this.#state.level);
  }

  setAttention(on: boolean): void {
    this.#state.attention = on;
    this.#png?.setAttention(on);
    if (this.#bound) set(this.#bound.attention, on);
  }

  setHover(on: boolean): void {
    this.#state.hover = on;
    this.#png?.setHover(on);
    if (this.#bound) set(this.#bound.hover, on);
  }

  poke(): void {
    this.#png?.poke();
    this.#bound?.poke.trigger();
  }

  destroy(): void {
    this.#rive?.cleanup();
    this.#png?.destroy();
    this.#el?.replaceChildren();
    this.#el = this.#canvas = this.#rive = this.#bound = this.#png = undefined;
  }

  #onLoad(): void {
    if (!this.#rive) return;
    const b = bind(this.#rive.viewModelInstance);
    if (!b) {
      this.#toPng();
      return;
    }
    this.#rive.resizeDrawingSurfaceToCanvas();
    // UX-02b: the first frame, then whatever the brain sent before the load.
    set(b.demo, FIRST_FRAME.demo);
    set(b.level, FIRST_FRAME.level);
    set(b.hover, FIRST_FRAME.hover);
    set(b.mood, b.entries[FIRST_FRAME.mood]);
    this.#bound = b;
    const s = this.#state;
    this.setMood(s.mood);
    this.setLevel(s.level);
    this.setHover(s.hover);
    this.setAttention(s.attention);
  }

  /** UX-09b: drop Rive and show the fallback for the current mood, cue and hover. */
  #toPng(): void {
    const el = this.#el;
    if (!el || this.#png) return;
    this.#rive?.cleanup();
    this.#rive = this.#bound = undefined;
    this.#canvas?.remove();
    this.#canvas = undefined;
    const s = this.#state;
    const png = this.#fallback();
    png.mount(el);
    png.setMood(s.mood);
    png.setAttention(s.attention);
    png.setHover(s.hover);
    this.#png = png;
  }
}
