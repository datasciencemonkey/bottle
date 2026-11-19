// @vitest-environment jsdom
/** ADR 0101 (#552, CAP-36, CAP-37): the hub notice of a missing OS permission. */
import { act } from 'react';
import type { Root } from 'react-dom/client';
import { render as mount, tauri, viewEmits } from '../test-tauri';
import {
  PERMISSION_ALLOW_TARGET,
  PERMISSION_CHECK_TARGET,
  PERMISSION_EVENT,
  PERMISSION_TEXT,
  PERMISSION_VIEW_ID,
} from '../../shared/permission-view';
import { Hub } from './Hub';
import { PermissionNotices } from './PermissionNotices';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const buttons = (el: HTMLElement) => [...el.querySelectorAll('button')];

/** Each test unmounts its views, so a focus listener of an old view sends nothing. */
let roots: Root[] = [];
async function render(node: Parameters<typeof mount>[0]) {
  const r = await mount(node);
  roots.push(r.root);
  return r;
}
afterEach(async () => {
  for (const r of roots) await act(async () => r.unmount());
  roots = [];
});

describe('Permission notices (hub)', () => {
  beforeEach(() => tauri.reset());

  it('[CAP-36] asks the brain on open and shows nothing while nothing is missing', async () => {
    const { el } = await render(<PermissionNotices />);
    expect(viewEmits()).toEqual([
      { mogId: PERMISSION_VIEW_ID, kind: 'click', payload: { target: PERMISSION_CHECK_TARGET } },
    ]);
    await tauri.emitTo('hub', PERMISSION_EVENT, { needs: [] });
    expect(el.querySelector('[data-view="permissions"]')).toBeNull();
  });

  it('[CAP-36] [CAP-37] shows each missing permission with one control and the Settings path', async () => {
    const { el } = await render(<PermissionNotices />);
    await tauri.emitTo('hub', PERMISSION_EVENT, {
      needs: [
        { permission: 'input_monitoring', action: 'allow' },
        { permission: 'accessibility', action: 'settings' },
      ],
    });
    const rows = [...el.querySelectorAll('[data-permission]')];
    expect(rows.map((r) => r.getAttribute('data-permission'))).toEqual([
      'input_monitoring',
      'accessibility',
    ]);
    expect(rows[0]?.textContent).toContain(PERMISSION_TEXT.input_monitoring.need);
    expect(rows[0]?.textContent).toContain(PERMISSION_TEXT.input_monitoring.hub);
    expect(rows[0]?.textContent).toContain('reopen Bottle');
    expect(rows[1]?.textContent).toContain(PERMISSION_TEXT.accessibility.need);
    expect(rows.map((r) => r.querySelectorAll('button').length)).toEqual([1, 1]);
    expect(buttons(el).map((b) => b.textContent)).toEqual(['Allow', 'Open Settings']);
  });

  it('[CAP-36] only a click sends the request, for that permission', async () => {
    const { el } = await render(<PermissionNotices />);
    await tauri.emitTo('hub', PERMISSION_EVENT, {
      needs: [{ permission: 'input_monitoring', action: 'allow' }],
    });
    expect(viewEmits()).toHaveLength(1);
    await act(async () => buttons(el)[0]?.click());
    expect(viewEmits().at(-1)).toEqual({
      mogId: PERMISSION_VIEW_ID,
      kind: 'click',
      payload: { target: PERMISSION_ALLOW_TARGET, permission: 'input_monitoring' },
    });
  });

  it('[CAP-36] the hub asks again when its window gets focus (back from System Settings)', async () => {
    const { root } = await render(<PermissionNotices />);
    const check = {
      mogId: PERMISSION_VIEW_ID,
      kind: 'click',
      payload: { target: PERMISSION_CHECK_TARGET },
    };
    expect(viewEmits()).toEqual([check]);
    await act(async () => void window.dispatchEvent(new Event('focus')));
    expect(viewEmits()).toEqual([check, check]);
    // An unmounted notice no longer listens.
    await act(async () => root.unmount());
    roots = roots.filter((r) => r !== root);
    await act(async () => void window.dispatchEvent(new Event('focus')));
    expect(viewEmits()).toHaveLength(2);
  });

  it('a malformed payload changes nothing', async () => {
    const { el } = await render(<PermissionNotices />);
    await tauri.emitTo('hub', PERMISSION_EVENT, {
      needs: [{ permission: 'camera', action: 'allow' }],
    });
    expect(el.querySelector('[data-view="permissions"]')).toBeNull();
  });

  it('[CAP-36] the hub shows the notice', async () => {
    const { el } = await render(<Hub />);
    await tauri.emitTo('hub', PERMISSION_EVENT, {
      needs: [{ permission: 'input_monitoring', action: 'allow' }],
    });
    expect(el.querySelector('[data-view="permissions"]')?.textContent).toContain(
      PERMISSION_TEXT.input_monitoring.need,
    );
  });
});
