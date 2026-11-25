// @vitest-environment jsdom
/** The owner's "attach Genie's answer to the clipboard" request: the quiet note. */
import { act } from 'react';
import { render, tauri } from '../test-tauri';
import { COPIED_TEXT, CopyAnswerNote, FAILED_TEXT, NOTE_MS } from './copy-answer-note';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

beforeEach(() => tauri.reset());
afterEach(() => {
  document.body.innerHTML = '';
  vi.useRealTimers();
});

describe('CopyAnswerNote', () => {
  it('shows nothing when there is no outcome yet', async () => {
    const { el } = await render(<CopyAnswerNote outcome={undefined} />);
    expect(el.querySelector('[data-agent-copy-note]')).toBeNull();
  });

  it('shows a check icon and "Copied" on success, role status', async () => {
    const { el } = await render(<CopyAnswerNote outcome="copied" />);
    const note = el.querySelector('[data-agent-copy-note]');
    expect(note?.getAttribute('role')).toBe('status');
    expect(note?.getAttribute('data-tone')).toBe('ok');
    expect(note?.textContent).toBe(COPIED_TEXT);
    expect(note?.querySelector('svg')).not.toBeNull();
  });

  it('shows "Couldn\'t copy" on failure, with no icon and the bad tone, never loud', async () => {
    const { el } = await render(<CopyAnswerNote outcome="failed" />);
    const note = el.querySelector('[data-agent-copy-note]');
    expect(note?.getAttribute('data-tone')).toBe('bad');
    expect(note?.textContent).toBe(FAILED_TEXT);
    expect(note?.querySelector('svg')).toBeNull();
  });

  it('goes away on its own after NOTE_MS, with no further prop change', async () => {
    vi.useFakeTimers();
    const { el } = await render(<CopyAnswerNote outcome="copied" />);
    expect(el.querySelector('[data-agent-copy-note]')).not.toBeNull();
    await act(async () => void vi.advanceTimersByTime(NOTE_MS - 1));
    expect(el.querySelector('[data-agent-copy-note]')).not.toBeNull();
    await act(async () => void vi.advanceTimersByTime(1));
    expect(el.querySelector('[data-agent-copy-note]')).toBeNull();
  });
});
