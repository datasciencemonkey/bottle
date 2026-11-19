/**
 * #573: show and focus the hub window after a user action (a double-click on a MOG or its card).
 *
 * The hub is built at start and a close only hides it (`src-tauri/src/lib.rs`), so the brain
 * finds it by its label and shows it with the Tauri window API. The brain capability grants only
 * `get-all-windows`, `show`, `unminimize` and `set-focus` for this. A focus activates Bottle,
 * which is right here: the user asked for the hub. No other path calls it.
 */
import { Window } from '@tauri-apps/api/window';

/** The part of a Tauri `Window` that this module uses. */
export interface HubWindowHandle {
  unminimize(): Promise<void>;
  show(): Promise<void>;
  setFocus(): Promise<void>;
}

/** The part of a Tauri `Window` that `isHubFocused` uses. */
export interface HubFocusHandle {
  isFocused(): Promise<boolean>;
}

export type FindWindow = (label: string) => Promise<HubWindowHandle | null>;

const tauriFind: FindWindow = (label) => Window.getByLabel(label);

/** Show, restore and focus the hub. False when it is not there or a step fails. */
export async function showHub(find: FindWindow = tauriFind): Promise<boolean> {
  try {
    const w = await find('hub');
    if (!w) return false;
    await w.unminimize();
    await w.show();
    await w.setFocus();
    return true;
  } catch {
    return false;
  }
}

/**
 * Whether the hub window has OS focus right now (read-only; #145). A ⌥G follow-up into a thread
 * asks this, so a missed `blur` of the view can never send a ⌥G from another app into a thread.
 * False when the hub is not there or the call fails.
 */
export async function isHubFocused(
  find: (label: string) => Promise<HubFocusHandle | null> = (label) => Window.getByLabel(label),
): Promise<boolean> {
  try {
    const w = await find('hub');
    return w ? await w.isFocused() : false;
  } catch {
    return false;
  }
}
