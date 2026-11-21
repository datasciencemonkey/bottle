// @vitest-environment jsdom
import { act } from 'react';
import {
  DICTATION_WORDS_STORAGE_KEY,
  correctDictation,
  readUserWordsText,
} from '../../shared/dictation-vocab';
import { render, tauri } from '../test-tauri';
import { DictationWords } from './DictationWords';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

function type(el: HTMLTextAreaElement, value: string) {
  const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  set?.call(el, value);
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

describe('Dictation words setting (hub)', () => {
  beforeEach(() => {
    tauri.reset();
    localStorage.clear();
  });

  it('shows the group with a labelled box and no words yet', async () => {
    const { el } = await render(<DictationWords />);
    expect(el.querySelector('h2')?.textContent).toBe('Dictation words');
    const area = el.querySelector<HTMLTextAreaElement>('textarea');
    expect(area?.value).toBe('');
    expect(el.querySelector('label')?.textContent).toContain('Fix words the speech engine');
    expect(el.querySelector('[data-dictation-words-status]')?.textContent).toBe(
      'No words of your own yet.',
    );
  });

  it('saves each edit in local settings and the brain reads it', async () => {
    const { el } = await render(<DictationWords />);
    const area = el.querySelector<HTMLTextAreaElement>('textarea');
    await act(async () => type(area as HTMLTextAreaElement, 'sathish → Sathish\nbad line'));
    expect(localStorage.getItem(DICTATION_WORDS_STORAGE_KEY)).toBe('sathish → Sathish\nbad line');
    expect(el.querySelector('[data-dictation-words-status]')?.textContent).toContain(
      '1 line is not used',
    );
    expect(readUserWordsText()).toContain('sathish');
    expect(correctDictation('hi sathish', [{ heard: 'sathish', write: 'Sathish' }])).toBe(
      'hi Sathish',
    );
  });

  it('loads the saved list and counts the words', async () => {
    localStorage.setItem(DICTATION_WORDS_STORAGE_KEY, 'a b → AB\nc → See');
    const { el } = await render(<DictationWords />);
    expect(el.querySelector<HTMLTextAreaElement>('textarea')?.value).toBe('a b → AB\nc → See');
    expect(el.querySelector('[data-dictation-words-status]')?.textContent).toBe('2 words saved.');
  });
});
