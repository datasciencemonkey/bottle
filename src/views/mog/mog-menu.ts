/**
 * #ui-4: the MOG right-click menu. Names and shape checks only, no state.
 *
 * Hide, Close, Open in Bottle and Change home workspace…, in display order. `MogView.tsx` renders
 * them with plain CSS inside the same non-activating panel (MOG-02): the window never becomes key
 * or main regardless of what it shows, so this menu carries no more focus-stealing risk than the
 * native one every other OS menu has, while staying something a component test can click and a
 * screen reader can read (`role="menu"`/`role="menuitem"`).
 */

export type MogMenuAction = 'hide' | 'close' | 'open_in_bottle' | 'change_home';

export interface MogMenuItem {
  action: MogMenuAction;
  label: string;
}

/** The four items, in the order the menu shows them. */
export const MOG_MENU_ITEMS: readonly MogMenuItem[] = [
  { action: 'hide', label: 'Hide' },
  { action: 'close', label: 'Close' },
  { action: 'open_in_bottle', label: 'Open in Bottle' },
  { action: 'change_home', label: 'Change home workspace…' },
];
