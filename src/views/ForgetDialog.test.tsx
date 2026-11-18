// @vitest-environment jsdom
/**
 * #123: the thin Forget dialog (MEM-32a, MEM-32c, MEM-32d, G-45b, AT-16). The brain builds the
 * model (`forget-flow.ts`). Every value here is fake.
 */
import { act } from 'react';
import type { ForgetDialogModel } from '../shared/forget-view';
import { ForgetDialog } from './ForgetDialog';
import { render, tauri, viewEmits } from './test-tauri';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('./test-tauri')).windowModule);

const MOG = '00000000-0000-4000-8000-000000000001';
const CID = '01f0aaaa-test-not-real-0001';
const REVIEW = 'https://acme.cloud.databricks.com/one?o=1001';

const memoryReview = {
  label: 'Review Genie One memories',
  text: 'Genie One keeps its own memory. Bottle cannot delete it.',
  url: REVIEW,
};

const confirm: ForgetDialogModel = {
  mogId: MOG,
  scope: 'session',
  phase: 'confirm',
  title: 'Forget this session',
  text: 'Bottle deletes this session on this computer. You cannot undo this.',
  memoryReview,
};

const done: ForgetDialogModel = {
  ...confirm,
  phase: 'done',
  result: {
    local: 4,
    deletedConversations: 0,
    notice: {
      mogId: MOG,
      label: 'Delete in Genie One',
      text: 'Bottle cannot delete Genie One conversations yet.',
      links: [
        {
          conversationId: CID,
          url: `https://acme.cloud.databricks.com/one/chat/threads/${CID}?o=1001`,
        },
      ],
      noLink: 0,
    },
    otherText: 'Bottle could not delete 1 Genie conversation. Delete it in Genie.',
  },
};

function must<T>(v: T | null): T {
  if (v === null) throw new Error('missing element');
  return v;
}

beforeEach(() => tauri.reset());

describe('ForgetDialog (#123)', () => {
  it('[MEM-32a] the confirm step shows the text and sends forget_confirm with no content', async () => {
    const { el, root } = await render(<ForgetDialog model={confirm} />);
    const box = must(el.querySelector('[data-forget-dialog]'));
    expect(box.getAttribute('role')).toBe('dialog');
    expect(box.textContent).toContain(confirm.title);
    expect(box.textContent).toContain(confirm.text);
    await act(async () => {
      must(el.querySelector<HTMLButtonElement>('button[data-forget-confirm]')).click();
    });
    expect(viewEmits()).toContainEqual({
      mogId: MOG,
      kind: 'click',
      payload: { target: 'forget_confirm', scope: 'session' },
    });
    act(() => root.unmount());
  });

  it('[MEM-32d] [AT-16] the dialog links to the Genie One memory review', async () => {
    const { el, root } = await render(<ForgetDialog model={confirm} />);
    const btn = must(el.querySelector<HTMLButtonElement>('button[data-genie-one-memory-review]'));
    expect(btn.textContent).toBe('Review Genie One memories');
    expect(must(el.querySelector('[data-genie-one-memory-review-url]')).textContent).toBe(REVIEW);
    expect(el.textContent).toContain('Genie One keeps its own memory');
    await act(async () => {
      btn.click();
    });
    expect(viewEmits()).toContainEqual({
      mogId: MOG,
      kind: 'click',
      payload: { target: 'genie_one_memory_review' },
    });
    act(() => root.unmount());
  });

  it('[MEM-32c] [G-45b] [AT-16] the done step mounts the Delete in Genie One notice', async () => {
    const { el, root } = await render(<ForgetDialog model={done} />);
    expect(el.querySelector('button[data-forget-confirm]')).toBeNull();
    expect(must(el.querySelector('[data-forget-result]')).textContent).toContain('4');
    const link = must(el.querySelector<HTMLButtonElement>('button[data-genie-one-delete-link]'));
    expect(link.textContent).toBe('Delete in Genie One');
    expect(must(el.querySelector('[data-forget-other]')).textContent).toBe(done.result?.otherText);
    // The memory review link stays on the done step (MEM-32d).
    expect(el.querySelector('button[data-genie-one-memory-review]')).not.toBeNull();
    act(() => root.unmount());
  });

  it('[MEM-32a] Cancel (confirm) and Close (done) send forget_cancel', async () => {
    const { el, root } = await render(<ForgetDialog model={confirm} />);
    const btn = must(el.querySelector<HTMLButtonElement>('button[data-forget-cancel]'));
    expect(btn.textContent).toBe('Cancel');
    await act(async () => {
      btn.click();
    });
    expect(viewEmits()).toContainEqual({
      mogId: MOG,
      kind: 'click',
      payload: { target: 'forget_cancel' },
    });
    act(() => root.unmount());
    const d = await render(<ForgetDialog model={done} />);
    expect(must(d.el.querySelector('button[data-forget-cancel]')).textContent).toBe('Close');
    act(() => d.root.unmount());
  });
});
