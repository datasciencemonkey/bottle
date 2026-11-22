// @vitest-environment jsdom
import { act } from 'react';
import { render, tauri, viewEmits } from '../test-tauri';
import { Hub } from './Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const A = '00000000-0000-4000-8000-00000000000a';
const B = '00000000-0000-4000-8000-00000000000b';
const overlays = { attention: false, recording: false, offline: false, localOnly: false };

describe('V1 hub view', () => {
  beforeEach(() => tauri.reset());

  it('[MOG-05b] [MOG-06d] shows an unshown dictation from the hub Channel with Copy, and sends ready after it subscribes', async () => {
    const { el } = await render(<Hub />);
    const sub = tauri.invoke.mock.calls.find(([cmd]) => cmd === 'view_notice_subscribe');
    const channel = (sub?.[1] as { args: { onNotice: { onmessage?: (m: unknown) => void } } }).args
      .onNotice;
    // The hub sends `ready` (no payload) only after the subscribe call, so the brain resends.
    const order = tauri.invoke.mock.calls.map(([cmd, a]) =>
      cmd === 'view_emit' ? `view_emit:${(a as { args: { kind: string } }).args.kind}` : cmd,
    );
    expect(order.indexOf('view_emit:ready')).toBeGreaterThan(
      order.indexOf('view_notice_subscribe'),
    );
    expect(viewEmits()).toContainEqual({ mogId: 'hub', kind: 'ready' });
    await act(async () =>
      channel.onmessage?.({ kind: 'dictation_unshown', mogId: A, text: 'Send the deck today.' }),
    );
    const notice = el.querySelector('[data-notice="dictation_unshown"]');
    expect(notice?.textContent).toContain('Send the deck today.');
    expect(notice?.querySelector('button[data-action="copy"]')?.textContent).toBe('Copy');
    await act(async () =>
      (notice?.querySelector('button[data-action="dismiss"]') as HTMLElement).click(),
    );
    expect(el.querySelector('[data-notice="dictation_unshown"]')).toBeNull();
  });

  it('[#568] a hub list with a notice, or with an Edit ask, shows the MOGs area', async () => {
    const { el } = await render(<Hub />);
    const mogsArea = () => el.querySelector<HTMLElement>('[data-hub-view="mogs"]');
    expect(mogsArea()?.hidden).toBe(true);
    const list = { mogs: [], unreadable: [], runningCount: 0, maxRunning: 3 };
    // A list with neither a notice nor an Edit ask does not move the hub.
    await tauri.emit('hub:mogs', list);
    expect(mogsArea()?.hidden).toBe(true);
    await tauri.emit('hub:mogs', {
      ...list,
      notice: { kind: 'info', text: 'Create a MOG to start' },
    });
    expect(mogsArea()?.hidden).toBe(false);
  });

  it('[GL-5] an Edit ask shows the MOGs area', async () => {
    const { el } = await render(<Hub />);
    const list = { mogs: [], unreadable: [], runningCount: 0, maxRunning: 3 };
    await tauri.emit('hub:mogs', { ...list, openEdit: { mogId: A, token: 1 } });
    expect(el.querySelector<HTMLElement>('[data-hub-view="mogs"]')?.hidden).toBe(false);
  });

  it('[V1] shows the product name and the Labs label', async () => {
    const { el } = await render(<Hub />);
    expect(el.querySelector('h1')?.textContent).toBe('Bottle');
    expect(el.textContent).toContain('Databricks Labs / experimental');
  });

  it('[V1] subscribes to mog:view_state and lists each MOG with its mood', async () => {
    // ui-1 (#97, #103): the live MOG chips moved into the "mogs" area, not shown by default.
    const { el } = await render(<Hub />);
    expect(tauri.listeners('mog:view_state')).toBe(1);
    await tauri.emit('mog:view_state', { mogId: A, mood: 'idle', overlays });
    await tauri.emit('mog:view_state', { mogId: B, mood: 'processing', overlays });
    await tauri.emit('mog:view_state', { mogId: A, mood: 'sleeping', overlays });
    await tauri.emit('mog:view_state', { mogId: 7 });
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    await act(async () =>
      el.querySelector<HTMLButtonElement>('[data-sidebar-item="mogs"]')?.click(),
    );
    const rows = [...el.querySelectorAll('li[data-mog]')].map((li) => [
      li.getAttribute('data-mog'),
      li.getAttribute('data-mood'),
    ]);
    expect(rows).toEqual([
      [A, 'sleeping'],
      [B, 'processing'],
    ]);
  });

  it('[V1] [ARC-2] sends a MOG selection only through view_emit', async () => {
    const { el } = await render(<Hub />);
    await tauri.emit('mog:view_state', { mogId: A, mood: 'idle', overlays });
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    await act(async () =>
      el.querySelector<HTMLButtonElement>('[data-sidebar-item="mogs"]')?.click(),
    );
    await act(async () => (el.querySelector(`li[data-mog="${A}"] button`) as HTMLElement).click());
    // The hub notice (#27, MB-08a) and the MOG list (#26) ask the brain with hub-scope emits
    // (notice.ready, mogs.ready), the CLI step asks with id `cli` (#161), the permission notice
    // with id `permissions` (ADR 0101), the connect panel with id `connect` (`ready`) and Conversations with id `conversations` (#103); the
    // sidebar area switches are local state (no emit); the MOG selection is the rest.
    expect(viewEmits()).toContainEqual({
      mogId: 'hub',
      kind: 'click',
      payload: { action: 'notice.ready' },
    });
    expect(
      viewEmits().filter(
        (e) =>
          !['hub', 'cli', 'permissions', 'conversations', 'recording', 'store', 'connect'].includes(
            (e as { mogId: string }).mogId,
          ),
      ),
    ).toEqual([{ mogId: A, kind: 'click' }]);
  });
});

describe('hub sidebar navigation and Conversations (ui-1, #97, #103)', () => {
  beforeEach(() => {
    tauri.reset();
    // The pin is the only sidebar choice that persists (use-hub-sidebar.ts); start each test
    // from the documented default so one test's pin cannot leak into the next.
    localStorage.clear();
  });

  const toggle = (el: HTMLElement) => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]');
  const items = (el: HTMLElement) => [...el.querySelectorAll<HTMLElement>('[data-sidebar-item]')];
  const conv = (el: HTMLElement) =>
    el.querySelector('section[aria-label="Conversations"]') as HTMLElement;
  // `.click()` alone never dispatches `pointerdown` (review of #602, finding 2): a real click is
  // a pointerdown then a click, and HubSidebar's outside-click listener only sees the former.
  const realClick = (b: HTMLElement | null | undefined) => {
    b?.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    b?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  };

  it('[#103] the sidebar is hidden by default; the hub opens on Conversations', async () => {
    const { el } = await render(<Hub />);
    expect(el.querySelector('[data-sidebar]')).toBeNull();
    expect(conv(el).hidden).toBe(false);
    expect(el.querySelector('.hub-area-title')?.textContent).toBe('Conversations');
    // The product header stays a direct child of main (AT-37).
    expect(el.querySelector('main > h1')?.textContent).toBe('Bottle');
    expect(el.querySelector('main > p')?.textContent).toBe('Databricks Labs / experimental');
  });

  it('[#103] the toggle opens the sidebar with Conversations, MOGs, Recordings and Settings', async () => {
    const { el } = await render(<Hub />);
    await act(async () => toggle(el)?.click());
    expect(el.querySelector('[data-sidebar]')).not.toBeNull();
    expect(items(el).map((b) => b.textContent)).toEqual([
      'Conversations',
      'MOGs',
      'Recordings',
      'Settings',
    ]);
    expect(items(el)[0]?.getAttribute('aria-current')).toBe('page');
  });

  it('[#103] picking an area switches it and keeps the content column stable', async () => {
    const { el } = await render(<Hub />);
    await act(async () => toggle(el)?.click());
    await act(async () => items(el)[1]?.click());
    expect(el.querySelector('.hub-area-title')?.textContent).toBe('MOGs');
    expect(conv(el).hidden).toBe(true);
    expect(el.querySelector('[data-hub-view="mogs"]')).not.toBeNull();
  });

  it('[#103] Esc hides the sidebar; a pin keeps it open across Esc', async () => {
    const { el } = await render(<Hub />);
    await act(async () => toggle(el)?.click());
    expect(el.querySelector('[data-sidebar]')).not.toBeNull();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(el.querySelector('[data-sidebar]')).toBeNull();

    await act(async () => toggle(el)?.click());
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-pin]')?.click());
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(el.querySelector('[data-sidebar]')).not.toBeNull();
  });

  // LOW (review of #602 round 3): the Wipe dialog (Radix Dialog, `role="dialog"`) owns Esc while
  // it is open; the sidebar's own document-level Esc listener used to race it and close the
  // sidebar underneath the still-open dialog.
  it('[LOW review #602] Esc does not close the sidebar while the Wipe dialog is open', async () => {
    const { el } = await render(<Hub />);
    await act(async () => toggle(el)?.click());
    await act(async () => items(el)[3]?.click());
    // Picking an area closes an unpinned overlay (review of #602 "also fix"); reopen it so this
    // test can check whether Esc leaves it open while the dialog is up.
    await act(async () => toggle(el)?.click());
    await act(async () =>
      tauri.emitTo('hub', 'hub:wipe_dialog', {
        scope: 'everything',
        alsoGenie: false,
        phase: 'confirm',
        title: 'Wipe everything',
        text: 'Delete everything on this computer? You cannot undo this.',
      }),
    );
    expect(el.querySelector('[data-wipe-dialog]')).not.toBeNull();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(el.querySelector('[data-sidebar]')).not.toBeNull();
  });

  it('[#103] an open from a double-click shows Conversations', async () => {
    const { el } = await render(<Hub />);
    await act(async () => toggle(el)?.click());
    await act(async () => items(el)[3]?.click());
    expect(conv(el).hidden).toBe(true);
    await tauri.emitTo('hub', 'hub:conversations.open', {});
    expect(conv(el).hidden).toBe(false);
  });

  // review of #602, finding 2: a real click on the toggle while the sidebar is open is a
  // pointerdown (outside the panel, used to close it) then a click (toggleVisible, which
  // reopened the now-closed sidebar) — net effect: it looked like the toggle did nothing.
  it('[review #602] a real click on the toggle closes an open, unpinned sidebar', async () => {
    const { el } = await render(<Hub />);
    await act(async () => toggle(el)?.click());
    expect(el.querySelector('[data-sidebar]')).not.toBeNull();
    await act(async () => realClick(toggle(el)));
    expect(el.querySelector('[data-sidebar]')).toBeNull();
  });

  // review of #602 ("also fix"): a native overlay sidebar closes once it knows where to go.
  it('[review #602] picking an area closes an unpinned sidebar', async () => {
    const { el } = await render(<Hub />);
    await act(async () => toggle(el)?.click());
    await act(async () => items(el)[1]?.click());
    expect(el.querySelector('[data-sidebar]')).toBeNull();
    expect(el.querySelector('.hub-area-title')?.textContent).toBe('MOGs');
  });

  // review of #602 ("also fix"): a pin keeps the sidebar open across an area pick too.
  it('[review #602] picking an area keeps a pinned sidebar open', async () => {
    const { el } = await render(<Hub />);
    await act(async () => toggle(el)?.click());
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-pin]')?.click());
    await act(async () => items(el)[1]?.click());
    expect(el.querySelector('[data-sidebar]')).not.toBeNull();
  });

  // review of #602, finding 3: pinned, the sidebar pushes `.hub-content` over by its own width
  // (`--hub-sidebar-width`, read in hub.css) instead of overlaying it for good.
  // review of #602 round 4, blocker 1: the variable is set on `.hub` (main), not `.hub-body`,
  // so the product header row (outside `.hub-body`, AT-37) can read it too.
  it('[review #602] a pinned sidebar pushes the content column instead of covering it', async () => {
    const { el } = await render(<Hub />);
    const hub = () => el.querySelector<HTMLElement>('main.hub');
    expect(hub()?.style.getPropertyValue('--hub-sidebar-width')).toBe('0px');
    await act(async () => toggle(el)?.click());
    // Unpinned: still an overlay, no reflow.
    expect(hub()?.style.getPropertyValue('--hub-sidebar-width')).toBe('0px');
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-pin]')?.click());
    expect(hub()?.style.getPropertyValue('--hub-sidebar-width')).toBe('220px');
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-collapse]')?.click());
    expect(hub()?.style.getPropertyValue('--hub-sidebar-width')).toBe('56px');
  });

  // review of #602 ("also fix"): switching areas never unmounts one, so the Wipe scope choice
  // (and any open Wipe dialog) survives a sidebar nav click.
  it('[review #602] switching areas keeps Settings mounted (the Wipe choice survives)', async () => {
    const { el } = await render(<Hub />);
    await act(async () => toggle(el)?.click());
    await act(async () => items(el)[3]?.click());
    const radios = () => [...el.querySelectorAll<HTMLElement>('[role="radio"]')];
    await act(async () => radios()[2]?.click());
    expect(radios()[2]?.getAttribute('aria-checked')).toBe('true');
    await act(async () => toggle(el)?.click());
    await act(async () => items(el)[0]?.click());
    expect(el.querySelector('[data-hub-view="settings"]')).not.toBeNull();
    await act(async () => toggle(el)?.click());
    await act(async () => items(el)[3]?.click());
    expect(radios()[2]?.getAttribute('aria-checked')).toBe('true');
  });
});

describe('V1 hub view: call recording (ADR 0103)', () => {
  beforeEach(() => tauri.reset());

  // ui-1 (#97, #103): Recordings is a sidebar item now, like MOGs and Settings, not a button in
  // the old always-visible HubNav; its settings section lives inside the Settings area.
  it('[REC-V15] the hub has a Recordings area and a Call recording settings section', async () => {
    const { el } = await render(<Hub />);
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    const items = [...el.querySelectorAll<HTMLElement>('[data-sidebar-item]')];
    expect(items.map((b) => b.textContent)).toContain('Recordings');
    const settingsItem = items.find((b) => b.textContent === 'Settings') as HTMLElement;
    await act(async () => settingsItem.click());
    expect(el.querySelector('[data-view="recording-settings"]')).not.toBeNull();

    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    const recItem = [...el.querySelectorAll<HTMLElement>('[data-sidebar-item]')].find(
      (b) => b.textContent === 'Recordings',
    ) as HTMLElement;
    await act(async () => recItem.click());
    expect(el.querySelector('section[aria-label="Recordings"]')?.hasAttribute('hidden')).toBe(
      false,
    );
    expect(el.querySelector('[data-hub-view="settings"]')?.hasAttribute('hidden')).toBe(true);
  });
});
