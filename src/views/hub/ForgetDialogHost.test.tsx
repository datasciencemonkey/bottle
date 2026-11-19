// @vitest-environment jsdom
/**
 * #123: the hub hosts the Forget dialog and opens it from the MOG list (MEM-32a, MEM-32d, ARC-5).
 * Every value here is fake.
 */
import { act } from 'react';
import type { ForgetDialogModel } from '../../shared/forget-view';
import { render, tauri, viewEmits } from '../test-tauri';
import { ForgetDialogHost } from './ForgetDialogHost';
import { MogList } from './MogList';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const A = '00000000-0000-4000-8000-00000000000a';

const model: ForgetDialogModel = {
  mogId: A,
  scope: 'mog',
  phase: 'confirm',
  title: 'Forget this MOG',
  text: 'Delete all of this MOG on this computer? You cannot undo this.',
  memoryReview: {
    label: 'Review Genie One memories',
    text: 'Genie One keeps its own memory.',
    url: 'https://acme.cloud.databricks.com/one?o=1001',
  },
};

beforeEach(() => tauri.reset());

describe('hub Forget dialog (#123)', () => {
  it('[MEM-32a] [ARC-5] shows the model sent to the hub only, and closes on null', async () => {
    const { el, root } = await render(<ForgetDialogHost />);
    expect(tauri.listenOptions.get('hub:forget_dialog')).toEqual([{ target: 'hub' }]);
    expect(el.querySelector('[data-forget-dialog]')).toBeNull();
    await tauri.emitTo('mog-x', 'hub:forget_dialog', model);
    expect(el.querySelector('[data-forget-dialog]')).toBeNull();
    await tauri.emitTo('hub', 'hub:forget_dialog', model);
    expect(el.querySelector('[data-forget-dialog]')?.textContent).toContain('Forget this MOG');
    // MEM-32d: the review link shows with the dialog.
    expect(el.querySelector('button[data-genie-one-memory-review]')).not.toBeNull();
    await tauri.emitTo('hub', 'hub:forget_dialog', { bad: true });
    expect(el.querySelector('[data-forget-dialog]')).not.toBeNull();
    await tauri.emitTo('hub', 'hub:forget_dialog', null);
    expect(el.querySelector('[data-forget-dialog]')).toBeNull();
    act(() => root.unmount());
  });

  it('[MEM-32a] each MOG row has Forget memory, which sends forget_open with the MOG id', async () => {
    const { el, root } = await render(<MogList />);
    await tauri.emitTo('hub', 'hub:mogs', {
      mogs: [
        {
          mogId: A,
          name: 'Sales',
          home: 'sales-ws',
          persona: '',
          running: false,
          colorVariant: 'coral',
        },
      ],
      unreadable: [],
      runningCount: 0,
      maxRunning: 8,
    });
    const btn = el.querySelector<HTMLButtonElement>('button[data-forget-open]');
    expect(btn?.getAttribute('aria-label')).toBe('Forget the memory of Sales');
    await act(async () => btn?.click());
    expect(viewEmits()).toContainEqual({
      mogId: A,
      kind: 'click',
      payload: { target: 'forget_open' },
    });
    act(() => root.unmount());
  });
});
