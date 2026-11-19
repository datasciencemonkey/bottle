/**
 * The hub sidebar's state (ui-1, #97, #103; course-corrected to hidden-by-default). The sidebar
 * is an overlay, hidden until the user asks for it: the header's toggle button, or Cmd+\ / Cmd+B.
 * Esc or a click outside it hides it again, unless the user pinned it open (a toggle inside the
 * sidebar). The pinned choice is the only part that survives a reload, under the localStorage key
 * `bottle.hub.sidebar` ("pinned" or "hidden", the default). Pinned or not, the sidebar can still
 * collapse to icons-only with tooltips; that choice is session-only (not persisted).
 */
import { useCallback, useEffect, useState } from 'react';

export const SIDEBAR_STORAGE_KEY = 'bottle.hub.sidebar';

function readPinned(storage: Pick<Storage, 'getItem'> = window.localStorage): boolean {
  try {
    return storage.getItem(SIDEBAR_STORAGE_KEY) === 'pinned';
  } catch {
    return false;
  }
}

function writePinned(
  pinned: boolean,
  storage: Pick<Storage, 'setItem'> = window.localStorage,
): void {
  try {
    storage.setItem(SIDEBAR_STORAGE_KEY, pinned ? 'pinned' : 'hidden');
  } catch {
    // A private window or a disabled store (ADR 0104): the choice just does not survive a reload.
  }
}

export interface HubSidebarState {
  /** Whether the sidebar overlay shows right now. */
  visible: boolean;
  /** Whether it is icons-only. */
  collapsed: boolean;
  /** Whether the user pinned it open: Esc and an outside click no longer hide it. */
  pinned: boolean;
  toggleVisible: () => void;
  toggleCollapsed: () => void;
  togglePinned: () => void;
  /** Esc / outside click: a no-op while pinned. */
  requestClose: () => void;
}

export function useHubSidebar(): HubSidebarState {
  const [pinned, setPinned] = useState(readPinned);
  const [visible, setVisible] = useState(readPinned);
  const [collapsed, setCollapsed] = useState(false);

  const toggleVisible = useCallback(() => setVisible((v) => !v), []);
  const toggleCollapsed = useCallback(() => setCollapsed((c) => !c), []);
  const togglePinned = useCallback(() => {
    setPinned((p) => {
      const next = !p;
      writePinned(next);
      if (next) setVisible(true);
      return next;
    });
  }, []);
  const requestClose = useCallback(() => {
    setVisible((v) => (pinned ? v : false));
  }, [pinned]);

  // Cmd+\ and Cmd+B toggle the sidebar from anywhere in the hub window.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key === '\\' || e.key.toLowerCase() === 'b') {
        e.preventDefault();
        toggleVisible();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [toggleVisible]);

  return { visible, collapsed, pinned, toggleVisible, toggleCollapsed, togglePinned, requestClose };
}

/** Exposed for the test: lets it read the raw persisted value without reaching into `window`. */
export function readPersistedSidebar(storage: Pick<Storage, 'getItem'> = window.localStorage) {
  return readPinned(storage) ? 'pinned' : 'hidden';
}
