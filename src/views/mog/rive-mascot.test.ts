// @vitest-environment jsdom
/**
 * The Rive mascot view (UX-02a, UX-02b, UX-03, UX-08b, UX-09a, UX-09b) with a fake Rive runtime.
 * The real rendering in the transparent panel is a device check (needs:mac, see ADR 0027).
 */
import type { Mood } from '../../shared/interfaces';
import { FIRST_FRAME, MOODS, PngMascot, type MascotRenderer } from './mascot';
import { MOG_RIV_URL, RiveMascot, type RiveFactory, type RiveLike } from './rive-mascot';
import type { OfflineRiveOptions } from './rive-loader';

/** One fake view-model property that records every write. */
class Prop<T> {
  writes: T[] = [];
  #v: T;
  constructor(v: T) {
    this.#v = v;
  }
  get value(): T {
    return this.#v;
  }
  set value(v: T) {
    this.writes.push(v);
    this.#v = v;
  }
}

function fakeRive(
  enumValues: string[] = [
    'Idle',
    'Listening',
    'Processing',
    'Answering',
    'Success',
    'Error',
    'Sleeping',
  ],
) {
  // The art defaults differ from the first frame, so the UX-02b test sees each init write.
  const mood = Object.assign(new Prop<string>('Error'), { values: enumValues });
  const props = {
    mood,
    level: new Prop<number>(0.7),
    hover: new Prop<boolean>(true),
    demo: new Prop<boolean>(true),
    attention: new Prop<boolean>(false),
  };
  const pokes: number[] = [];
  const vm = {
    enum: (p: string) => (p === 'mood' ? props.mood : null),
    number: (p: string) => (p === 'level' ? props.level : null),
    boolean: (p: string) =>
      p === 'hover'
        ? props.hover
        : p === 'demo'
          ? props.demo
          : p === 'attention'
            ? props.attention
            : null,
    trigger: (p: string) => (p === 'poke' ? { trigger: () => pokes.push(1) } : null),
  };
  let cleaned = 0;
  let opts: OfflineRiveOptions | undefined;
  const rive: RiveLike = {
    get viewModelInstance() {
      return vm;
    },
    resizeDrawingSurfaceToCanvas: () => undefined,
    cleanup: () => {
      cleaned++;
    },
  };
  const create: RiveFactory = (o) => {
    opts = o;
    return rive;
  };
  const built = (): OfflineRiveOptions => {
    if (!opts) throw new Error('no Rive instance was built');
    return opts;
  };
  return {
    props,
    pokes,
    create,
    opts: built,
    load: () => built().onLoad?.(new Event('load') as never),
    fail: () => built().onLoadError?.(new Event('error') as never),
    cleaned: () => cleaned,
  };
}

function mountWith(create: RiveFactory, fallback?: () => MascotRenderer) {
  const el = document.createElement('div');
  document.body.append(el);
  const m = new RiveMascot(create, fallback);
  m.mount(el);
  return { el, m };
}

const staticPng = (el: HTMLElement) => el.querySelector<HTMLImageElement>('img[data-mascot="png"]');
/** The fallback pose on show: the lamp mascot's current Genie plane (#286). */
const pngMood = (el: HTMLElement) =>
  el.querySelector<HTMLImageElement>('[data-mascot="lamp"] img.genie-pose[data-on]');

describe('RiveMascot', () => {
  it('[UX-09a] [UX-08b] [SEC-20a] builds one offline instance in DrawOnChanged mode with the Rive listeners off', () => {
    const r = fakeRive();
    const { el } = mountWith(r.create);
    const o = r.opts();
    expect(o.src).toBe(MOG_RIV_URL);
    expect(o.drawingOptions).toBe('drawOnChanged');
    expect(o.shouldDisableRiveListeners).toBe(true);
    expect(o.autoBind).toBe(true);
    expect(o.canvas).toBeInstanceOf(HTMLCanvasElement);
    expect(el.querySelectorAll('canvas')).toHaveLength(1);
  });

  it('[UX-09a] the bundled .riv URL is a path on the app origin', () => {
    expect(MOG_RIV_URL).toBe('/mascot/mog.riv');
  });

  it('[UX-02b] [AT-20] on load sets demo=false, level=0, hover=false and mood=idle', () => {
    expect(FIRST_FRAME).toEqual({ demo: false, level: 0, hover: false, mood: 'idle' });
    const r = fakeRive();
    mountWith(r.create);
    r.load();
    expect(r.props.demo.writes).toEqual([false]);
    expect(r.props.level.writes).toEqual([0]);
    expect(r.props.hover.writes).toEqual([false]);
    expect(r.props.mood.writes).toEqual(['Idle']);
    expect(r.props.attention.writes).toEqual([]);
  });

  it('[UX-02b] [AT-20] state that arrives before the load is applied after the first frame', () => {
    const r = fakeRive();
    const { m } = mountWith(r.create);
    m.setMood('processing');
    m.setAttention(true);
    expect(r.props.mood.writes).toEqual([]);
    r.load();
    // The first frame comes first, then the state the brain sent.
    expect(r.props.mood.writes).toEqual(['Idle', 'Processing']);
    expect(r.props.attention.value).toBe(true);
  });

  it('[UX-02a] [AT-20] a poke fires the poke trigger and never turns on demo or changes the mood', () => {
    const r = fakeRive();
    const { m } = mountWith(r.create);
    r.load();
    m.setMood('success');
    const moodWrites = r.props.mood.writes.length;
    for (let i = 0; i < 10; i++) m.poke();
    expect(r.pokes).toHaveLength(10);
    expect(r.props.mood.writes).toHaveLength(moodWrites);
    expect(r.props.mood.value).toBe('Success');
    expect(r.props.demo.writes.every((d) => d === false)).toBe(true);
  });

  it('[UX-02a] [AT-20] sets each of the 7 moods by its Rive enum entry', () => {
    const r = fakeRive();
    const { m } = mountWith(r.create);
    r.load();
    for (const mood of MOODS) {
      m.setMood(mood);
      expect(r.props.mood.value.toLowerCase()).toBe(mood);
    }
  });

  it('[UX-02a] a Rive file whose mood enum is not exactly the 7 moods falls back to PNG', () => {
    for (const values of [
      ['idle', 'listening', 'processing', 'answering', 'success', 'error'],
      ['idle', 'listening', 'processing', 'answering', 'success', 'error', 'sleeping', 'dancing'],
    ]) {
      const r = fakeRive(values);
      const { el } = mountWith(r.create);
      r.load();
      expect(pngMood(el), values.join()).not.toBeNull();
      expect(el.querySelector('canvas')).toBeNull();
      expect(r.cleaned()).toBe(1);
    }
  });

  it('[UX-03] [AT-37] the attention cue shows over any mood', () => {
    const r = fakeRive();
    const { m } = mountWith(r.create);
    r.load();
    m.setAttention(true);
    for (const mood of MOODS) {
      m.setMood(mood);
      expect(r.props.attention.value).toBe(true);
    }
    m.setAttention(false);
    expect(r.props.attention.value).toBe(false);
  });

  it('[UX-08a] DOM hover drives the Rive hover input', () => {
    const r = fakeRive();
    const { m } = mountWith(r.create);
    r.load();
    m.setHover(true);
    expect(r.props.hover.value).toBe(true);
    m.setHover(false);
    expect(r.props.hover.value).toBe(false);
  });

  it('[UX-09a] [AT-21] writes a view-model value only when it changes, so an idle MOG draws no frames', () => {
    const r = fakeRive();
    const { m } = mountWith(r.create);
    r.load();
    const before = Object.fromEntries(
      Object.entries(r.props).map(([k, p]) => [k, p.writes.length]),
    );
    for (let i = 0; i < 50; i++) {
      m.setMood('idle');
      m.setLevel(0);
      m.setHover(false);
      m.setAttention(false);
    }
    const after = Object.fromEntries(Object.entries(r.props).map(([k, p]) => [k, p.writes.length]));
    expect(after).toEqual(before);
  });

  it('[UX-06a] clamps the level it sends to Rive into 0..1', () => {
    const r = fakeRive();
    const { m } = mountWith(r.create);
    r.load();
    m.setLevel(2);
    expect(r.props.level.value).toBe(1);
    m.setLevel(-1);
    expect(r.props.level.value).toBe(0);
    m.setLevel(Number.NaN);
    expect(r.props.level.value).toBe(0);
  });

  it('[UX-09b] [AT-21] a Rive load error swaps in the PNG fallback with the current mood and cue', () => {
    const r = fakeRive();
    const { el, m } = mountWith(r.create);
    m.setMood('error');
    m.setAttention(true);
    r.fail();
    expect(el.querySelector('canvas')).toBeNull();
    expect(pngMood(el)?.getAttribute('src')).toMatch(/genie-error\.png/);
    expect(el.querySelector('[data-mascot="attention"]')).not.toBeNull();
    expect(r.cleaned()).toBe(1);
    m.setMood('success');
    expect(pngMood(el)?.getAttribute('src')).toMatch(/genie-success\.png/);
  });

  it('[UX-09b] [AT-21] a runtime that throws or has no view model also falls back to PNG', () => {
    const throwing: RiveFactory = () => {
      throw new Error('no wasm');
    };
    expect(pngMood(mountWith(throwing).el)).not.toBeNull();
    const r = fakeRive();
    const noVm: RiveFactory = (o) => {
      const x = r.create(o);
      return { ...x, viewModelInstance: null, cleanup: x.cleanup };
    };
    const { el } = mountWith(noVm);
    r.load();
    expect(pngMood(el)).not.toBeNull();
  });

  it('[UX-09b] destroy cleans up the Rive instance and the DOM', () => {
    const r = fakeRive();
    const { el, m } = mountWith(r.create);
    r.load();
    m.destroy();
    expect(r.cleaned()).toBe(1);
    expect(el.childElementCount).toBe(0);
    r.fail();
    expect(el.childElementCount).toBe(0);
  });
});

describe('PngMascot (UX-09b fallback)', () => {
  it('[UX-09b] [AT-21] shows a static PNG for each of the 7 moods', () => {
    const el = document.createElement('div');
    const m = new PngMascot();
    m.mount(el);
    expect(staticPng(el)?.getAttribute('src')).toMatch(/fallback\/mood-idle\.png/);
    const seen = new Set<string>();
    for (const mood of MOODS) {
      m.setMood(mood);
      const src = staticPng(el)?.getAttribute('src') ?? '';
      expect(src).toMatch(new RegExp(`mood-${mood}\\.png`));
      seen.add(src);
    }
    expect(seen.size).toBe(7);
    expect(MOODS).toEqual([
      'idle',
      'listening',
      'processing',
      'answering',
      'success',
      'error',
      'sleeping',
    ] satisfies Mood[]);
  });

  it('[UX-03] [UX-09b] shows the "?" attention art over any mood', () => {
    const el = document.createElement('div');
    const m = new PngMascot();
    m.mount(el);
    const cue = () => el.querySelector<HTMLImageElement>('img[data-mascot="attention"]');
    expect(cue()).toBeNull();
    m.setAttention(true);
    for (const mood of MOODS) {
      m.setMood(mood);
      expect(cue()?.getAttribute('src')).toMatch(/fallback\/attention\.png/);
    }
    m.setAttention(false);
    expect(cue()).toBeNull();
  });

  it('[UX-09b] level, hover and poke are no-ops and destroy empties the element', () => {
    const el = document.createElement('div');
    const m = new PngMascot();
    m.mount(el);
    m.setLevel(1);
    m.setHover(true);
    m.poke();
    expect(staticPng(el)?.getAttribute('src')).toMatch(/mood-idle\.png/);
    m.destroy();
    expect(el.childElementCount).toBe(0);
  });
});

describe('lamp-alive: look and hidden reach the lamp fallback only', () => {
  it('forwards setLook and setHidden to the fallback, and the hidden state to a late one', () => {
    const calls: string[] = [];
    const fallback: MascotRenderer = {
      mount: () => undefined,
      setMood: () => undefined,
      setLevel: () => undefined,
      setAttention: () => undefined,
      setHover: () => undefined,
      poke: () => undefined,
      destroy: () => undefined,
      setLook: (dx, dy) => calls.push(`look:${dx},${dy}`),
      setHidden: (h) => calls.push(`hidden:${h}`),
    };
    const r = new RiveMascot(
      () => {
        throw new Error('no rive');
      },
      () => fallback,
    );
    r.setHidden(true);
    r.mount(document.createElement('div'));
    r.setLook(0.25, -0.5);
    r.setHidden(false);
    expect(calls).toEqual(['hidden:true', 'look:0.25,-0.5', 'hidden:false']);
  });

  it('does nothing before a fallback exists (the .riv has no look input yet)', () => {
    const r = new RiveMascot(() => ({
      viewModelInstance: null,
      resizeDrawingSurfaceToCanvas: () => undefined,
      cleanup: () => undefined,
    }));
    expect(() => {
      r.setLook(1, 0);
      r.setHidden(true);
    }).not.toThrow();
  });
});
