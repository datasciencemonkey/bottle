// @vitest-environment jsdom
/**
 * #162 follow-up (W-07e, review of #602): the one deliberate way to open "Connect a workspace"
 * again from Settings, now that the setup region's step stays hidden until the brain opens it.
 */
import { act } from 'react';
import { CONNECT_EVENT, CONNECT_VIEW_ID } from '../../shared/connect-view';
import { inputEmits, render, tauri } from '../test-tauri';
import { ConnectAnotherWorkspace } from './ConnectAnotherWorkspace';
import { Hub } from './Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const click = (el: Element | null) => act(async () => (el as HTMLElement).click());

describe('Connect another workspace (Settings row)', () => {
  beforeEach(() => tauri.reset());

  it('[W-07e] shows a labelled row and sends nothing on mount', async () => {
    const { el } = await render(<ConnectAnotherWorkspace />);
    expect(el.textContent).toContain('Connect another workspace…');
    expect(inputEmits()).toEqual([]);
  });

  it('[W-07e] the button sends the same connect_open click', async () => {
    const { el } = await render(<ConnectAnotherWorkspace />);
    await click(el.querySelector('[data-connect-another-open]'));
    expect(inputEmits()).toEqual([
      { mogId: CONNECT_VIEW_ID, kind: 'click', payload: { target: 'connect_open' } },
    ]);
  });

  it('[W-07f] [W-07e] the hub has exactly one connect_open entry point: the Settings row', async () => {
    const { el } = await render(<Hub />);
    // The setup region's own ConnectWorkspace step has no model yet, so it renders nothing and
    // offers no button of its own (review of #602, the panel bug).
    expect(el.querySelector('[data-connect-open]')).toBeNull();
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    await act(async () =>
      el.querySelector<HTMLButtonElement>('[data-sidebar-item="settings"]')?.click(),
    );
    expect(el.querySelector('[data-view="connect-another-workspace"]')).not.toBeNull();
    const openers = el.querySelectorAll('[data-connect-another-open]');
    expect(openers).toHaveLength(1);
    await click(openers[0] ?? null);
    expect(inputEmits()).toContainEqual({
      mogId: CONNECT_VIEW_ID,
      kind: 'click',
      payload: { target: 'connect_open' },
    });
  });

  it('[W-07e] the flow opens inline under the row, not in the setup region, with focus on the URL field', async () => {
    const { el } = await render(<Hub />);
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    await act(async () =>
      el.querySelector<HTMLButtonElement>('[data-sidebar-item="settings"]')?.click(),
    );
    await click(el.querySelector('[data-connect-another-open]'));
    // The brain answers the click with the URL step, marked inline.
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'url', firstRun: false, inline: true });
    const row = el.querySelector('[data-view="connect-another-workspace"]');
    expect(row?.querySelector('[data-view="connect-workspace"]')).not.toBeNull();
    expect(el.querySelector('[data-setup-region] [data-view="connect-workspace"]')).toBeNull();
    expect(document.activeElement).toBe(el.querySelector('input[name="workspace-url"]'));
    // Cancel closes it again.
    await click(el.querySelector('[data-connect-close]'));
    expect(inputEmits()).toContainEqual({
      mogId: CONNECT_VIEW_ID,
      kind: 'click',
      payload: { target: 'connect_close' },
    });
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'closed' });
    expect(el.querySelector('[data-view="connect-workspace"]')).toBeNull();
  });

  it('[W-07e] with a first-run flow open, the row scrolls it into view and focuses it', async () => {
    const { el } = await render(<Hub />);
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'url', firstRun: true });
    const panel = el.querySelector<HTMLElement>(
      '[data-setup-region] [data-view="connect-workspace"]',
    );
    expect(panel).not.toBeNull();
    const scroll = vi.fn();
    (panel as HTMLElement).scrollIntoView = scroll;
    expect(document.activeElement).not.toBe(el.querySelector('input[name="workspace-url"]'));
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    await act(async () =>
      el.querySelector<HTMLButtonElement>('[data-sidebar-item="settings"]')?.click(),
    );
    await click(el.querySelector('[data-connect-another-open]'));
    expect(scroll).toHaveBeenCalled();
    expect(document.activeElement).toBe(el.querySelector('input[name="workspace-url"]'));
  });
});
