// @vitest-environment jsdom
import { act } from 'react';
import { render, tauri } from '../test-tauri';
import { Hub } from './Hub';
import {
  MIC_PREFERENCE_LABEL,
  MicPreference,
  ipcMicSettings,
  memoryMicSettings,
} from './MicPreference';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

describe('Mic preference setting (hub)', () => {
  beforeEach(() => tauri.reset());
  afterEach(() => tauri.invoke.mockImplementation(async () => ({})));

  it('[CAP-22] shows the setting with the exact text, off by default', async () => {
    const { el } = await render(<MicPreference settings={memoryMicSettings()} />);
    expect(MIC_PREFERENCE_LABEL).toBe('Prefer the built-in mic when a Bluetooth headset connects');
    const box = el.querySelector<HTMLInputElement>(
      'input[type="checkbox"][name="preferBuiltInMic"]',
    );
    expect(box).not.toBeNull();
    expect(box?.checked).toBe(false);
    expect(el.querySelector('label')?.textContent).toBe(MIC_PREFERENCE_LABEL);
  });

  it('[CAP-22] a click saves the setting and the box shows the saved value', async () => {
    const settings = memoryMicSettings();
    const { el } = await render(<MicPreference settings={settings} />);
    const box = el.querySelector<HTMLInputElement>('input[name="preferBuiltInMic"]');
    await act(async () => box?.click());
    expect(await settings.read()).toBe(true);
    expect(box?.checked).toBe(true);
    await act(async () => box?.click());
    expect(await settings.read()).toBe(false);
    expect(box?.checked).toBe(false);
  });

  it('[CAP-22] [CAP-17a] the hub shows the setting and asks for no permission at launch', async () => {
    const { el } = await render(<Hub />);
    expect(el.textContent).toContain(MIC_PREFERENCE_LABEL);
    const calls = tauri.invoke.mock.calls.map(([cmd]) => cmd);
    expect(calls).not.toContain('permission_request');
    expect(calls).not.toContain('capture_start');
  });

  it('[CAP-22] the hub reads the setting through app_setting_read by default', async () => {
    tauri.invoke.mockImplementation(async (cmd: string) =>
      cmd === 'app_setting_read' ? { key: 'app.capture.preferBuiltInMic', value: true } : {},
    );
    const { el } = await render(<MicPreference />);
    expect(tauri.invoke).toHaveBeenCalledWith('app_setting_read', {
      args: { key: 'app.capture.preferBuiltInMic' },
    });
    const box = el.querySelector<HTMLInputElement>('input[name="preferBuiltInMic"]');
    expect(box?.checked).toBe(true);
  });

  it('[CAP-22] the IPC settings write app.capture.preferBuiltInMic with app_setting_write', async () => {
    let saved = false;
    tauri.invoke.mockImplementation(async (cmd: string, a?: unknown) => {
      if (cmd === 'app_setting_write') {
        saved = (a as { args: { value: boolean } }).args.value;
      }
      return { key: 'app.capture.preferBuiltInMic', value: saved };
    });
    const settings = ipcMicSettings();
    await settings.write(true);
    expect(tauri.invoke).toHaveBeenCalledWith('app_setting_write', {
      args: { key: 'app.capture.preferBuiltInMic', value: true },
    });
    expect(await settings.read()).toBe(true);
  });

  it('[CAP-22] a refused or malformed read shows the setting off', async () => {
    tauri.invoke.mockImplementation(async () => {
      throw { code: 'host_failed' };
    });
    expect(await ipcMicSettings().read()).toBe(false);
    tauri.invoke.mockImplementation(async () => ({}));
    expect(await ipcMicSettings().read()).toBe(false);
  });
});
