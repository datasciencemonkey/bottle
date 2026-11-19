/**
 * #ui-4: the MOG right-click menu. Names and shape checks only, no state.
 *
 * Review #603 (findings 1 and 4): the menu is a native Tauri 2 menu (`@tauri-apps/api/menu`),
 * not HTML, so it never risks taking focus (MOG-02: the window still never becomes key or main)
 * and a native `NSMenu` always fits on screen (unlike the old 168px-wide HTML panel inside a
 * 160px window) and closes itself on an outside click or Esc, with no listener of our own. A MOG
 * view may only listen to events and call `view_emit` (ARC-4), so the brain builds and pops the
 * menu up (`mog-context-menu.ts`); this lives in `src/brain` (not `src/views`), not MogView.tsx,
 * so the view and brain bundles stay split (MOG-08a).
 */

export type MogMenuAction = 'hide' | 'close' | 'open_in_bottle' | 'change_home';

export interface MogMenuItem {
  action: MogMenuAction;
  label: string;
}

/**
 * The four items, in the order the menu shows them. `hidden` is this MOG's current Hide state
 * (review #603, LOW finding): the first item reads "Show" once Hide already ran, so the menu
 * always names the verb that will actually run next.
 */
export function mogMenuItems(hidden: boolean): readonly MogMenuItem[] {
  return [
    { action: 'hide', label: hidden ? 'Show' : 'Hide' },
    { action: 'close', label: 'Close' },
    { action: 'open_in_bottle', label: 'Open in Bottle' },
    { action: 'change_home', label: 'Change home workspace…' },
  ];
}
