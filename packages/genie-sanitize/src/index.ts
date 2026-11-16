/**
 * Question sanitizer, secret guard and message caps for the Genie bridge
 * (research/final-requirements.md: G-47, G-36, G-30a, G-30b, G-30c).
 */

/** The preamble shape of section 4.7 (built elsewhere, by the ContextAssembler). */
export interface ContextPreamble {
  header: 'Context (from my notes, may be wrong):';
  items: { id: string; text: string; source: 'you' | 'confirmed'; ts: number; truncated: boolean }[];
  chars: number;
}

/** The subset of section 4.7 `BackendErrorCode` that this package can return. */
export type SanitizeErrorCode = 'SECRET_IN_QUESTION' | 'CLIENT_BUG';

export const QUESTION_MAX_CHARS = 1_000;
export const MESSAGE_MAX_CHARS = 2_000;
export const MESSAGE_MAX_BYTES = 25_000;

export interface CappedQuestion { text: string; cut: boolean; notice?: string }

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
  | ({ ok: true } & ComposedMessage)
  | { ok: false; code: SanitizeErrorCode; reason: 'message_chars' | 'message_bytes'; userText: string };

export type PrepareResult =
  | ({ ok: true } & ComposedMessage)
  | { ok: false; code: SanitizeErrorCode; reason: 'secret' | 'empty' | 'message_chars' | 'message_bytes'; userText: string };

export function sanitizeQuestion(_raw: string): string {
  throw new Error('not implemented');
}

export function findSecret(_text: string): string | undefined {
  throw new Error('not implemented');
}

export function charCount(_text: string): number {
  throw new Error('not implemented');
}

export function utf8ByteLength(_text: string): number {
  throw new Error('not implemented');
}

export function capQuestion(_text: string): CappedQuestion {
  throw new Error('not implemented');
}

export function renderMessage(_question: string, _preamble?: ContextPreamble): string {
  throw new Error('not implemented');
}

export function composeMessage(_question: string, _preamble?: ContextPreamble): ComposeResult {
  throw new Error('not implemented');
}

export function prepareAsk(_input: { question: string; preamble?: ContextPreamble }): PrepareResult {
  throw new Error('not implemented');
}

export type ByteCheck =
  | { ok: true; bytes: number }
  | { ok: false; code: SanitizeErrorCode; reason: 'message_bytes'; userText: string };

export function checkByteLimit(_text: string): ByteCheck {
  throw new Error('not implemented');
}
