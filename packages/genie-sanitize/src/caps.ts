/** G-30a, G-30b, G-30c: the question cap, the whole-message cap and the UTF-8 byte guard. */
import type { Blocked, ContextPreamble } from './types.ts';

export const QUESTION_MAX_CHARS = 1_000;
export const MESSAGE_MAX_CHARS = 2_000;
export const MESSAGE_MAX_BYTES = 25_000;

/** Counts characters as Unicode code points. */
export function charCount(text: string): number {
  return Array.from(text).length;
}

/** UTF-8 byte length; a lone surrogate counts 3 bytes, as its U+FFFD replacement does. */
export function utf8ByteLength(text: string): number {
  let n = 0;
  for (const ch of text) {
    const cp = ch.codePointAt(0) ?? 0;
    n += cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
  }
  return n;
}

export interface CappedQuestion {
  text: string;
  cut: boolean;
  notice?: string;
}

/** G-30a: cuts the sanitized question to 1,000 chars at a word edge, with a notice. */
export function capQuestion(text: string): CappedQuestion {
  const cps = Array.from(text);
  if (cps.length <= QUESTION_MAX_CHARS) return { text, cut: false };
  const head = cps.slice(0, QUESTION_MAX_CHARS);
  let end = head.length;
  if (!/\s/.test(cps[QUESTION_MAX_CHARS] ?? '')) {
    const lastSpace = head.findLastIndex((c) => /\s/.test(c));
    if (lastSpace > 0) end = lastSpace;
  }
  const cutText = head.slice(0, end).join('').trimEnd() || head.join('');
  return { text: cutText, cut: true, notice: 'Bottle shortened the question to 1,000 characters.' };
}

/** The exact text that goes to Genie: header, one "- " line per item, a blank line, the question. */
export function renderMessage(question: string, preamble?: ContextPreamble): string {
  if (!preamble || preamble.items.length === 0) return question;
  return renderPreamble(preamble) + '\n\n' + question;
}

function renderPreamble(p: ContextPreamble): string {
  return p.header + p.items.map((i) => '\n- ' + i.text).join('');
}

export type ByteCheck = { ok: true; bytes: number } | Blocked<'message_bytes'>;

/** G-30c: the byte guard blocks a message over 25,000 UTF-8 bytes. */
export function checkByteLimit(text: string): ByteCheck {
  const bytes = utf8ByteLength(text);
  if (bytes <= MESSAGE_MAX_BYTES) return { ok: true, bytes };
  return {
    ok: false,
    code: 'CLIENT_BUG',
    reason: 'message_bytes',
    userText: 'This message is too large to send to Genie.',
  };
}

export interface ComposedMessage {
  text: string;
  question: string;
  preamble?: ContextPreamble;
  droppedItemIds: string[];
  notices: string[];
  chars: number;
  bytes: number;
}

export type ComposeResult =
  ({ ok: true } & ComposedMessage) | Blocked<'message_chars' | 'message_bytes'>;

/**
 * G-30b and G-30c: builds the whole message. Over 2,000 chars, it drops preamble items
 * first (the last item first), then the header. Then the byte guard runs.
 */
export function composeMessage(question: string, preamble?: ContextPreamble): ComposeResult {
  const items = preamble ? [...preamble.items] : [];
  const droppedItemIds: string[] = [];
  const kept = (): ContextPreamble | undefined =>
    preamble && items.length > 0 ? { ...preamble, items } : undefined;
  let text = renderMessage(question, kept());
  while (charCount(text) > MESSAGE_MAX_CHARS) {
    const last = items.pop();
    if (!last) {
      return {
        ok: false,
        code: 'CLIENT_BUG',
        reason: 'message_chars',
        userText: 'This question is too long to send to Genie.',
      };
    }
    droppedItemIds.push(last.id);
    text = renderMessage(question, kept());
  }
  const bytes = checkByteLimit(text);
  if (!bytes.ok) return bytes;
  const p = kept();
  const notices = droppedItemIds.length
    ? [
        `Bottle left out ${droppedItemIds.length} context item(s) to keep the message within 2,000 characters.`,
      ]
    : [];
  return {
    ok: true,
    text,
    question,
    ...(p ? { preamble: { ...p, chars: charCount(renderPreamble(p)) } } : {}),
    droppedItemIds,
    notices,
    chars: charCount(text),
    bytes: bytes.bytes,
  };
}
