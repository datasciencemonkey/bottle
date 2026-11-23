// @vitest-environment jsdom
import { act } from 'react';
import { render, tauri, viewEmits } from '../test-tauri';
import { PERMISSION_EVENT, PERMISSION_TEXT } from '../../shared/permission-view';
import { HUB_RECORDINGS_EVENT } from '../../shared/recording-view';
import { HUB_WORKSPACES_EVENT } from '../../shared/hub-workspaces';
import { Hub } from './Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const A = '00000000-0000-4000-8000-00000000000a';
const AGENT = { crossWorkspaceData: false, crossWorkspaceLocked: false, genieCallBudget: 10 };

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

  it('[ADR 0117] a workspace list with a notice shows Settings; one without does not', async () => {
    const { el } = await render(<Hub />);
    const settings = () => el.querySelector<HTMLElement>('[data-hub-view="settings"]');
    expect(settings()?.hidden).toBe(true);
    const list = { rows: [], home: null, agent: AGENT };
    await tauri.emit(HUB_WORKSPACES_EVENT, list);
    expect(settings()?.hidden).toBe(true);
    await tauri.emit(HUB_WORKSPACES_EVENT, { ...list, notice: { kind: 'info', text: 'Saved' } });
    expect(settings()?.hidden).toBe(false);
    expect(el.querySelector('[data-workspace-notice]')?.textContent).toBe('Saved');
  });

  it('[ADR 0117] only a NEW workspace notice moves the hub, and it focuses the Workspace heading', async () => {
    // Other tests leave their hubs in the document; the heading is found by id.
    document.body.innerHTML = '';
    const { el } = await render(<Hub />);
    const settings = () => el.querySelector<HTMLElement>('[data-hub-view="settings"]');
    const list = { rows: [], home: null, agent: AGENT };
    const note = { kind: 'info', text: 'Saved' };
    await tauri.emit(HUB_WORKSPACES_EVENT, { ...list, notice: note });
    await act(async () => new Promise((r) => requestAnimationFrame(() => r(undefined))));
    expect(settings()?.hidden).toBe(false);
    expect(document.activeElement?.id).toBe('workspace-settings-title');
    // The user goes elsewhere; a republish of the same notice leaves them there.
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    await act(async () =>
      el.querySelector<HTMLButtonElement>('[data-sidebar-item="conversations"]')?.click(),
    );
    expect(settings()?.hidden).toBe(true);
    await tauri.emit(HUB_WORKSPACES_EVENT, { ...list, notice: note });
    expect(settings()?.hidden).toBe(true);
    // A different notice moves the hub again.
    await tauri.emit(HUB_WORKSPACES_EVENT, { ...list, notice: { kind: 'error', text: 'No' } });
    expect(settings()?.hidden).toBe(false);
  });

  it('[ADR 0117] Workspace is the first section of Settings', async () => {
    const { el } = await render(<Hub />);
    const first = el.querySelector('[data-hub-view="settings"]')?.firstElementChild;
    expect(first?.getAttribute('data-view')).toBe('workspace-settings');
  });

  it('[MEM-32a] the Memory area has Forget memory… for the lamp MOG, sent through view_emit', async () => {
    const { el } = await render(<Hub />);
    const btn = () => el.querySelector<HTMLButtonElement>('[data-memory-area] [data-forget-open]');
    // No MOG has reported yet: no button.
    expect(btn()).toBeNull();
    await tauri.emit('mog:view_state', {
      mogId: A,
      mood: 'idle',
      overlays: { attention: false, recording: false, offline: false, localOnly: false },
    });
    expect(btn()?.textContent).toBe('Forget memory…');
    await act(async () => btn()?.click());
    expect(viewEmits()).toContainEqual({
      mogId: A,
      kind: 'click',
      payload: { target: 'forget_open' },
    });
  });

  it('[V1] shows the product name and the Labs label', async () => {
    const { el } = await render(<Hub />);
    expect(el.querySelector('h1')?.textContent).toBe('Bottle');
    expect(el.textContent).toContain('Databricks Labs / experimental');
  });

  it('[ADR 0117] the hub has no MOG list: no live chips, no mogs area, one ready ping', async () => {
    const { el } = await render(<Hub />);
    await tauri.emit('mog:view_state', {
      mogId: A,
      mood: 'idle',
      overlays: { attention: false, recording: false, offline: false, localOnly: false },
    });
    expect(el.querySelector('[data-hub-view="mogs"]')).toBeNull();
    expect(el.querySelector('.hub-live')).toBeNull();
    expect(
      viewEmits().filter(
        (e) => (e as { payload?: { action?: string } }).payload?.action === 'workspaces.ready',
      ),
    ).toHaveLength(1);
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
  const home = (el: HTMLElement) => el.querySelector('section[aria-label="Home"]') as HTMLElement;
  const conv = (el: HTMLElement) =>
    el.querySelector('section[aria-label="Conversations"]') as HTMLElement;
  // `.click()` alone never dispatches `pointerdown` (review of #602, finding 2): a real click is
  // a pointerdown then a click, and HubSidebar's outside-click listener only sees the former.
  const realClick = (b: HTMLElement | null | undefined) => {
    b?.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    b?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  };

  it('[#103] the sidebar is hidden by default; the hub opens on Home', async () => {
    const { el } = await render(<Hub />);
    expect(el.querySelector('[data-sidebar]')).toBeNull();
    expect(home(el).hidden).toBe(false);
    expect(conv(el).hidden).toBe(true);
    expect(el.querySelector('.hub-area-title')?.textContent).toBe('Home');
    // The product header stays a direct child of main (AT-37).
    expect(el.querySelector('main > h1')?.textContent).toBe('Bottle');
    expect(el.querySelector('main > p')?.textContent).toBe('Databricks Labs / experimental');
  });

  it('[home] Home shows the lamp, the wordmark and the punchline, with no buttons', async () => {
    const { el } = await render(<Hub />);
    const h = home(el);
    expect(h.querySelector('img')?.getAttribute('alt')).toContain('peeking out of its lamp');
    expect(h.querySelector('[data-home-wordmark]')?.textContent).toBe('Bottle');
    expect(h.querySelector('[data-home-punchline]')?.textContent).toBe('Ship in a bottle.');
    expect(h.querySelectorAll('button, a, [role="button"]')).toHaveLength(0);
    // Clicking the wordmark does nothing: the area stays Home.
    await act(async () => realClick(h.querySelector<HTMLElement>('[data-home-wordmark]')));
    expect(home(el).hidden).toBe(false);
    expect(el.querySelector('.hub-area-title')?.textContent).toBe('Home');
  });

  it('[home] the setup steps show on Home while one is pending', async () => {
    const { el } = await render(<Hub />);
    expect(el.querySelector('[data-setup-region] [data-notice]')).toBeNull();
    await tauri.emitTo('hub', PERMISSION_EVENT, {
      needs: [{ permission: 'input_monitoring', action: 'allow' }],
    });
    expect(home(el).hidden).toBe(false);
    const pending = el.querySelector('[data-setup-region] [data-permission="input_monitoring"]');
    expect(pending?.textContent).toContain(PERMISSION_TEXT.input_monitoring.need);
    // The landing stays under it; the pending step is not hidden by Home.
    expect(home(el).querySelector('[data-home-wordmark]')).not.toBeNull();
  });

  it('[home] the landing fades in by default', async () => {
    const { el } = await render(<Hub />);
    expect(el.querySelector('.hub-home-stage')?.getAttribute('data-motion')).toBe('fade');
  });

  it('[#103] the toggle opens the sidebar with Home, Conversations, Recordings, Memory, Settings', async () => {
    const { el } = await render(<Hub />);
    await act(async () => toggle(el)?.click());
    expect(el.querySelector('[data-sidebar]')).not.toBeNull();
    expect(items(el).map((b) => b.textContent)).toEqual([
      'Home',
      'Conversations',
      'Recordings',
      'Memory',
      'Settings',
    ]);
    expect(items(el)[0]?.getAttribute('aria-current')).toBe('page');
  });

  it('[home] Cmd+B and Cmd+\\ toggle the sidebar on Home', async () => {
    const { el } = await render(<Hub />);
    const key = (k: string) =>
      act(async () => {
        document.dispatchEvent(
          new KeyboardEvent('keydown', { key: k, metaKey: true, bubbles: true }),
        );
      });
    await key('b');
    expect(el.querySelector('[data-sidebar]')).not.toBeNull();
    await key('\\');
    expect(el.querySelector('[data-sidebar]')).toBeNull();
  });

  it('[home] each purpose open lands on its own area, not Home', async () => {
    const area = (el: HTMLElement) => el.querySelector('.hub-area-title')?.textContent;
    // Open in Bottle (card or MOG menu): Conversations.
    const a = await render(<Hub />);
    await tauri.emitTo('hub', 'hub:conversations.open', {});
    expect(area(a.el)).toBe('Conversations');
    // A workspace notice (a switch or sign-in result): Settings.
    const b = await render(<Hub />);
    await tauri.emit(HUB_WORKSPACES_EVENT, {
      rows: [],
      home: null,
      agent: AGENT,
      notice: { kind: 'error', text: 'Sign-in failed' },
    });
    expect(area(b.el)).toBe('Settings');
    // Choose where to save (the recordings banner): Settings.
    const d = await render(<Hub />);
    await act(async () => d.el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    await act(async () =>
      d.el.querySelector<HTMLButtonElement>('[data-sidebar-item="recordings"]')?.click(),
    );
    await tauri.emitTo('hub', HUB_RECORDINGS_EVENT, {
      rows: [],
      storageProblem: { reason: 'No My Files on this workspace.', canUpload: false },
    });
    await act(async () =>
      d.el.querySelector<HTMLButtonElement>('[data-recording-banner-action]')?.click(),
    );
    expect(area(d.el)).toBe('Settings');
  });

  it('[#103] picking an area switches it and keeps the content column stable', async () => {
    const { el } = await render(<Hub />);
    await act(async () => toggle(el)?.click());
    await act(async () => items(el)[1]?.click());
    expect(el.querySelector('.hub-area-title')?.textContent).toBe('Conversations');
    expect(conv(el).hidden).toBe(false);
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
    await act(async () => items(el)[2]?.click());
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
    await act(async () => items(el)[2]?.click());
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
    expect(el.querySelector('.hub-area-title')?.textContent).toBe('Conversations');
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
    await act(async () => items(el)[2]?.click());
    const radios = () => [...el.querySelectorAll<HTMLElement>('[role="radio"]')];
    await act(async () => radios()[2]?.click());
    expect(radios()[2]?.getAttribute('aria-checked')).toBe('true');
    await act(async () => toggle(el)?.click());
    await act(async () => items(el)[0]?.click());
    expect(el.querySelector('[data-hub-view="settings"]')).not.toBeNull();
    await act(async () => toggle(el)?.click());
    await act(async () => items(el)[2]?.click());
    expect(radios()[2]?.getAttribute('aria-checked')).toBe('true');
  });
});

describe('V1 hub view: call recording (ADR 0103)', () => {
  beforeEach(() => tauri.reset());

  // ui-1 (#97, #103): Recordings is a sidebar item now, like Conversations and Settings, not a button in
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
