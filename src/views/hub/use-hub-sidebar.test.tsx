// @vitest-environment jsdom
/**
 * ui-1 (#97, #103): the hub sidebar is hidden by default; a user can pin it open, and that choice
 * (only that choice) survives a reload under localStorage key `bottle.hub.sidebar`.
 */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { SIDEBAR_STORAGE_KEY, useHubSidebar, type HubSidebarState } from './use-hub-sidebar';

function mountHook(): { state: () => HubSidebarState; root: Root; el: HTMLDivElement } {
  const el = document.createElement('div');
  document.body.appendChild(el);
  const root = createRoot(el);
  let current: HubSidebarState | undefined;
  function Probe() {
    current = useHubSidebar();
    return null;
  }
  act(() => root.render(<Probe />));
  return {
    state: () => {
      if (!current) throw new Error('hook did not run');
      return current;
    },
    root,
    el,
  };
}

beforeEach(() => localStorage.clear());

describe('useHubSidebar (ui-1)', () => {
  it('starts hidden and unpinned with no persisted choice', () => {
    const { state, root, el } = mountHook();
    expect(state().visible).toBe(false);
    expect(state().pinned).toBe(false);
    expect(state().collapsed).toBe(false);
    act(() => root.unmount());
    el.remove();
  });

  it('toggleVisible shows and hides the sidebar', () => {
    const { state, root, el } = mountHook();
    act(() => state().toggleVisible());
    expect(state().visible).toBe(true);
    act(() => state().toggleVisible());
    expect(state().visible).toBe(false);
    act(() => root.unmount());
    el.remove();
  });

  it('requestClose hides an unpinned sidebar, but not a pinned one', () => {
    const { state, root, el } = mountHook();
    act(() => state().toggleVisible());
    act(() => state().requestClose());
    expect(state().visible).toBe(false);

    act(() => state().toggleVisible());
    act(() => state().togglePinned());
    expect(state().pinned).toBe(true);
    act(() => state().requestClose());
    expect(state().visible).toBe(true);
    act(() => root.unmount());
    el.remove();
  });

  it('persists only the pinned choice, under the documented key', () => {
    const { state, root, el } = mountHook();
    act(() => state().toggleVisible());
    act(() => state().togglePinned());
    expect(localStorage.getItem(SIDEBAR_STORAGE_KEY)).toBe('pinned');
    act(() => state().togglePinned());
    expect(localStorage.getItem(SIDEBAR_STORAGE_KEY)).toBe('hidden');
    act(() => root.unmount());
    el.remove();
  });

  it('starts visible when a previous session pinned it open', () => {
    localStorage.setItem(SIDEBAR_STORAGE_KEY, 'pinned');
    const { state, root, el } = mountHook();
    expect(state().visible).toBe(true);
    expect(state().pinned).toBe(true);
    act(() => root.unmount());
    el.remove();
  });

  it('toggleCollapsed flips the icons-only state, and it is not persisted', () => {
    localStorage.setItem(SIDEBAR_STORAGE_KEY, 'pinned');
    const { state, root, el } = mountHook();
    act(() => state().toggleCollapsed());
    expect(state().collapsed).toBe(true);
    expect(localStorage.getItem(SIDEBAR_STORAGE_KEY)).toBe('pinned');
    act(() => root.unmount());
    el.remove();
  });
});
