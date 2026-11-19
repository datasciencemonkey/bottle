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
    // A private window or a disabled store (ADR 0106): the choice just does not survive a reload.
  }
}

/** True while the focused element would normally consume Cmd+B or Cmd+\ itself (a
    text field, a textarea, a contenteditable region) — review of #602: the sidebar shortcut
    must not steal those keys away from typing. */
function focusIsInTextInput(): boolean {
  const el = document.activeElement;
  if (!(el instanceof HTMLElement)) return false;
  if (el.isContentEditable) return true;
  const tag = el.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT';
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

  // Cmd+\ and Cmd+B toggle the sidebar from anywhere in the hub window (review of #602: metaKey
  // only — Bottle is macOS-first (AGENTS.md), and the previous `metaKey || ctrlKey` let Ctrl+B
  // fire the toggle on a Mac too, stealing the Emacs "move back a character" binding away from
  // every text field. Windows ctrlKey parity, if the hub ever ships there, is a separate PR.
  // Never while a text field, textarea or contenteditable region has focus, so the shortcut
  // never steals a letter the user meant to type).
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!e.metaKey) return;
      if (e.key !== '\\' && e.key.toLowerCase() !== 'b') return;
      if (focusIsInTextInput()) return;
      e.preventDefault();
      toggleVisible();
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
