// @vitest-environment jsdom
/**
 * #122: the thin view of the "Delete in Genie One" fallback (G-45b, AT-16). Forget (#123) and
 * Wipe (#124) mount it with the notice from the brain. Every value here is fake.
 */
import { act } from 'react';
import type { GenieOneDeleteNoticeModel } from '../shared/genie-one-delete-view';
import { GenieOneDeleteNotice } from './GenieOneDeleteNotice';
import { render, tauri, viewEmits } from './test-tauri';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('./test-tauri')).windowModule);

const MOG = '00000000-0000-4000-8000-000000000001';
const CID = '01f0aaaa-test-not-real-0001';
const URL_1 = `https://acme.cloud.databricks.com/one/chat/threads/${CID}?o=1001`;

const notice: GenieOneDeleteNoticeModel = {
  mogId: MOG,
  label: 'Delete in Genie One',
  text: 'Bottle cannot delete Genie One conversations yet. Use Delete in Genie One to delete each one there.',
  links: [{ conversationId: CID, url: URL_1 }],
  noLink: 1,
  noLinkText: 'Bottle has no link for 1 conversation. Find it in your Genie One history.',
};

/** The element, or a test failure if it is missing. */
function must<T>(v: T | null): T {
  if (v === null) throw new Error('missing element');
  return v;
}

beforeEach(() => tauri.reset());

describe('GenieOneDeleteNotice (G-45b, AT-16)', () => {
  it('[G-45b] [AT-16] shows the disclosure, the Delete in Genie One link and the no-link text', async () => {
    const { el, root } = await render(<GenieOneDeleteNotice notice={notice} />);
    const box = must(el.querySelector('[data-genie-one-delete]'));
    expect(box.getAttribute('role')).toBe('status');
    expect(box.textContent).toContain(notice.text);
    expect(box.textContent).toContain(notice.noLinkText);
    const btn = must(el.querySelector<HTMLButtonElement>('button[data-genie-one-delete-link]'));
    expect(btn.textContent).toBe('Delete in Genie One');
    expect(must(el.querySelector('[data-genie-one-delete-url]')).textContent).toBe(URL_1);
    act(() => root.unmount());
  });

  it('[G-45b] the link click goes to the brain with view_emit, with the conversation ID only', async () => {
    const { el, root } = await render(<GenieOneDeleteNotice notice={notice} />);
    await act(async () => {
      must(el.querySelector<HTMLButtonElement>('button[data-genie-one-delete-link]')).click();
    });
    expect(viewEmits()).toContainEqual({
      mogId: MOG,
      kind: 'click',
      payload: { target: 'genie_one_delete', conversationId: CID },
    });
    act(() => root.unmount());
  });

  it('[G-45b] hides the no-link text when every thread has a link', async () => {
    const { el, root } = await render(
      <GenieOneDeleteNotice
        notice={{
          mogId: MOG,
          label: notice.label,
          text: notice.text,
          links: notice.links,
          noLink: 0,
        }}
      />,
    );
    expect(el.querySelector('[data-genie-one-delete-nolink]')).toBeNull();
    act(() => root.unmount());
  });
});
