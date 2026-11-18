// Adapted from datasciencemonkey/antigravity-bricks-plugin plugins/databricks-genie/scripts/genie_bridge.py @ a31fd2c (MIT).
// From it: an answer names its conversation, and the next call of that conversation goes to the
// workspace where it started (`conversation_in`, the `conversations` pin in `call_tool`).
/**
 * The GL-1 event normaliser of the GenieBridge (#33, ADR 0084): G-21a, G-21b, G-22, G-23a,
 * G-23b.
 *
 * A backend turns its own replies into the normalised `AskEvent` stream here, so UI, mascot and
 * memory code read only AskEvents and never the backend kind (G-21a).
 *
 * - G-21b: each request ends in exactly one terminal event: `answer`, `failed`,
 *   `needs_input(clarify)`, `uncertain`, `abandoned` or `cancel_requested`. `oneTerminal` drops
 *   any event after the first terminal one, and ends a stream without one in `STREAM_LOST`.
 * - G-22: a `completed` answer of less than 400 chars that ends with "?" and has no query item
 *   (from any poll) is `needs_input(clarify)`. The next ask of the same MOG and profile in 5 min
 *   goes to the same conversation (`ClarifyFollowUps`). It is never sent on its own: only the
 *   user's next utterance uses it.
 * - G-23a: `incomplete` with an answer is `answer{incomplete:true}`: the card shows "Partial
 *   answer". G-23b: `incomplete` with no answer is `failed GENIE_INCOMPLETE`.
 *
 * TODO(#87): the `^pub_.+_output$` item filter (G-20b) and the fallback deep link (G-19a).
 * TODO(#149): the soft notice at 4 min and the late re-polls after the give-up.
 * TODO(#29): SP-09 records the live clarify signal; change `isClarify` if it differs.
 */
import type { AskEvent, MogId, ProfileName } from '../../shared/interfaces';
import type { ConversationManager } from './conversation-manager';
import {
  classifyMcpReply,
  classifyTransportCode,
  toFailedEvent,
  userTextFor,
  type Gl5Context,
} from './genie-errors';
import { ipcErrorCode, type PollOutcome, type PollTarget, type PollUpdate } from './poll-actor';

/** G-22: a clarify answer has less than this many chars. */
export const CLARIFY_MAX_CHARS = 400;
/** G-22: the next utterance in this time goes to the clarify conversation. */
export const CLARIFY_FOLLOW_UP_MS = 5 * 60_000;
/** UX-14a: the card bolds 3 key numbers or less. */
export const MAX_KEY_NUMBERS = 3;

type AnswerEvent = Extract<AskEvent, { t: 'answer' }>;
type FailedEvent = Extract<AskEvent, { t: 'failed' }>;
export type ClarifyEvent = Extract<AskEvent, { t: 'needs_input' }> & { kind: 'clarify' };
export type QueryItemRef = AnswerEvent['queryItems'][number];

/** The GL-1 terminal events (G-21b). */
export type TerminalAskEvent =
  | AnswerEvent
  | FailedEvent
  | ClarifyEvent
  | Extract<AskEvent, { t: 'uncertain' | 'abandoned' | 'cancel_requested' }>;

/**
 * True for an event that ends its request. `needs_input` ends it only for `clarify`: an
 * `approval` or a `choose_profile` comes before the ask is sent.
 */
export function isTerminal(e: AskEvent): e is TerminalAskEvent {
  switch (e.t) {
    case 'answer':
    case 'failed':
    case 'uncertain':
    case 'abandoned':
    case 'cancel_requested':
      return true;
    case 'needs_input':
      return e.kind === 'clarify';
    default:
      return false;
  }
}

/** G-22: a short answer that ends with "?" and has no query items. */
export function isClarify(md: string, items: readonly QueryItemRef[]): boolean {
  const text = md.trim();
  return (
    text !== '' && [...text].length < CLARIFY_MAX_CHARS && text.endsWith('?') && items.length === 0
  );
}

const BOLD = /\*\*([^*\n]+?)\*\*/g;

/**
 * UX-14a: the key numbers of an answer: its bold spans that hold a digit, copied verbatim, 3 or
 * less. Nothing is rounded, reformatted or computed. TODO(#86): the blurb bolds them.
 */
export function keyNumbersOf(md: string): { text: string }[] {
  const out: { text: string }[] = [];
  for (const m of md.matchAll(BOLD)) {
    const text = (m[1] ?? '').trim();
    if (/\d/.test(text)) out.push({ text });
    if (out.length === MAX_KEY_NUMBERS) break;
  }
  return out;
}

/** Merge the `query_items` of one poll into `into`, by item ID, in first-seen order. */
export function mergeQueryItems(into: QueryItemRef[], raw: unknown): void {
  if (!Array.isArray(raw)) return;
  for (const q of raw as { item_id?: unknown; sql?: unknown }[]) {
    if (typeof q?.item_id !== 'string' || q.item_id === '') continue;
    const sql = typeof q.sql === 'string' ? q.sql : null;
    const seen = into.find((x) => x.itemId === q.item_id);
    if (!seen) into.push({ itemId: q.item_id, sql });
    else if (sql !== null) seen.sql = sql;
  }
}

function text(v: unknown): string {
  return typeof v === 'string' ? v : '';
}

/** The terminal event of a finished poll loop. */
export function terminalEvent(
  o: PollOutcome,
  items: readonly QueryItemRef[],
  ctx: Gl5Context = {},
): TerminalAskEvent {
  switch (o.kind) {
    case 'gave_up':
      return { t: 'abandoned' };
    // The caller stopped the request. TODO(#151): send genie_cancel_response.
    case 'aborted':
      return { t: 'cancel_requested' };
    case 'error':
      // G-14: an isError reply goes through the GL-5 matrix. Upstream text never shows.
      return toFailedEvent(
        classifyMcpReply(o.reply, ctx) ?? classifyTransportCode('bad_reply', ctx),
      );
    case 'terminal':
      break;
  }
  const md = text(o.result.final_answer);
  if (o.status === 'failed') return genieFailed(ctx);
  if (o.status === 'incomplete' && md.trim() === '')
    return {
      t: 'failed',
      code: 'GENIE_INCOMPLETE',
      userText: userTextFor('GENIE_INCOMPLETE', ctx),
      retry: 'user',
    };
  if (o.status === 'completed' && isClarify(md, items))
    return { t: 'needs_input', kind: 'clarify', prompt: md.trim(), options: [] };
  // A completed poll with no answer and no table has nothing to show.
  if (md.trim() === '' && items.length === 0) return genieFailed(ctx);
  return {
    t: 'answer',
    md,
    incomplete: o.status === 'incomplete',
    keyNumbers: keyNumbersOf(md),
    deepLink: text(o.result.deep_link),
    queryItems: items.map((q) => ({ ...q })),
  };
}

function genieFailed(ctx: Gl5Context): FailedEvent {
  return {
    t: 'failed',
    code: 'GENIE_FAILED',
    userText: userTextFor('GENIE_FAILED', ctx),
    retry: 'user',
  };
}

function progressSteps(v: unknown): string[] | null {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : null;
}

/**
 * The AskEvents of one accepted ask: a `progress` event for each poll that is not terminal, then
 * one terminal event. It polls on the PollActor of `cm` and never sends `genie_ask`.
 */
export async function* pollEvents(
  target: PollTarget,
  cm: ConversationManager,
  ctx: Gl5Context = {},
  signal?: AbortSignal,
): AsyncGenerator<AskEvent> {
  const polls = cm.poll(target, signal);
  const items: QueryItemRef[] = [];
  for (;;) {
    let r: IteratorResult<PollUpdate, PollOutcome>;
    try {
      r = await polls.next();
    } catch (e) {
      // An R8 refusal ends the loop (the IDs stay in the journal). G-14: classify the code;
      // the retry is a user action, never an automatic ask.
      const code = ipcErrorCode(e);
      if (code === undefined) throw e;
      yield toFailedEvent(classifyTransportCode(code, ctx));
      return;
    }
    if (r.done) {
      yield terminalEvent(r.value, items, ctx);
      return;
    }
    // G-20a: merge the query items of every poll.
    mergeQueryItems(items, r.value.result.query_items);
    if (r.value.status === 'in_progress' || r.value.status === 'unknown')
      yield {
        t: 'progress',
        steps: progressSteps(r.value.result.progress_steps),
        elapsedMs: r.value.elapsedMs,
      };
  }
}

/**
 * G-21b: pass the events of one request up to its first terminal event, and drop the rest. A
 * stream that ends with no terminal event ends in `STREAM_LOST`.
 */
export async function* oneTerminal(source: AsyncIterable<AskEvent>): AsyncGenerator<AskEvent> {
  for await (const e of source) {
    yield e;
    if (isTerminal(e)) return;
  }
  yield { t: 'failed', code: 'STREAM_LOST', userText: userTextFor('STREAM_LOST'), retry: 'user' };
}

/** G-22, GL-3: where the answer to a clarify question goes. */
export interface ClarifyFollowUps {
  /** A request of `mogId` ended in `needs_input(clarify)` at `at`. */
  note(mogId: MogId, c: { conversationId: string; profile: ProfileName }, at: number): void;
  /**
   * The conversation for the next utterance of `mogId` at `at`, if it is in 5 min and on the
   * same profile. The next utterance uses up the follow-up either way.
   */
  take(mogId: MogId, profile: ProfileName, at: number): string | undefined;
}

export function createClarifyFollowUps(): ClarifyFollowUps {
  const open = new Map<MogId, { conversationId: string; profile: ProfileName; at: number }>();
  return {
    note: (mogId, c, at) => void open.set(mogId, { ...c, at }),
    take(mogId, profile, at) {
      const c = open.get(mogId);
      open.delete(mogId);
      if (!c || c.profile !== profile) return undefined;
      return at - c.at <= CLARIFY_FOLLOW_UP_MS ? c.conversationId : undefined;
    },
  };
}
