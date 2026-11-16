/**
 * The mascot renderer seam (ADR 0002, ADR 0027). The MOG view only talks to this interface.
 * `RiveMascot` (rive-mascot.ts) is the renderer; `PngMascot` is its static fallback (UX-09b).
 * A renderer only shows what the brain sends: it never picks a mood (UX-01b).
 */
import type { Mood } from '../../shared/interfaces';
import answeringPng from '../../../mog-rive/fallback/mood-answering.png?url';
import errorPng from '../../../mog-rive/fallback/mood-error.png?url';
import idlePng from '../../../mog-rive/fallback/mood-idle.png?url';
import listeningPng from '../../../mog-rive/fallback/mood-listening.png?url';
import processingPng from '../../../mog-rive/fallback/mood-processing.png?url';
import sleepingPng from '../../../mog-rive/fallback/mood-sleeping.png?url';
import successPng from '../../../mog-rive/fallback/mood-success.png?url';
import attentionPng from '../../../mog-rive/fallback/attention.png?url';

export interface MascotRenderer {
  mount(el: HTMLElement): void;
  setMood(mood: Mood): void;
  /** 0..1 from the level meter (UX-06a). */
  setLevel(level: number): void;
  /** The "?" cue over any mood (UX-03). */
  setAttention(on: boolean): void;
  /** DOM hover (UX-08a); the Rive listeners stay off (UX-08b). */
  setHover(on: boolean): void;
  /** A visual poke. It never changes the mood (UX-02a). */
  poke(): void;
  destroy(): void;
}

/** The 7 moods of the Rive file (UX-02a), in the order of its `MogMood` enum. */
export const MOODS: readonly Mood[] = [
  'idle',
  'listening',
  'processing',
  'answering',
  'success',
  'error',
  'sleeping',
];

/** UX-02b: the inputs a renderer sets when it loads, before the brain's first state. */
export const FIRST_FRAME = { demo: false, level: 0, hover: false, mood: 'idle' } as const;

/** UX-09b: one transparent PNG per mood (`mog-rive/fallback/`, `tools/render_mog_fallback.py`). */
export const MOOD_PNG: Readonly<Record<Mood, string>> = {
  idle: idlePng,
  listening: listeningPng,
  processing: processingPng,
  answering: answeringPng,
  success: successPng,
  error: errorPng,
  sleeping: sleepingPng,
};

/** UX-03: only the "?" bubble, transparent elsewhere, so it shows over any mood. */
export const ATTENTION_PNG: string = attentionPng;

/** The static fallback when Rive does not load (UX-09b). */
export class PngMascot implements MascotRenderer {
  #el: HTMLElement | undefined;
  #img: HTMLImageElement | undefined;
  #cue: HTMLImageElement | undefined;
  #mood: Mood = FIRST_FRAME.mood;
  #attention = false;

  mount(el: HTMLElement): void {
    this.#el = el;
    const img = document.createElement('img');
    img.dataset['mascot'] = 'png';
    img.alt = '';
    img.draggable = false;
    this.#img = img;
    el.append(img);
    this.#render();
  }

  setMood(mood: Mood): void {
    this.#mood = mood;
    this.#render();
  }

  setAttention(on: boolean): void {
    this.#attention = on;
    this.#render();
  }

  setLevel(_level: number): void {}
  setHover(_on: boolean): void {}
  poke(): void {}

  destroy(): void {
    this.#el?.replaceChildren();
    this.#el = this.#img = this.#cue = undefined;
  }

  #render(): void {
    if (!this.#el || !this.#img) return;
    this.#img.src = MOOD_PNG[this.#mood];
    if (this.#attention && !this.#cue) {
      const cue = document.createElement('img');
      cue.dataset['mascot'] = 'attention';
      cue.alt = '';
      cue.draggable = false;
      cue.src = ATTENTION_PNG;
      this.#cue = cue;
      this.#el.append(cue);
    } else if (!this.#attention && this.#cue) {
      this.#cue.remove();
      this.#cue = undefined;
    }
  }
}
