// @vitest-environment jsdom
import { act } from 'react';
import { HUB_NOTICE_EVENT, HUB_SCOPE, LEGAL_NOTICE } from '../../shared/hub-notice';
import { inputEmits, render, tauri, viewEmits } from '../test-tauri';
import { Hub } from './Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const CHECKBOX = 'I read this notice. I will get consent before I record other people.';

async function hubWithNotice(version: number, accepted = false) {
  const r = await render(<Hub />);
  await tauri.emitTo('hub', HUB_NOTICE_EVENT, { version, accepted });
  return r;
}

const dialog = (el: HTMLElement) => el.querySelector<HTMLElement>('[role="dialog"]');
function must<T>(v: T | null | undefined): T {
  if (v === null || v === undefined) throw new Error('element not found');
  return v;
}
const box = (el: HTMLElement) => must(el.querySelector<HTMLInputElement>('input[type="checkbox"]'));
const cont = (el: HTMLElement) =>
  must([...el.querySelectorAll('button')].find((b) => b.textContent === 'Continue'));

/**
 * The notice emits of the hub: drop the MOG list request `mogs.ready` (#26), the CLI step ask
 * (#161) and the permission notice ask (ADR 0101).
 */
const noticeEmits = () =>
  inputEmits().filter(
    (e) =>
      (e as { payload?: { action?: unknown } }).payload?.action !== 'mogs.ready' &&
      !['cli', 'permissions', 'conversations', 'store'].includes(
        (e as { mogId?: string }).mogId ?? '',
      ),
  );

describe('V1 hub: first-run legal notice (#27)', () => {
  beforeEach(() => {
    tauri.reset();
    document.body.innerHTML = '';
  });

  it('[MB-08a] asks the brain for the notice state on open, only through view_emit', async () => {
    await render(<Hub />);
    expect(tauri.listeners(HUB_NOTICE_EVENT)).toBe(1);
    expect(tauri.listenOptions.get(HUB_NOTICE_EVENT)).toEqual([{ target: 'hub' }]);
    expect(noticeEmits()).toEqual([
      { mogId: HUB_SCOPE, kind: 'click', payload: { action: 'notice.ready' } },
    ]);
  });

  it('[MB-08a] [AT-24] shows "Before you start" with the notice version and the 9.3 content', async () => {
    const { el } = await hubWithNotice(3);
    const d = must(dialog(el));
    expect(d).not.toBeNull();
    expect(d.querySelector('h2')?.textContent).toBe('Before you start');
    expect(d.textContent).toContain('Notice version 3');
    for (const p of LEGAL_NOTICE.paragraphs) expect(d.textContent).toContain(p.text);
    // 9.3 step 2: each content item is in the copy.
    const text = LEGAL_NOTICE.paragraphs.map((p) => p.text).join(' ');
    for (const item of [
      /records/i,
      /microphone stays closed unless you act/i,
      /Databricks workspace/i,
      /until you use Wipe/i,
      /consent of all persons/i,
      /no voiceprints/i,
      /no speaker identification/i,
      /no emotion recognition/i,
    ])
      expect(text).toMatch(item);
    // The MOG list stays hidden until the notice is accepted.
    expect(el.querySelector('ul[aria-label="MOGs"]')).toBeNull();
  });

  it('[MB-08a] keeps Continue off until the user checks the box', async () => {
    const { el } = await hubWithNotice(1);
    expect(box(el).checked).toBe(false);
    expect(box(el).closest('label')?.textContent).toBe(CHECKBOX);
    expect(cont(el).disabled).toBe(true);
    await act(async () => cont(el).click());
    expect(noticeEmits()).toHaveLength(1); // only notice.ready
    await act(async () => box(el).click());
    expect(cont(el).disabled).toBe(false);
    await act(async () => box(el).click());
    expect(cont(el).disabled).toBe(true);
  });

  it('[MB-08a] sends the accepted version on Continue and hides the notice when the brain confirms', async () => {
    const { el } = await hubWithNotice(1);
    await act(async () => box(el).click());
    await act(async () => cont(el).click());
    expect(viewEmits().at(-1)).toEqual({
      mogId: HUB_SCOPE,
      kind: 'click',
      payload: { action: 'notice.accept', noticeVersion: 1 },
    });
    // The view does not decide: the notice stays until the brain reports the record.
    expect(dialog(el)).not.toBeNull();
    await tauri.emitTo('hub', HUB_NOTICE_EVENT, { version: 1, accepted: true });
    expect(dialog(el)).toBeNull();
    expect(el.querySelector('ul[aria-label="MOGs"]')).not.toBeNull();
  });

  it('[MB-08b] [AT-24] shows the notice again, unchecked, when the version increases', async () => {
    const { el } = await hubWithNotice(1);
    await act(async () => box(el).click());
    await tauri.emitTo('hub', HUB_NOTICE_EVENT, { version: 1, accepted: true });
    expect(dialog(el)).toBeNull();
    await tauri.emitTo('hub', HUB_NOTICE_EVENT, { version: 2, accepted: false });
    expect(dialog(el)?.textContent).toContain('Notice version 2');
    expect(box(el).checked).toBe(false);
    expect(cont(el).disabled).toBe(true);
  });

  it('[MB-08a] the notice takes focus and is labelled for a screen reader', async () => {
    const { el } = await hubWithNotice(1);
    const d = must(dialog(el));
    expect(document.activeElement).toBe(d);
    expect(d.getAttribute('aria-modal')).toBe('true');
    const title = document.getElementById(d.getAttribute('aria-labelledby') ?? '');
    expect(title?.textContent).toBe('Before you start');
    const body = document.getElementById(d.getAttribute('aria-describedby') ?? '');
    expect(body?.textContent).toContain(must(LEGAL_NOTICE.paragraphs[0]).text);
    // Native controls: the checkbox and the button are in the Tab order and work by keyboard.
    expect(box(el).tabIndex).toBe(0);
    expect(cont(el).getAttribute('type')).toBe('button');
  });

  it('[MB-08a] marks draft copy as not approved while LEGAL_NOTICE.draft is true', async () => {
    const { el } = await hubWithNotice(1);
    const d = must(dialog(el));
    expect(LEGAL_NOTICE.draft).toBe(true);
    expect(d.textContent).toContain('Draft notice. This text is not yet approved.');
  });

  it('[MB-08a] shows a fixed message and keeps the gate closed when the store cannot open (#541)', async () => {
    const { el } = await hubWithNotice(1);
    await act(async () => box(el).click());
    await act(async () => cont(el).click());
    expect(el.querySelector('[role="alert"]')).toBeNull();
    await tauri.emitTo('hub', HUB_NOTICE_EVENT, { version: 1, accepted: false, storeError: true });
    const d = must(dialog(el));
    const alert = must(d.querySelector('[role="alert"]'));
    expect(alert.textContent).toBe(
      'Bottle could not open its local store. Restart Bottle; if this repeats, see Settings > Diagnostics.',
    );
    expect(LEGAL_NOTICE.storeErrorText).toBe(alert.textContent);
    // The gate stays closed: the MOG list stays hidden, and the user can try again.
    expect(el.querySelector('ul[aria-label="MOGs"]')).toBeNull();
    expect(box(el).checked).toBe(true);
    expect(cont(el).disabled).toBe(false);
    // A later state without the error removes the message.
    await tauri.emitTo('hub', HUB_NOTICE_EVENT, { version: 1, accepted: false });
    expect(must(dialog(el)).querySelector('[role="alert"]')).toBeNull();
  });

  it('[MB-08a] ignores a malformed notice payload', async () => {
    const { el } = await render(<Hub />);
    await tauri.emitTo('hub', HUB_NOTICE_EVENT, { version: '1', accepted: false });
    await tauri.emitTo('hub', HUB_NOTICE_EVENT, null);
    expect(dialog(el)).toBeNull();
  });
});
