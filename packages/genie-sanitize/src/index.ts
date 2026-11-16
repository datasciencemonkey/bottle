/**
 * Question sanitizer, secret guard and message caps for the Genie bridge
 * (research/final-requirements.md: G-47, G-36, G-30a, G-30b, G-30c).
 */
import { capQuestion, composeMessage, type ComposedMessage } from './caps.ts';
import { sanitizeQuestion, stripInvisible } from './sanitize.ts';
import { findSecret } from './secrets.ts';
import type { Blocked, ContextPreamble } from './types.ts';

export * from './caps.ts';
export * from './sanitize.ts';
export * from './secrets.ts';
export type * from './types.ts';

export type PrepareResult =
  | ({ ok: true } & ComposedMessage)
  | Blocked<'secret' | 'empty' | 'message_chars' | 'message_bytes'>;

const SECRET_TEXT =
  'This question looks like it contains a secret (a token or a key), so Bottle did not send it. Remove it and try again.';

/**
 * The full pre-send pipeline: secret check (G-36) on the raw, the de-invisibled and the
 * sanitized text, sanitizer (G-47), question cap (G-30a), message caps (G-30b, G-30c).
 */
export function prepareAsk(input: { question: string; preamble?: ContextPreamble }): PrepareResult {
  const sanitized = sanitizeQuestion(input.question);
  for (const candidate of [input.question, stripInvisible(input.question), sanitized]) {
    if (findSecret(candidate))
      return { ok: false, code: 'SECRET_IN_QUESTION', reason: 'secret', userText: SECRET_TEXT };
  }
  if (sanitized === '') {
    return {
      ok: false,
      code: 'CLIENT_BUG',
      reason: 'empty',
      userText: 'There is no question to send.',
    };
  }
  const capped = capQuestion(sanitized);
  const composed = composeMessage(capped.text, input.preamble);
  if (!composed.ok) return composed;
  return {
    ...composed,
    notices: capped.notice ? [capped.notice, ...composed.notices] : composed.notices,
  };
}
