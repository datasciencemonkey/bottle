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
  it('[L3] the status line stays mounted, empty, with no outcome yet', async () => {
    const { el } = await render(<CopyAnswerNote outcome={undefined} />);
    const line = el.querySelector('[data-agent-copy-note]');
    expect(line?.getAttribute('role')).toBe('status');
    expect(line?.getAttribute('data-agent-copy-note')).toBe('');
    expect(line?.textContent).toBe('');
    expect(line?.querySelector('[data-tone]')).toBeNull();
  });

  it('shows a check icon and "Copied" on success, inside the status line', async () => {
    const { el } = await render(<CopyAnswerNote outcome="copied" />);
    const line = el.querySelector('[data-agent-copy-note]');
    expect(line?.getAttribute('data-agent-copy-note')).toBe('copied');
    const tone = line?.querySelector('[data-tone]');
    expect(tone?.getAttribute('data-tone')).toBe('ok');
    expect(line?.textContent).toBe(COPIED_TEXT);
    expect(line?.querySelector('svg')).not.toBeNull();
  });

  it('shows "Couldn\'t copy" on failure, with no icon and the bad tone, never loud', async () => {
    const { el } = await render(<CopyAnswerNote outcome="failed" />);
    const line = el.querySelector('[data-agent-copy-note]');
    expect(line?.querySelector('[data-tone]')?.getAttribute('data-tone')).toBe('bad');
    expect(line?.textContent).toBe(FAILED_TEXT);
    expect(line?.querySelector('svg')).toBeNull();
  });

  it('[L3] fades out and clears on its own after NOTE_MS, with no further prop change', async () => {
    vi.useFakeTimers();
    const { el } = await render(<CopyAnswerNote outcome="copied" />);
    const line = () => el.querySelector('[data-agent-copy-note]');
    expect(line()?.getAttribute('data-agent-copy-note')).toBe('copied');
    await act(async () => void vi.advanceTimersByTime(NOTE_MS - 1));
    expect(line()?.getAttribute('data-agent-copy-note')).toBe('copied');
    await act(async () => void vi.advanceTimersByTime(1));
    // The derived state has moved on (nothing to show); the exit animation itself may still be
    // finishing in the DOM, which `AnimatePresence` owns, not this attribute.
    expect(line()?.getAttribute('data-agent-copy-note')).toBe('');
  });
});
