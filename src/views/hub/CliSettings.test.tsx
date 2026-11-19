// @vitest-environment jsdom
/** #16: the hub shows the Databricks CLI path and version (W-02a, W-02c, W-02d). Fake values. */
import { act } from 'react';
import { render, tauri, viewEmits } from '../test-tauri';
import {
  CLI_RECOMMENDED_TEXT,
  CLI_SETTINGS_EVENT,
  CLI_SETTINGS_TARGET,
  CLI_VIEW_ID,
  CLI_WARNING_TEXT,
  cliSettingsModel,
} from '../../shared/cli-view';
import { CliSettings } from './CliSettings';
import { Hub } from './Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const PATH = '/usr/local/bin/databricks';

describe('CLI settings (hub)', () => {
  beforeEach(() => tauri.reset());

  it('[W-02a] [W-02c] asks the brain on a click and shows the CLI path and version', async () => {
    const { el } = await render(<CliSettings />);
    expect(viewEmits()).toEqual([]);
    await act(async () => el.querySelector<HTMLButtonElement>('[data-cli-settings]')?.click());
    expect(viewEmits()).toEqual([
      {
        mogId: CLI_VIEW_ID,
        kind: 'click',
        payload: { target: CLI_SETTINGS_TARGET },
      },
    ]);
    await tauri.emitTo(
      'hub',
      CLI_SETTINGS_EVENT,
      cliSettingsModel({ path: PATH, version: '1.25.0' }),
    );
    const text = el.textContent ?? '';
    expect(text).toContain('Databricks CLI');
    expect(text).toContain(PATH);
    expect(text).toContain('1.25.0');
    expect(text).not.toContain(CLI_RECOMMENDED_TEXT);
  });

  it('[W-02c] a CLI below 1.25 shows that 1.25 or later is recommended', async () => {
    const { el } = await render(<CliSettings />);
    await tauri.emitTo(
      'hub',
      CLI_SETTINGS_EVENT,
      cliSettingsModel({ path: PATH, version: '1.20.0' }),
    );
    expect(el.textContent).toContain(CLI_RECOMMENDED_TEXT);
  });

  it('[W-02d] a changed CLI shows the warning from the launch check', async () => {
    const { el } = await render(<CliSettings />);
    await tauri.emitTo(
      'hub',
      CLI_SETTINGS_EVENT,
      cliSettingsModel({ path: PATH, version: '1.26.0' }, ['CLI_VERSION_CHANGED']),
    );
    expect(el.querySelector('[role="alert"]')?.textContent).toContain(
      CLI_WARNING_TEXT.CLI_VERSION_CHANGED,
    );
  });

  it('[W-02a] a missing or unreadable CLI shows a plain status and a bad model is ignored', async () => {
    const { el } = await render(<CliSettings />);
    await tauri.emitTo('hub', CLI_SETTINGS_EVENT, cliSettingsModel({ path: '', version: '' }));
    expect(el.textContent).toContain('Bottle did not find the Databricks CLI.');
    await tauri.emitTo('hub', CLI_SETTINGS_EVENT, { unknown: true });
    expect(el.textContent).toContain('Bottle could not read the Databricks CLI status.');
    await tauri.emitTo('hub', CLI_SETTINGS_EVENT, { path: 7 });
    expect(el.textContent).toContain('Bottle could not read the Databricks CLI status.');
  });

  it('[W-02a] the hub shows the CLI settings', async () => {
    // ui-1 (#97, #103): CLI settings lives in the Settings area, not shown by default.
    const { el } = await render(<Hub />);
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    await act(async () =>
      el.querySelector<HTMLButtonElement>('[data-sidebar-item="settings"]')?.click(),
    );
    expect(el.querySelector('[data-view="cli-settings"]')).not.toBeNull();
  });
});
