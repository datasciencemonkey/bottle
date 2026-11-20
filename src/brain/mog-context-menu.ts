/**
 * #ui-4: the brain side of the MOG right-click menu (review #603, findings 1 and 4).
 *
 * A MOG view may only listen to brain events and call `view_emit` (ARC-4); only the brain holds
 * `core:menu:*`. So the view only asks `{ target: MOG_MENU_OPEN, x, y, hidden }` (its own window
 * label tells us which MOG, the same as any other `view_emit`), and this module builds and pops
 * up a native Tauri 2 menu on that MOG's own window (`Window.getByLabel`). A native menu always
 * fits the screen, however small the window, and dismisses itself on an outside click or Esc,
 * with nothing for either window to listen for.
 *
 * Picking Close, Open in Bottle or Change home workspace… replays the exact `view:input` click a
 * real HTML-menu click used to send (`bus.dispatch`), so `genie-card-wiring.ts` and
 * `talk-target.ts` need no change at all: they, and the talk target itself, treat this exactly
 * like a MOG click, same as before. Hide stays MOG-view-local state (plain CSS, no brain round
 * trip to show or hide the lamp art): picking it only tells the one MOG window to flip it
 * (`MOG_MENU_HIDE_EVENT`), over `emitTo`.
 */
import { emitTo } from '@tauri-apps/api/event';
import { LogicalPosition } from '@tauri-apps/api/dpi';
import { Menu } from '@tauri-apps/api/menu';
import { Window } from '@tauri-apps/api/window';
import type { MogId } from '../shared/interfaces';
import {
  MOG_MENU_CHANGE_HOME,
  MOG_MENU_CLOSE,
  MOG_MENU_HIDE_EVENT,
  MOG_MENU_OPEN,
  OPEN_CONVERSATIONS,
} from '../shared/genie-status';
import { EVENTS } from '../shared/ipc';
import { mogMenuItems, type MogMenuAction } from './mog-menu-items';
import { onViewInput, viewLabel, type BrainBus } from './bus';

export interface MogContextMenuDeps {
  /**
   * Build and show the menu for `mogId` at the window-relative `(x, y)` the view's `contextmenu`
   * reported, with `hidden` (that MOG's current Hide state) labelling the first item; `onPick`
   * runs with the action once a real native menu would call `id => void`. Default: a real Tauri
   * menu on the MOG's own window (`popupMogMenu`); nothing shows for a MOG window that is gone.
   */
  popup(
    mogId: MogId,
    x: number,
    y: number,
    hidden: boolean,
    onPick: (action: MogMenuAction) => void,
  ): Promise<void>;
  /**
   * Replay the pick the same way a real HTML-menu click used to (`view:input`, kind `click`,
   * `{ target }`): `genie-card-wiring.ts` and `talk-target.ts` act on it unchanged.
   */
  replayClick(mogId: MogId, target: string): void;
  /** Tell the one MOG window to flip Hide (view-local state only it holds). */
  setHidden(mogId: MogId, hidden: boolean): Promise<unknown>;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

/** Wire the MOG right-click menu's open request and its picks to `deps`. */
export function wireMogContextMenu(bus: BrainBus, deps: MogContextMenuDeps): () => void {
  return onViewInput(bus, (input) => {
    if (input.kind !== 'click' || input.from !== `mog-${input.mogId}`) return;
    const p = input.payload;
    if (!isObj(p) || p['target'] !== MOG_MENU_OPEN) return;
    const { x, y } = p;
    if (typeof x !== 'number' || typeof y !== 'number') return;
    const hidden = p['hidden'] === true;
    const { mogId } = input;
    void deps.popup(mogId, x, y, hidden, (action) => {
      switch (action) {
        case 'hide':
          void deps.setHidden(mogId, !hidden);
          return;
        case 'close':
          deps.replayClick(mogId, MOG_MENU_CLOSE);
          return;
        case 'open_in_bottle':
          deps.replayClick(mogId, OPEN_CONVERSATIONS);
          return;
        case 'change_home':
          deps.replayClick(mogId, MOG_MENU_CHANGE_HOME);
          return;
      }
    });
  });
}

/** The production menu: a real Tauri 2 native menu on the MOG's own window. */
export async function popupMogMenu(
  mogId: MogId,
  x: number,
  y: number,
  hidden: boolean,
  onPick: (action: MogMenuAction) => void,
): Promise<void> {
  const target = await Window.getByLabel(viewLabel('mog', mogId));
  // The MOG window closed between the click and the popup (rare): nothing to show.
  if (!target) return;
  const items = mogMenuItems(hidden).map((item) => ({
    id: item.action,
    text: item.label,
    action: () => onPick(item.action),
  }));
  const menu = await Menu.new({ items });
  await menu.popup(new LogicalPosition(x, y), target);
}

/** The production wiring: the real popup, a real bus replay and a real `emitTo`. */
export function createAppMogContextMenu(bus: BrainBus): () => void {
  return wireMogContextMenu(bus, {
    popup: popupMogMenu,
    replayClick: (mogId, target) =>
      bus.dispatch({
        event: EVENTS['view.input'].name,
        payload: { from: `mog-${mogId}`, input: { mogId, kind: 'click', payload: { target } } },
      }),
    setHidden: (mogId, hidden) => emitTo(viewLabel('mog', mogId), MOG_MENU_HIDE_EVENT, { hidden }),
  });
}
