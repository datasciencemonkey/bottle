/**
 * The mascot renderer seam (ADR 0001). SP-01 (#10) and #169 pick the Rive runtime;
 * until then a DOM placeholder renders. The MOG view only talks to this interface.
 */
export interface MascotRenderer {
  mount(el: HTMLElement): void;
  setMood(mood: string): void;
  setLevel(level: number): void;
  destroy(): void;
}

/** A plain DOM stand-in for the Rive mascot. TODO(#10): replace with the SP-01 runtime. */
export class PlaceholderMascot implements MascotRenderer {
  #el: HTMLElement | undefined;

  mount(el: HTMLElement): void {
    this.#el = el;
    el.dataset['mood'] = 'idle';
    el.textContent = 'MOG';
  }

  setMood(mood: string): void {
    if (this.#el) this.#el.dataset['mood'] = mood;
  }

  setLevel(level: number): void {
    if (this.#el) this.#el.dataset['level'] = String(Math.max(0, Math.min(1, level)));
  }

  destroy(): void {
    this.#el?.replaceChildren();
    this.#el = undefined;
  }
}
