// @vitest-environment jsdom
/** The one-time "Choose your home workspace" dialog (ADR 0117, FLAG 5). Every value is fake. */
import { act, useState } from 'react';
import { HUB_SCOPE, HUB_WORKSPACES_EVENT } from '../../shared/hub-workspaces';
import { inputEmits as allEmits, render, tauri } from '../test-tauri';
import { Hub } from './Hub';
import { HomePickDialog } from './HomePickDialog';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const PICK = { suggested: 'ops', candidates: ['sales', 'ops', 'old'] };
const NAMES = new Map([
  ['sales', 'Sales'],
  ['ops', 'Ops'],
]);
/** Only the workspace actions: the hub sends its own ready pings too. */
const inputEmits = () =>
  allEmits().filter((e) =>
    /^workspace\.(?!ready)/.test((e as { payload?: { action?: string } }).payload?.action ?? ''),
  );
const act2 = (fn: () => void) => act(async () => fn());
const dlg = () => document.querySelector<HTMLElement>('[data-home-pick-dialog]');
const radios = () => [...document.querySelectorAll<HTMLElement>('[role="radio"]')];

beforeEach(() => {
  tauri.reset();
  document.body.innerHTML = '';
});

/** The hub keeps the dismissed state; this wrapper does the same. */
function Wrapped({
  homePick,
  names,
  dismissed = vi.fn(),
}: {
  homePick: Parameters<typeof HomePickDialog>[0]['homePick'];
  names?: ReadonlyMap<string, string>;
  dismissed?: () => void;
}) {
  const [open, setOpen] = useState(true);
  return (
    <>
      <button type="button" data-reopen onClick={() => setOpen(true)}>
        reopen
      </button>
      <HomePickDialog
        homePick={homePick}
        names={names}
        open={open}
        onDismiss={() => {
          dismissed();
          setOpen(false);
        }}
      />
    </>
  );
}

describe('HomePickDialog', () => {
  it('shows nothing without homePick', async () => {
    await render(<Wrapped homePick={undefined} />);
    expect(dlg()).toBeNull();
  });

  it('shows the title, the one line, a radio list with the suggestion selected, and Use as home', async () => {
    const { root } = await render(<Wrapped homePick={PICK} names={NAMES} />);
    expect(dlg()?.getAttribute('role')).toBe('dialog');
    expect(dlg()?.querySelector('h2')?.textContent).toBe('Choose your home workspace');
    expect(dlg()?.textContent).toContain(
      'Plain questions go to your home workspace first. You can change it later.',
    );
    expect(dlg()?.getAttribute('aria-labelledby')).toBeTruthy();
    expect(dlg()?.getAttribute('aria-describedby')).toBeTruthy();
    expect(radios().map((r) => r.getAttribute('data-profile'))).toEqual(['sales', 'ops', 'old']);
    expect(radios().map((r) => r.getAttribute('aria-checked'))).toEqual(['false', 'true', 'false']);
    expect(dlg()?.textContent).toContain('Ops');
    expect(dlg()?.querySelector('[data-home-pick-use]')?.textContent).toBe('Use as home');
    act(() => root.unmount());
  });

  it('Use as home sends the suggested profile, or the one the user picked', async () => {
    const { root } = await render(<Wrapped homePick={PICK} names={NAMES} />);
    const use = () => dlg()?.querySelector<HTMLElement>('[data-home-pick-use]');
    await act2(() => use()?.click());
    expect(inputEmits()).toEqual([
      {
        mogId: HUB_SCOPE,
        kind: 'click',
        payload: { action: 'workspace.pick_home', profile: 'ops' },
      },
    ]);
    await act2(() => radios()[0]?.click());
    expect(radios()[0]?.getAttribute('aria-checked')).toBe('true');
    await act2(() => use()?.click());
    expect(inputEmits().at(-1)).toEqual({
      mogId: HUB_SCOPE,
      kind: 'click',
      payload: { action: 'workspace.pick_home', profile: 'sales' },
    });
    act(() => root.unmount());
  });

  it('moves focus into the dialog, traps Tab inside it, and arrows change the choice', async () => {
    const { root } = await render(
      <>
        <button type="button" data-outside>
          outside
        </button>
        <Wrapped homePick={PICK} names={NAMES} />
      </>,
    );
    expect(dlg()?.contains(document.activeElement)).toBe(true);
    // Radix roving focus: the checked radio is the one tab stop, then the button, then wrap.
    expect(document.activeElement).toBe(radios()[1]);
    await act2(() => {
      radios()[1]?.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    });
    expect(dlg()?.contains(document.activeElement)).toBe(true);
    // The tab ring never leaves the dialog: the outside button is not reachable.
    const tabbables = [...(dlg()?.querySelectorAll<HTMLElement>('button, [tabindex="0"]') ?? [])];
    expect(tabbables.length).toBeGreaterThan(0);
    expect(tabbables.every((t) => dlg()?.contains(t))).toBe(true);
    expect(
      document.querySelector('[data-outside]')?.closest('[aria-hidden="true"]'),
    ).not.toBeNull();
    act(() => root.unmount());
  });

  it('Tab from the last button wraps to the first stop, inside the dialog', async () => {
    const { root } = await render(<Wrapped homePick={PICK} names={NAMES} />);
    const tab = () =>
      act2(() => {
        document.activeElement?.dispatchEvent(
          new KeyboardEvent('keydown', {
            key: 'Tab',
            shiftKey: false,
            bubbles: true,
            cancelable: true,
          }),
        );
      });
    const later = dlg()?.querySelector<HTMLElement>('[data-home-pick-later]') as HTMLElement;
    expect(later.textContent).toBe('Not now');
    later.focus();
    await tab();
    expect(document.activeElement).toBe(radios()[1]);
    act(() => root.unmount());
  });

  it('Not now hides the dialog, sends nothing, and the host can reopen it', async () => {
    const dismissed = vi.fn();
    const { root } = await render(<Wrapped homePick={PICK} names={NAMES} dismissed={dismissed} />);
    await act2(() => dlg()?.querySelector<HTMLElement>('[data-home-pick-later]')?.click());
    expect(dlg()).toBeNull();
    expect(dismissed).toHaveBeenCalledTimes(1);
    expect(inputEmits()).toEqual([]);
    await act2(() => document.querySelector<HTMLElement>('[data-reopen]')?.click());
    expect(dlg()).not.toBeNull();
    act(() => root.unmount());
  });

  it('Escape hides it without sending anything; a different pick shows it again', async () => {
    const { el } = await render(<Hub />);
    const agent = { genieCallBudget: 10, copyGenieAnswer: true };
    const rows = ['sales', 'ops'].map((p) => ({
      profile: p,
      name: p === 'ops' ? 'Ops' : 'Sales',
      host: `${p}.x.com`,
      colorVariant: 'coral',
      signIn: 'signed_in',
      available: true,
      isHome: false,
    }));
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, {
      rows,
      home: null,
      agent,
      homePick: { suggested: 'ops', candidates: ['sales', 'ops'] },
    });
    expect(el.querySelector('[data-home-pick-dialog]')).not.toBeNull();
    // Shown over the Home area, with the profile names.
    expect(el.querySelector('[data-home-pick-dialog]')?.textContent).toContain('Sales');
    await act2(() => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(el.querySelector('[data-home-pick-dialog]')).toBeNull();
    expect(inputEmits()).toEqual([]);
    // "Choose…" in the Workspace section opens the same dialog again.
    await act2(() => el.querySelector<HTMLElement>('[data-choose-home]')?.click());
    expect(el.querySelector('[data-home-pick-dialog]')).not.toBeNull();
    await act2(() => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(el.querySelector('[data-home-pick-dialog]')).toBeNull();
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, {
      rows,
      home: null,
      agent,
      homePick: { suggested: 'sales', candidates: ['sales', 'ops'] },
    });
    expect(el.querySelector('[data-home-pick-dialog]')).not.toBeNull();
    // Once the brain clears homePick the dialog goes away.
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, { rows, home: 'sales', agent });
    expect(el.querySelector('[data-home-pick-dialog]')).toBeNull();
  });
});
