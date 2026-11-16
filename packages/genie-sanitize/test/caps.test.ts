import { describe, expect, it } from 'vitest';
import {
  capQuestion, charCount, checkByteLimit, composeMessage, prepareAsk, renderMessage, utf8ByteLength,
  MESSAGE_MAX_BYTES, MESSAGE_MAX_CHARS, QUESTION_MAX_CHARS, type ContextPreamble,
} from '../src/index.ts';

const HEADER = 'Context (from my notes, may be wrong):' as const;
function preamble(texts: string[]): ContextPreamble {
  const items = texts.map((text, i) => ({ id: `item-${i + 1}`, text, source: 'you' as const, ts: i, truncated: false }));
  return { header: HEADER, items, chars: 0 };
}
const words = (n: number, w = 'word') => Array.from({ length: n }, () => w).join(' ');

describe('question cap', () => {
  it('[G-30a] keeps a question of 1,000 chars or less unchanged and gives no notice', () => {
    const q = 'a'.repeat(QUESTION_MAX_CHARS);
    const r = capQuestion(q);
    expect(r).toEqual({ text: q, cut: false });
  });

  it('[G-30a] cuts a longer question at a word edge with a notice', () => {
    const q = words(300); // 1,499 chars
    const r = capQuestion(q);
    expect(r.cut).toBe(true);
    expect(charCount(r.text)).toBeLessThanOrEqual(QUESTION_MAX_CHARS);
    expect(r.text.endsWith('word')).toBe(true);
    expect(q.startsWith(r.text + ' ')).toBe(true);
    expect(r.notice).toMatch(/1,000/);
  });

  it('[G-30a] hard-cuts a single long word at 1,000 code points without splitting a surrogate pair', () => {
    const q = '\u{1F600}'.repeat(1_200);
    const r = capQuestion(q);
    expect(r.cut).toBe(true);
    expect(charCount(r.text)).toBe(QUESTION_MAX_CHARS);
    expect(r.text).toBe('\u{1F600}'.repeat(1_000));
  });

  it('[G-30a] applies the cap after the sanitizer runs', () => {
    const noisy = 'x'.repeat(990) + '\u200B'.repeat(500) + ' ok';
    const r = prepareAsk({ question: noisy });
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.question).toBe('x'.repeat(990) + ' ok');
      expect(r.notices).toEqual([]);
    }
    const long = prepareAsk({ question: '<b>' + words(400) + '</b>' });
    expect(long.ok).toBe(true);
    if (long.ok) {
      expect(charCount(long.question)).toBeLessThanOrEqual(QUESTION_MAX_CHARS);
      expect(long.question).not.toContain('<');
      expect(long.notices.length).toBe(1);
    }
  });

  it('[G-30a] counts characters as Unicode code points', () => {
    expect(charCount('a\u{1F600}b')).toBe(3);
  });
});

describe('whole message caps', () => {
  it('[G-30b] sends the question exactly when there is no preamble', () => {
    const r = composeMessage('How many orders?');
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.text).toBe('How many orders?');
      expect(r.preamble).toBeUndefined();
    }
  });

  it('[G-30b] renders the header, then the items, then the question', () => {
    expect(renderMessage('Q?', preamble(['one', 'two']))).toBe(`${HEADER}\n- one\n- two\n\nQ?`);
  });

  it('[G-30b] keeps a message of 2,000 chars or less unchanged', () => {
    const p = preamble(['short note']);
    const r = composeMessage('Q?', p);
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.droppedItemIds).toEqual([]);
      expect(r.text).toBe(renderMessage('Q?', p));
      expect(r.preamble?.chars).toBe(charCount(r.text) - charCount('\n\nQ?'));
    }
  });

  it('[G-30b] drops preamble items first (last item first) and keeps the full question', () => {
    const question = words(199); // 995 chars
    const items = Array.from({ length: 5 }, (_, i) => `${i}`.repeat(280)); // 5 x 280: over the 2,000 cap
    const r = composeMessage(question, preamble(items));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(charCount(r.text)).toBeLessThanOrEqual(MESSAGE_MAX_CHARS);
      expect(r.text.endsWith('\n\n' + question)).toBe(true);
      expect(r.text.startsWith(HEADER)).toBe(true);
      expect(r.droppedItemIds).toEqual(['item-5', 'item-4']);
      expect(r.preamble?.items.map((i) => i.id)).toEqual(['item-1', 'item-2', 'item-3']);
      expect(r.notices.length).toBe(1);
    }
  });

  it('[G-30b] drops the whole preamble, header included, when no item fits', () => {
    const question = 'q'.repeat(1_990);
    const r = composeMessage(question, preamble(['note']));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.text).toBe(question);
      expect(r.preamble).toBeUndefined();
      expect(r.droppedItemIds).toEqual(['item-1']);
    }
  });

  it('[G-30b] blocks a question that alone is over 2,000 chars', () => {
    const r = composeMessage('q'.repeat(MESSAGE_MAX_CHARS + 1));
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.code).toBe('CLIENT_BUG');
      expect(r.reason).toBe('message_chars');
    }
  });

  it('[G-30c] measures UTF-8 bytes', () => {
    expect(utf8ByteLength('a')).toBe(1);
    expect(utf8ByteLength('é')).toBe(2);
    expect(utf8ByteLength('€')).toBe(3);
    expect(utf8ByteLength('\u{1F600}')).toBe(4);
  });

  it('[G-30c] the byte guard blocks a message over 25,000 UTF-8 bytes', () => {
    const r = checkByteLimit('\u{1F600}'.repeat(6_251)); // 25,004 bytes
    expect(r).toEqual({ ok: false, code: 'CLIENT_BUG', reason: 'message_bytes', userText: expect.any(String) });
    expect(checkByteLimit('\u{1F600}'.repeat(6_250))).toEqual({ ok: true, bytes: MESSAGE_MAX_BYTES });
  });

  it('[G-30c] every composed message reports its byte size within the guard', () => {
    const r = composeMessage('\u{1F600}'.repeat(1_000), preamble(['\u{1F600}'.repeat(160)]));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.bytes).toBe(utf8ByteLength(r.text));
      expect(r.bytes).toBeLessThanOrEqual(MESSAGE_MAX_BYTES);
    }
  });
});
