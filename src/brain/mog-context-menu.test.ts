/**
 * #ui-4 (review #603, findings 1 and 4): the brain side of the native MOG right-click menu.
 */
import { core } from './test-core';
import { BrainBus, onViewInput } from './bus';
import {
  createAppMogContextMenu,
  popupMogMenu,
  wireMogContextMenu,
  type MogContextMenuDeps,
} from './mog-context-menu';
import { MOG_MENU_HIDE_EVENT, MOG_MENU_OPEN, OPEN_CONVERSATIONS } from '../shared/genie-status';
import type { MogMenuAction } from './mog-menu-items';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

/**
 * A test double for `@tauri-apps/api/window` and `@tauri-apps/api/menu`, for `popupMogMenu` /
 * `createAppMogContextMenu` only (`wireMogContextMenu`'s own tests inject plain fakes instead).
 * `vi.hoisted` so the `vi.mock` factories below (hoisted above these imports) can close over it.
 */
const menuDouble = vi.hoisted(() => ({
  windows: new Map<string, { label: string }>(),
  builtItems: undefined as { id: string; text: string; action: () => void }[] | undefined,
  popupArgs: undefined as { at: unknown; window: unknown } | undefined,
  reset() {
    this.windows.clear();
    this.builtItems = undefined;
    this.popupArgs = undefined;
  },
}));

vi.mock('@tauri-apps/api/window', () => ({
  Window: { getByLabel: async (label: string) => menuDouble.windows.get(label) ?? null },
}));
vi.mock('@tauri-apps/api/menu', () => ({
  Menu: {
    new: async (opts: { items?: { id: string; text: string; action: () => void }[] }) => {
      menuDouble.builtItems = opts.items ?? [];
      return {
        popup: async (at: unknown, window: unknown) => {
          menuDouble.popupArgs = { at, window };
        },
      };
    },
  },
}));

const A = '00000000-0000-4000-8000-00000000000a';

function input(bus: BrainBus, from: string, mogId: string, kind: string, payload?: unknown) {
  bus.dispatch({ event: 'view:input', payload: { from, input: { mogId, kind, payload } } });
}

function setup() {
  const bus = new BrainBus();
  const log: unknown[][] = [];
  let onPick: ((action: MogMenuAction) => void) | undefined;
  const deps: MogContextMenuDeps = {
    popup: async (mogId, x, y, hidden, pick) => {
      log.push(['popup', mogId, x, y, hidden]);
      onPick = pick;
    },
    replayClick: (mogId, target) => void log.push(['replay', mogId, target]),
    setHidden: async (mogId, hidden) => void log.push(['hidden', mogId, hidden]),
  };
  const off = wireMogContextMenu(bus, deps);
  const pick = (action: MogMenuAction) => onPick?.(action);
  return { bus, log, off, pick };
}

describe('MOG context menu wiring (#ui-4, review #603)', () => {
  it('a MOG_MENU_OPEN click from the MOG asks the deps to pop up the menu at its position and hidden state', () => {
    const { bus, log } = setup();
    input(bus, `mog-${A}`, A, 'click', { target: MOG_MENU_OPEN, x: 12, y: 34, hidden: true });
    expect(log).toEqual([['popup', A, 12, 34, true]]);
  });

  it('ignores a MOG_MENU_OPEN with a bad shape, from the hub, or from another kind of click', () => {
    const { bus, log } = setup();
    input(bus, `mog-${A}`, A, 'click', { target: MOG_MENU_OPEN, x: '1', y: 2 });
    input(bus, `mog-${A}`, A, 'click', { target: MOG_MENU_OPEN });
    input(bus, 'hub', 'hub', 'click', { target: MOG_MENU_OPEN, x: 1, y: 2 });
    input(bus, `mog-${A}`, A, 'poked', { target: MOG_MENU_OPEN, x: 1, y: 2 });
    input(bus, `mog-${A}`, A, 'click', { x: 1, y: 2 });
    expect(log).toEqual([]);
  });

  it('[hide] tells setHidden to flip the state the view reported, with no replay (view-local)', () => {
    const { bus, log, pick } = setup();
    input(bus, `mog-${A}`, A, 'click', { target: MOG_MENU_OPEN, x: 1, y: 2, hidden: false });
    pick('hide');
    expect(log).toEqual([
      ['popup', A, 1, 2, false],
      ['hidden', A, true],
    ]);
  });

  it('[close/open_in_bottle/change_home] replay the exact click a real HTML-menu click used to send', () => {
    const { bus, log, pick } = setup();
    input(bus, `mog-${A}`, A, 'click', { target: MOG_MENU_OPEN, x: 1, y: 2, hidden: false });
    pick('close');
    pick('open_in_bottle');
    pick('change_home');
    expect(log.slice(1)).toEqual([
      ['replay', A, 'mog_menu_close'],
      ['replay', A, OPEN_CONVERSATIONS],
      ['replay', A, 'mog_menu_change_home'],
    ]);
  });

  it('the returned function stops the wiring', () => {
    const { bus, log, off } = setup();
    off();
    input(bus, `mog-${A}`, A, 'click', { target: MOG_MENU_OPEN, x: 1, y: 2 });
    expect(log).toEqual([]);
  });
});

describe('createAppMogContextMenu (production wiring)', () => {
  const flush = () => new Promise((r) => setTimeout(r, 0));

  beforeEach(() => {
    core.reset();
    menuDouble.reset();
    menuDouble.windows.set(`mog-${A}`, { label: `mog-${A}` });
  });

  it("[ui-4] [#603] builds the four items (Hide/Show labelled by the state reported) and pops them up on the MOG's own window", async () => {
    const bus = new BrainBus();
    createAppMogContextMenu(bus);
    input(bus, `mog-${A}`, A, 'click', { target: MOG_MENU_OPEN, x: 5, y: 6, hidden: true });
    await flush();
    expect(menuDouble.builtItems?.map((i) => [i.id, i.text])).toEqual([
      ['hide', 'Show'],
      ['close', 'Close'],
      ['open_in_bottle', 'Open in Bottle'],
      ['change_home', 'Change home workspace…'],
    ]);
    expect(menuDouble.popupArgs?.at).toMatchObject({ x: 5, y: 6 });
    expect(menuDouble.popupArgs?.window).toEqual({ label: `mog-${A}` });
  });

  it('[ui-4] [#603] picking Close replays a real view:input click, the same shape genie-card-wiring.ts and talk-target.ts already handle', async () => {
    const bus = new BrainBus();
    const seen: unknown[] = [];
    // A second, independent listener on the same bus stands in for genie-card-wiring.ts /
    // talk-target.ts: it must see exactly what a real HTML-menu click used to send.
    onViewInput(bus, (viewInput) => seen.push(viewInput));
    createAppMogContextMenu(bus);
    input(bus, `mog-${A}`, A, 'click', { target: MOG_MENU_OPEN, x: 5, y: 6, hidden: false });
    await flush();
    // The open request itself is one view:input the second listener also sees; picking Close is
    // the second, synthetic one this module replays.
    menuDouble.builtItems?.find((i) => i.id === 'close')?.action();
    expect(seen).toHaveLength(2);
    expect(seen[1]).toEqual({
      from: `mog-${A}`,
      mogId: A,
      kind: 'click',
      payload: { target: 'mog_menu_close' },
    });
  });

  it('[ui-4] [#603] picking Hide emits MOG_MENU_HIDE_EVENT to the one MOG window only, flipping the state it reported', async () => {
    const bus = new BrainBus();
    createAppMogContextMenu(bus);
    input(bus, `mog-${A}`, A, 'click', { target: MOG_MENU_OPEN, x: 0, y: 0, hidden: false });
    await flush();
    menuDouble.builtItems?.find((i) => i.id === 'hide')?.action();
    await flush();
    expect(core.emitTo).toHaveBeenCalledWith(`mog-${A}`, MOG_MENU_HIDE_EVENT, { hidden: true });
  });

  it('[ui-4] [#603] a MOG window that already closed: nothing shows and nothing is sent to it', async () => {
    menuDouble.windows.clear();
    const bus = new BrainBus();
    createAppMogContextMenu(bus);
    input(bus, `mog-${A}`, A, 'click', { target: MOG_MENU_OPEN, x: 0, y: 0, hidden: false });
    await flush();
    expect(menuDouble.builtItems).toBeUndefined();
    expect(core.emitTo).not.toHaveBeenCalled();
  });

  it('[ui-4] [#603] popupMogMenu shows nothing for a MOG window that already closed', async () => {
    menuDouble.windows.clear();
    await expect(popupMogMenu(A, 1, 2, false, () => undefined)).resolves.toBeUndefined();
    expect(menuDouble.builtItems).toBeUndefined();
  });
});

describe('MOG_MENU_HIDE_EVENT', () => {
  it('[ui-4] [#603] is the fixed brain-to-view event name', () => {
    expect(MOG_MENU_HIDE_EVENT).toBe('mog:menu_hide');
  });
});
