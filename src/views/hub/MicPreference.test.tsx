// @vitest-environment jsdom
import { act } from 'react';
import { render, tauri } from '../test-tauri';
import { Hub } from './Hub';
import { MIC_PREFERENCE_LABEL, MicPreference, memoryMicSettings } from './MicPreference';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

describe('Mic preference setting (hub)', () => {
  beforeEach(() => tauri.reset());

  // ui-1 (ADR 0104): the setting is now a shadcn/ui Switch (Radix), which renders as an
  // accessible `role="switch"` button with `aria-checked`, not a native `<input
  // type="checkbox">`. The accessible contract (role, aria-checked, the label text and its
  // `for`/`id` association) is equivalent; only the concrete tag name changed.
  const switchEl = (el: HTMLElement) =>
    el.querySelector<HTMLButtonElement>('button[role="switch"]');

  it('[CAP-22] shows the setting with the exact text, off by default', async () => {
    const { el } = await render(<MicPreference settings={memoryMicSettings()} />);
    expect(MIC_PREFERENCE_LABEL).toBe('Prefer the built-in mic when a Bluetooth headset connects');
    const box = switchEl(el);
    expect(box).not.toBeNull();
    expect(box?.getAttribute('aria-checked')).toBe('false');
    expect(el.querySelector('label')?.textContent).toBe(MIC_PREFERENCE_LABEL);
  });

  it('[CAP-22] a click saves the setting and the switch shows the saved value', async () => {
    const settings = memoryMicSettings();
    const { el } = await render(<MicPreference settings={settings} />);
    const box = switchEl(el);
    await act(async () => box?.click());
    expect(await settings.read()).toBe(true);
    expect(switchEl(el)?.getAttribute('aria-checked')).toBe('true');
    await act(async () => box?.click());
    expect(await settings.read()).toBe(false);
    expect(switchEl(el)?.getAttribute('aria-checked')).toBe('false');
  });

  it('[CAP-22] [CAP-17a] the hub shows the setting and asks for no permission at launch', async () => {
    // ui-1 (#97, #103): the mic preference lives in the Settings area, not shown by default.
    const { el } = await render(<Hub />);
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    await act(async () =>
      el.querySelector<HTMLButtonElement>('[data-sidebar-item="settings"]')?.click(),
    );
    expect(el.textContent).toContain(MIC_PREFERENCE_LABEL);
    const calls = tauri.invoke.mock.calls.map(([cmd]) => cmd);
    expect(calls).not.toContain('permission_request');
    expect(calls).not.toContain('capture_start');
  });
});
