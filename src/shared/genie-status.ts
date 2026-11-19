/**
 * #573: the live status of one Ask Genie request, shared by the brain (which picks the text) and
 * the card view (which shows it and runs the timer). Not a contract file: the brain sends these
 * texts in the normal `card.update` blurbs.
 *
 * Genie One does not stream tokens. Each `genie_poll_response` gives `progress_steps`: the raw
 * steps of the Genie run, for example "Running SQL: SELECT ...", "Query result: | a | b |" or
 * "Thinking: ...". Their text can hold SQL, table rows and names, so it never shows (G-17b,
 * UX-16a). Only the prefix of the newest step picks one fixed, calm stage line.
 */

/** The first status line, from the send until Genie reports a stage. */
export const ASKING_GENIE = 'Asking Genie';

/** The fixed stage lines, by the prefix of the newest Genie step. The first match wins. */
const STAGES: readonly (readonly [RegExp, string])[] = [
  [/^\s*(writing|generating|drafting)\s+(the\s+)?(sql|query)\b/i, 'Writing the query'],
  [/^\s*running\s+(sql|the\s+query|a\s+query|query)\b/i, 'Running the query'],
  [/^\s*(query\s+result|results?)\b/i, 'Reading the results'],
  [/^\s*(thinking|reasoning|planning)\b/i, 'Thinking it through'],
  [/^\s*(searching|finding|looking|exploring|reading)\b/i, 'Finding the data'],
];

/**
 * The stage line for the newest Genie step, or `undefined` when no step names a known stage (the
 * caller keeps the line it shows). Never returns upstream text.
 */
export function genieStage(steps: readonly string[] | null | undefined): string | undefined {
  const last = steps?.at(-1);
  if (typeof last !== 'string') return undefined;
  for (const [re, line] of STAGES) if (re.test(last)) return line;
  return undefined;
}

/** The blurb ID of a Genie request (`genie-direct.ts`); the card view keys on it. */
export const GENIE_BLURB_PREFIX = 'genie:';
export const genieBlurbId = (opId: string) => `${GENIE_BLURB_PREFIX}${opId}`;
export const isGenieBlurbId = (id: string | undefined): id is string =>
  typeof id === 'string' && id.startsWith(GENIE_BLURB_PREFIX);
/** The operation ID in a Genie blurb ID (the exchange `requestId` of the history). */
export const opIdOfGenieBlurb = (id: string) =>
  isGenieBlurbId(id) ? id.slice(GENIE_BLURB_PREFIX.length) : undefined;

/** The status timer shows from this time on (design spec 4). */
export const TIMER_AFTER_MS = 3_000;
/** From this time on the timer reads "Working… m:ss" (G-17a). */
export const WORKING_AFTER_MS = 60_000;

/** "m:ss" for `ms`. */
export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

/** The status timer text at `ms` after the send: none, then "0:03", then "Working… 1:04". */
export function timerText(ms: number): string | undefined {
  if (ms < TIMER_AFTER_MS) return undefined;
  return ms < WORKING_AFTER_MS ? clock(ms) : `Working… ${clock(ms)}`;
}

/** Design spec 5: how long an answer card stays when the user does not touch it. */
export const ANSWER_MIN_MS = 6_000;
export const ANSWER_MAX_MS = 20_000;
export const FAILURE_SHOW_MS = 8_000;
/** After the pointer leaves, the card stays at least this long. */
export const AFTER_HOVER_MS = 2_500;

/** T = 4 s + 0.25 s per word of the answer, from 6 s to 20 s. */
export function answerShowMs(md: string): number {
  const words = md.split(/\s+/).filter((w) => /[\p{L}\p{N}]/u.test(w)).length;
  return Math.min(ANSWER_MAX_MS, Math.max(ANSWER_MIN_MS, 4_000 + 250 * words));
}

/**
 * The `view_emit` `click` payloads of the Genie card flow (no new `view_emit` kind). A view sends
 * `{ target: 'open_conversations' }` on a double-click of the MOG or the card, and the card sends
 * `{ target: 'card_done' }` when its answer faded by itself.
 */
export const OPEN_CONVERSATIONS = 'open_conversations';
export const CARD_DONE = 'card_done';

/** The card window event for the pointer on the MOG (the card holds its timer meanwhile). */
export const CARD_MOG_HOVER_EVENT = 'card:mog_hover';
/** The card window event that fades the card at once (a double-click on the MOG). */
export const CARD_DISMISS_EVENT = 'card:dismiss';
