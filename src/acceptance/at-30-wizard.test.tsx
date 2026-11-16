// @vitest-environment jsdom
/**
 * AT-30 "Global-key defaults and Secure Input" (#230): the wizard part (UX-37a, UX-37b).
 *
 * The R1 parts of AT-30 run in `crates/mog-core/tests/at_30_global_keys.rs`, and the HID numbers
 * in the #171 bench (`cargo test -p mog-core --test latency_bench at_30`). Wizard step 7 (#43) is
 * not built, so these tests stay `it.skip` with the feature issue.
 */
import { render, tauri } from '../views/test-tauri';
import { Hub } from '../views/hub/Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../views/test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../views/test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../views/test-tauri')).windowModule);

describe('AT-30 wizard step 7', () => {
  beforeEach(() => tauri.reset());

  it.skip('[AT-30] [UX-37a] step 7 shows the talk key, the panic key and a "Hold to test" meter TODO(#43)', async () => {
    const { el } = await render(<Hub />);
    const step = el.querySelector('[data-wizard-step="7"]');
    expect(step?.textContent).toContain('Right-Option');
    expect(step?.textContent).toContain('Cmd+Shift+Esc');
    expect(step?.querySelector('[role="meter"]')?.getAttribute('aria-label')).toBe('Hold to test');
  });

  it.skip('[AT-30] [UX-37b] the Accessibility link shows only for the insertion choice TODO(#43)', async () => {
    const { el } = await render(<Hub />);
    const links = () =>
      [...el.querySelectorAll('[data-wizard-step="7"] a')].map((a) => a.textContent);
    expect(links()).toEqual(['Microphone', 'Input Monitoring']);
    (el.querySelector('input[value="insert"]') as HTMLInputElement).click();
    expect(links()).toEqual(['Microphone', 'Input Monitoring', 'Accessibility']);
  });
});
