// @vitest-environment jsdom
/** #161: the first-run step "Install the Databricks CLI" in the hub (W-17b, W-17c). Fake values. */
import { act } from 'react';
import { render, tauri, viewEmits } from '../test-tauri';
import {
  CLI_INSTALL_EVENT,
  CLI_INSTALL_SOURCE,
  CLI_INSTALL_TARGET,
  CLI_INSTALL_VERSION,
  CLI_SETUP_TARGET,
  CLI_VIEW_ID,
} from '../../shared/cli-view';
import { CliInstallStep } from './CliInstallStep';
import { Hub } from './Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const FOLDER = '~/Library/Application Support/Bottle/bin';
const offer = {
  step: 'offer',
  source: CLI_INSTALL_SOURCE,
  version: CLI_INSTALL_VERSION,
  folder: FOLDER,
};

const button = (el: HTMLElement, text: string) =>
  [...el.querySelectorAll('button')].find((b) => b.textContent === text);

describe('CLI install step (hub)', () => {
  beforeEach(() => tauri.reset());

  it('[W-17c] asks the brain on open and shows the source, version and install folder', async () => {
    const { el } = await render(<CliInstallStep />);
    expect(viewEmits()).toEqual([
      { mogId: CLI_VIEW_ID, kind: 'click', payload: { target: CLI_SETUP_TARGET } },
    ]);
    expect(el.querySelector('[data-view="cli-install"]')).toBeNull();
    await tauri.emitTo('hub', CLI_INSTALL_EVENT, offer);
    const text = el.textContent ?? '';
    expect(text).toContain('Install the Databricks CLI');
    expect(text).toContain(CLI_INSTALL_SOURCE);
    expect(text).toContain(CLI_INSTALL_VERSION);
    expect(text).toContain(FOLDER);
    expect(button(el, 'Not now')).toBeDefined();
  });

  it('[W-17b] only the Install click asks for the install', async () => {
    const { el } = await render(<CliInstallStep />);
    await tauri.emitTo('hub', CLI_INSTALL_EVENT, offer);
    expect(viewEmits()).not.toContainEqual(
      expect.objectContaining({ payload: { target: CLI_INSTALL_TARGET } }),
    );
    await act(async () => button(el, 'Install the Databricks CLI')?.click());
    expect(viewEmits()).toContainEqual({
      mogId: CLI_VIEW_ID,
      kind: 'click',
      payload: { target: CLI_INSTALL_TARGET },
    });
  });

  it('[W-17c] "Not now" leaves the hub in the state "No workspace connected"', async () => {
    const { el } = await render(<CliInstallStep />);
    await tauri.emitTo('hub', CLI_INSTALL_EVENT, offer);
    await act(async () => button(el, 'Not now')?.click());
    expect(el.textContent).toContain('No workspace connected');
    expect(button(el, 'Install the Databricks CLI')).toBeUndefined();
    expect(viewEmits()).not.toContainEqual(
      expect.objectContaining({ payload: { target: CLI_INSTALL_TARGET } }),
    );
  });

  it('[W-17d] [W-17f] shows the failed install and then the path of the installed CLI', async () => {
    const { el } = await render(<CliInstallStep />);
    await tauri.emitTo('hub', CLI_INSTALL_EVENT, { ...offer, step: 'failed' });
    expect(el.querySelector('[role="alert"]')?.textContent).toContain('deleted the download');
    expect(button(el, 'Install the Databricks CLI')).toBeDefined();
    await tauri.emitTo('hub', CLI_INSTALL_EVENT, {
      step: 'installed',
      path: '/Users/someone/Library/Application Support/Bottle/bin/databricks',
      version: '1.26.0',
    });
    expect(el.textContent).toContain(
      '/Users/someone/Library/Application Support/Bottle/bin/databricks',
    );
    expect(button(el, 'Install the Databricks CLI')).toBeUndefined();
  });

  it('[W-17c] the hub shows the install step first, before the other setup steps', async () => {
    const { el } = await render(<Hub />);
    await tauri.emitTo('hub', CLI_INSTALL_EVENT, offer);
    const sections = [...el.querySelectorAll('main > section')];
    expect(sections[0]?.getAttribute('data-view')).toBe('cli-install');
  });
});
