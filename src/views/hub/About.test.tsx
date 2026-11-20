// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { act } from 'react';
import { render, tauri } from '../test-tauri';
import { About, aboutNotices } from './About';
import { Hub } from './Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const root = resolve(import.meta.dirname, '../../..');
const notices = readFileSync(resolve(root, 'THIRD_PARTY_NOTICES.md'), 'utf8');
const requirements = readFileSync(resolve(root, 'research/final-requirements.md'), 'utf8');
const s201 = requirements.slice(requirements.indexOf('### 20.1 Licence notices'));
const parakeet = s201.match(/\| NVIDIA Parakeet[^|]*\|[^|]*\| "([^"]+)"/)?.[1] ?? 'missing';
const squash = (s: string) => s.replace(/\s+/g, ' ');

describe('About screen (in the V1 hub)', () => {
  beforeEach(() => tauri.reset());

  it('[MB-10a] [MB-10b] [AT-37] shows the product name and the Labs label', async () => {
    const { el } = await render(<About />);
    expect(el.querySelector('h2')?.textContent).toBe('About Bottle');
    expect(el.querySelector('[data-label="labs"]')?.textContent).toBe(
      'Databricks Labs / experimental',
    );
    expect(el.textContent).not.toMatch(/mother ?bricks/i);
  });

  it('[SEC-21b] [AT-37] shows every notice of THIRD_PARTY_NOTICES.md as plain text', async () => {
    const { el } = await render(<About />);
    const pre = el.querySelector('pre[data-notices]');
    expect(pre?.textContent).toBe(aboutNotices(notices));
    expect(pre?.children.length).toBe(0);
    const text = squash(pre?.textContent ?? '');
    expect(text).toContain(squash(parakeet));
    for (const name of [
      'antigravity-bricks-plugin',
      'Handy',
      'sherpa-onnx',
      'onnxruntime',
      'Silero VAD v6',
      'Rive runtimes',
      'Tauri',
      'tauri-nspanel',
      'tauri-plugin-global-shortcut',
      'Gemini on Databricks',
      'Databricks CLI',
    ]) {
      expect(text).toContain(`## ${name} (`);
    }
    expect(text).toContain('Copyright (c) 2026 Sathish Gangichetty');
    expect(text).toContain('Copyright (c) 2025 CJ Pais');
    expect(text).toContain('Apache License Version 2.0, January 2004');
  });

  it('[SEC-21b] leaves out the "How to add an entry" maintainer section', () => {
    const shown = aboutNotices(notices);
    expect(notices).toContain('## How to add an entry');
    expect(shown).not.toContain('## How to add an entry');
    expect(shown).toContain('| Component | Licence | How Bottle uses it |');
  });

  it('[SEC-21b] [MB-10b] the hub includes the About screen', async () => {
    // ui-1 (#97, #103): About lives in the Settings area, not shown by default.
    const { el } = await render(<Hub />);
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    await act(async () =>
      el.querySelector<HTMLButtonElement>('[data-sidebar-item="settings"]')?.click(),
    );
    expect(el.querySelector('section[data-view="about"] pre[data-notices]')).not.toBeNull();
  });
});
