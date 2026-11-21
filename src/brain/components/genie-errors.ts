// Adapted from datasciencemonkey/antigravity-bricks-plugin plugins/databricks-genie/scripts/genie_bridge.py @ a31fd2c (MIT).
/**
 * The GL-5 error matrix of the GenieBridge (#32, ADR 0072): G-14, G-15a, G-15b, G-05b.
 *
 * One Genie One MCP failure has up to three sources: the HTTP status, the JSON-RPC `error` (or
 * the JSON body of an HTTP error, which R8 passes as `rpcError`) and the `isError` text of a
 * tool result. The matrix turns them into one `BackendErrorCode`, a fixed user text and the
 * Appendix E actions. The order is: the busy text (any source), then the HTTP status, then the
 * JSON-RPC code and the Databricks `error_code`, then the text, then the default.
 *
 * From the plugin's `forward` and `describe`: the status rules (401, 403 IP access list, 404
 * Genie One off, a non-JSON page) and the rule that Bottle shows only its own texts. Upstream
 * text is read to classify and is then dropped: it never goes into a user text, an event or a
 * log (W-14, SEC-05a). A host goes into a text only if it is a plain host name.
 *
 * `sendGenieAsk` sends one `genie_ask` and never sends it again (G-15b). R8 already resends once
 * after a 401 and after a 404 with a session (ADR 0069). The rate bucket owns the RATE_LIMITED
 * countdown (#138); the conversation queue of ARC-F2 is #148.
 * TODO(#29): the SP-09 live run records the busy envelope and the policy text. Change the rows
 * here if the live texts differ.
 */
import type { AskEvent, BackendErrorCode } from '../../shared/interfaces';
import type { CommandReturns } from '../../shared/ipc';

/** The `mcp_call` reply (section 4.5). */
export type McpCallReply = CommandReturns['mcp_call'];

/** The three error sources of G-14: the `GenieBridge.mapError` input. */
export interface GenieErrorInput {
  httpStatus?: number;
  rpcError?: unknown;
  isErrorText?: string;
}

/** What the mapper may put in a text. Only values that Bottle owns (the profile host). */
export interface Gl5Context {
  host?: string;
  /** The profile name (the workspace the user knows it by). Shown only if it is a plain name. */
  profile?: string;
}

/** The card actions of the Appendix E failure action map. */
export type FailureAction =
  | 'sign_in'
  | 'admin_checklist'
  | 'check_vpn'
  | 'retry'
  | 'resend_without_context'
  | 'wait'
  | 'new_conversation'
  | 'open_in_databricks'
  | 'check_again'
  | 'ask_again'
  | 'check_now'
  | 'edit_question'
  | 'set_up_cli';

type RetryKind = Extract<AskEvent, { t: 'failed' }>['retry'];

export interface Gl5Result {
  code: BackendErrorCode;
  /** A fixed Bottle text. Never upstream text. */
  userText: string;
  /** The GL-5 row (or `transport:<code>` for an R8 code). */
  row: string;
  actions: readonly FailureAction[];
  /** The `retry` of the failed AskEvent. */
  retry: RetryKind;
  /** True only for RATE_LIMITED: the countdown retries without a user action (ARC-F3). */
  autoRetry: boolean;
  /** `retry_after` seconds from the error body, if it is a finite number ≥ 0. */
  retryAfterS?: number;
}

/** The live busy text (D56, G-15a), matched as a case-insensitive substring. */
export const BUSY_MARKER = 'stream closed before response.created';

export function isBusyText(text: string): boolean {
  return text.toLowerCase().includes(BUSY_MARKER);
}

const IP_ACL = /\bip access list|\bip acl\b|\bsource ip\b/i;
const POLICY = /guardrail|blocked by (?:a |the )?(?:workspace )?policy|policy violation/i;
const RATE = /rate limit|too many requests|rate_limit_exceeded/i;
const PROTOCOL_CODES: ReadonlySet<number> = new Set([-32700, -32600, -32601, -32602]);
const RATE_CODES: ReadonlySet<string> = new Set(['REQUEST_LIMIT_EXCEEDED', 'RESOURCE_EXHAUSTED']);

interface Signals {
  status?: number;
  rpcCode?: number;
  errorCode?: string;
  text: string;
  retryAfterS?: number;
}

/** One GL-5 row: an ID, the code it gives and the rule. */
export interface Gl5Row {
  id: string;
  source: 'any' | 'http' | 'json_rpc' | 'text' | 'none';
  code: BackendErrorCode;
  matches: (s: Signals) => boolean;
}

const httpError = (s: Signals) => s.status !== undefined && (s.status < 200 || s.status > 299);

/** The GL-5 matrix, in the order the mapper tries it. The first row that matches wins. */
export const GL5_MATRIX: readonly Gl5Row[] = [
  // ARC-F2: the busy text comes with HTTP 200 (not a 409) in a JSON-RPC error or an isError text.
  { id: 'busy-text', source: 'any', code: 'CONVERSATION_BUSY', matches: (s) => isBusyText(s.text) },
  // R8 has already dropped the token and resent once (W-05d).
  { id: 'http-401', source: 'http', code: 'AUTH_EXPIRED', matches: (s) => s.status === 401 },
  {
    id: 'http-403-ip-acl',
    source: 'http',
    code: 'NETWORK_BLOCKED',
    matches: (s) => s.status === 403 && IP_ACL.test(s.text),
  },
  { id: 'http-403', source: 'http', code: 'NO_GRANT', matches: (s) => s.status === 403 },
  // R8 has already initialised again once for a 404 with a session (G-05a, ARC-F6).
  { id: 'http-404', source: 'http', code: 'GENIE_ONE_OFF', matches: (s) => s.status === 404 },
  // The Genie One REST 409 is "another active response" (connector spec).
  { id: 'http-409', source: 'http', code: 'CONVERSATION_BUSY', matches: (s) => s.status === 409 },
  { id: 'http-429', source: 'http', code: 'RATE_LIMITED', matches: (s) => s.status === 429 },
  // ARC-F4: a gateway timeout after the send is uncertain delivery; the user decides.
  { id: 'http-504', source: 'http', code: 'ASK_UNCERTAIN', matches: (s) => s.status === 504 },
  {
    id: 'http-5xx',
    source: 'http',
    code: 'GENIE_FAILED',
    matches: (s) => s.status !== undefined && s.status >= 500,
  },
  { id: 'http-4xx', source: 'http', code: 'CLIENT_BUG', matches: httpError },
  {
    id: 'rpc-protocol',
    source: 'json_rpc',
    code: 'CLIENT_BUG',
    matches: (s) => s.rpcCode !== undefined && PROTOCOL_CODES.has(s.rpcCode),
  },
  {
    id: 'ec-unauthenticated',
    source: 'json_rpc',
    code: 'AUTH_EXPIRED',
    matches: (s) => s.errorCode === 'UNAUTHENTICATED',
  },
  {
    id: 'ec-permission-denied',
    source: 'json_rpc',
    code: 'NO_GRANT',
    matches: (s) => s.errorCode === 'PERMISSION_DENIED' && !IP_ACL.test(s.text),
  },
  {
    id: 'ec-rate-limit',
    source: 'json_rpc',
    code: 'RATE_LIMITED',
    matches: (s) =>
      s.errorCode !== undefined &&
      (RATE_CODES.has(s.errorCode) || s.errorCode.startsWith('RATE_LIMIT_EXCEEDED')),
  },
  {
    id: 'ec-feature-disabled',
    source: 'json_rpc',
    code: 'GENIE_ONE_OFF',
    matches: (s) => s.errorCode === 'FEATURE_DISABLED',
  },
  {
    id: 'text-ip-acl',
    source: 'text',
    code: 'NETWORK_BLOCKED',
    matches: (s) => IP_ACL.test(s.text),
  },
  { id: 'text-policy', source: 'text', code: 'POLICY_DENIED', matches: (s) => POLICY.test(s.text) },
  {
    id: 'text-rate-limit',
    source: 'text',
    code: 'RATE_LIMITED',
    matches: (s) => RATE.test(s.text),
  },
  {
    id: 'tool-error',
    source: 'text',
    code: 'GENIE_FAILED',
    matches: (s) => s.text !== '' || s.rpcCode !== undefined || s.errorCode !== undefined,
  },
  // mapError was called with no error at all: a Bottle bug.
  { id: 'no-signal', source: 'none', code: 'CLIENT_BUG', matches: () => true },
];

/** The R8 failure codes of ADR 0069 decision 4 (and the IPC rejection codes) per GL-5 code. */
const TRANSPORT_CODES: Readonly<Record<string, BackendErrorCode>> = {
  genie_one_off: 'GENIE_ONE_OFF',
  auth_expired: 'AUTH_EXPIRED',
  rate_limited: 'RATE_LIMITED',
  non_json_reply: 'PROXY_PAGE',
  redirect_refused: 'PROXY_PAGE',
  timeout: 'ASK_UNCERTAIN',
  offline: 'OFFLINE',
  tool_not_allowed: 'TOOL_BLOCKED',
  invalid_tool_args: 'TOOL_BLOCKED',
  invalid_meta: 'TOOL_BLOCKED',
  host_refused: 'TOOL_BLOCKED',
  profile_invalid: 'CLIENT_BUG',
  not_implemented: 'CLIENT_BUG',
  reply_too_large: 'GENIE_FAILED',
  bad_reply: 'GENIE_FAILED',
  protocol_unsupported: 'GENIE_FAILED',
  host_failed: 'GENIE_FAILED',
  // ARC-F1, G-07b (#31, ADR 0073): R8 E_POLL_INFLIGHT, another poll on this conversation is open.
  // The PollActor polls again with the same IDs and never shows it; any other caller (a manual
  // "Check again") waits like a busy conversation and never asks again.
  // It crosses IPC as itself (#495). TODO(#467): the host routes `mcp_call` to R8.
  poll_inflight: 'CONVERSATION_BUSY',
};

const ACTIONS: Readonly<Record<BackendErrorCode, readonly FailureAction[]>> = {
  AUTH_EXPIRED: ['sign_in'],
  NETWORK_BLOCKED: ['admin_checklist', 'check_vpn'],
  NO_GRANT: ['admin_checklist', 'check_vpn'],
  GENIE_ONE_OFF: ['admin_checklist'],
  SPACE_UNAVAILABLE: ['admin_checklist'],
  CONVERSATION_BUSY: ['wait', 'new_conversation'],
  RATE_LIMITED: [],
  PROXY_PAGE: ['retry'],
  OFFLINE: ['retry'],
  ASK_UNCERTAIN: ['check_again', 'ask_again'],
  TOOL_BLOCKED: [],
  POLICY_DENIED: ['resend_without_context'],
  GENIE_FAILED: ['retry', 'open_in_databricks'],
  GENIE_INCOMPLETE: ['ask_again'],
  TIMEOUT_GAVE_UP: ['check_now', 'open_in_databricks'],
  SECRET_IN_QUESTION: ['edit_question'],
  STREAM_LOST: ['check_again'],
  AGENT_FAILED: ['retry'],
  CLIENT_BUG: [],
  UNSUPPORTED_AUTH: ['sign_in'],
  CLI_MISSING: ['set_up_cli'],
};

const RETRY: Readonly<Record<BackendErrorCode, RetryKind>> = {
  AUTH_EXPIRED: 'relogin',
  UNSUPPORTED_AUTH: 'relogin',
  POLICY_DENIED: 'without_preamble',
  CONVERSATION_BUSY: 'user',
  PROXY_PAGE: 'user',
  OFFLINE: 'user',
  ASK_UNCERTAIN: 'user',
  GENIE_FAILED: 'user',
  GENIE_INCOMPLETE: 'user',
  TIMEOUT_GAVE_UP: 'user',
  STREAM_LOST: 'user',
  AGENT_FAILED: 'user',
  SECRET_IN_QUESTION: 'user',
  NETWORK_BLOCKED: 'none',
  NO_GRANT: 'none',
  GENIE_ONE_OFF: 'none',
  SPACE_UNAVAILABLE: 'none',
  RATE_LIMITED: 'none',
  TOOL_BLOCKED: 'none',
  CLIENT_BUG: 'none',
  CLI_MISSING: 'none',
};

/**
 * The most `retry_after` seconds a countdown text shows (the #32 review follow-up). R8 holds a
 * profile for 300 s or less (ADR 0075), so a longer value never waits longer than this.
 */
export const RETRY_AFTER_TEXT_CAP_S = 300;

/** The text of an ask that R8 stopped on RATE_LIMITED after its tries (ADR 0075, ADR 0084). */
export const RATE_LIMITED_FINAL_TEXT = 'Genie is busy. Try again in a minute.';

const PLAIN_HOST = /^[a-z0-9](?:[a-z0-9-]{0,62}\.)*[a-z0-9-]{1,63}(?::\d{1,5})?$/i;

function hostText(ctx: Gl5Context): string | undefined {
  const h = ctx.host;
  return h !== undefined && h.length <= 253 && PLAIN_HOST.test(h) ? h : undefined;
}

const PLAIN_PROFILE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;

/** The workspace as the user knows it: "logfood (host)", "logfood", the host, or undefined. */
function workspaceLabel(ctx: Gl5Context): string | undefined {
  const host = hostText(ctx);
  const p = ctx.profile;
  const name = p !== undefined && PLAIN_PROFILE.test(p) ? p : undefined;
  if (name !== undefined && host !== undefined) return name === host ? host : `${name} (${host})`;
  return name ?? host;
}

/** The codes that point at the MOG's home workspace: the card offers "Change workspace…". */
export const WORKSPACE_CODES: ReadonlySet<BackendErrorCode> = new Set<BackendErrorCode>([
  'PROXY_PAGE',
  'NETWORK_BLOCKED',
  'OFFLINE',
]);

/** The fixed user text of a code (Appendix E, Table W-ERR, the plugin texts). */
export function userTextFor(
  code: BackendErrorCode,
  ctx: Gl5Context = {},
  retryAfterS?: number,
): string {
  const host = hostText(ctx);
  const at = host ?? 'this workspace';
  const ws = workspaceLabel(ctx);
  switch (code) {
    case 'AUTH_EXPIRED':
      return `Sign in to ${host ?? 'Databricks'} to ask Genie.`;
    case 'NETWORK_BLOCKED':
      return `${ws ?? 'This workspace'} refused the connection. Are you on the VPN?`;
    case 'NO_GRANT':
      return `You cannot use Genie in ${at}. Ask a workspace admin for access.`;
    case 'GENIE_ONE_OFF':
      // G-05b; the same text as `mog_core::mcp_transport::genie_one_off_message`.
      return `Genie One isn't available in ${at}.`;
    case 'SPACE_UNAVAILABLE':
      return 'This Genie space is not available.';
    case 'CONVERSATION_BUSY':
      return 'Genie is still answering your last question here.';
    case 'RATE_LIMITED':
      return retryAfterS !== undefined
        ? `Genie busy, retrying in ${Math.min(Math.ceil(retryAfterS), RETRY_AFTER_TEXT_CAP_S)} s`
        : 'Genie busy, retrying soon';
    case 'PROXY_PAGE':
      return `${ws ?? 'This workspace'} didn't answer. Are you on the VPN?`;
    case 'OFFLINE':
      return `Cannot reach ${ws ?? 'this workspace'}. Are you on the VPN or online?`;
    case 'ASK_UNCERTAIN':
      return 'I do not know if Genie got your question.';
    case 'TOOL_BLOCKED':
      return 'Bottle blocked this Genie call.';
    case 'POLICY_DENIED':
      return 'A workspace policy blocked this question.';
    case 'GENIE_FAILED':
      return 'Genie could not answer this question.';
    case 'GENIE_INCOMPLETE':
      return 'Genie did not finish all steps.';
    case 'TIMEOUT_GAVE_UP':
      return 'No answer after 10 min. I will check again later.';
    case 'SECRET_IN_QUESTION':
      return 'The question seems to contain a secret, so Bottle did not send it.';
    case 'STREAM_LOST':
      return 'The connection to Genie stopped.';
    case 'AGENT_FAILED':
      return 'The assistant stopped. Try again.';
    case 'CLIENT_BUG':
      return 'Bottle had an internal error.';
    case 'UNSUPPORTED_AUTH':
      return 'This profile does not use Databricks sign-in in the browser. Sign in with a bottle profile.';
    case 'CLI_MISSING':
      return 'The Databricks CLI is not installed.';
  }
}

/** The Appendix E actions of a code. */
export function failureActions(code: BackendErrorCode): readonly FailureAction[] {
  return ACTIONS[code];
}

function result(
  code: BackendErrorCode,
  row: string,
  ctx: Gl5Context,
  retryAfterS?: number,
): Gl5Result {
  const r: Gl5Result = {
    code,
    userText: userTextFor(code, ctx, retryAfterS),
    row,
    actions: ACTIONS[code],
    retry: RETRY[code],
    autoRetry: code === 'RATE_LIMITED',
  };
  if (code === 'RATE_LIMITED' && retryAfterS !== undefined) r.retryAfterS = retryAfterS;
  return r;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function finiteSeconds(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : undefined;
}

function signals(e: GenieErrorInput): Signals {
  const texts: string[] = [];
  if (typeof e.isErrorText === 'string') texts.push(e.isErrorText);
  const s: Signals = { text: '' };
  if (typeof e.httpStatus === 'number' && Number.isInteger(e.httpStatus)) s.status = e.httpStatus;
  const rpc = e.rpcError;
  if (typeof rpc === 'string') texts.push(rpc);
  if (isRecord(rpc)) {
    if (typeof rpc.code === 'number' && Number.isInteger(rpc.code)) s.rpcCode = rpc.code;
    if (typeof rpc.error_code === 'string') s.errorCode = rpc.error_code.toUpperCase();
    for (const k of ['message', 'error', 'detail'] as const) {
      const v = rpc[k];
      if (typeof v === 'string') texts.push(v);
    }
    const data = rpc.data;
    const after =
      finiteSeconds(rpc.retry_after) ??
      (isRecord(data) ? finiteSeconds(data.retry_after) : undefined);
    if (after !== undefined) s.retryAfterS = after;
  }
  s.text = texts.join('\n');
  return s;
}

/** G-14: classify one failure from its three sources with the GL-5 matrix. */
export function classifyGenieError(e: GenieErrorInput, ctx: Gl5Context = {}): Gl5Result {
  const s = signals(e);
  const row = GL5_MATRIX.find((r) => r.matches(s));
  // The last row (`no-signal`) always matches, so `row` is always set.
  return row
    ? result(row.code, row.id, ctx, s.retryAfterS)
    : result('CLIENT_BUG', 'no-signal', ctx);
}

/** An R8 failure code or an IPC rejection code (`{ error: <code> }`, ADR 0069 decision 4). */
export function classifyTransportCode(code: string, ctx: Gl5Context = {}): Gl5Result {
  const mapped = Object.hasOwn(TRANSPORT_CODES, code) ? TRANSPORT_CODES[code] : undefined;
  return result(mapped ?? 'GENIE_FAILED', mapped ? `transport:${code}` : 'transport:unknown', ctx);
}

/** The GL-5 result of an `mcp_call` reply, or undefined when the reply is not an error. */
export function classifyMcpReply(r: McpCallReply, ctx: Gl5Context = {}): Gl5Result | undefined {
  if (!r.isError) return undefined;
  return classifyGenieError(
    { httpStatus: r.httpStatus, rpcError: r.rpcError, isErrorText: r.text },
    ctx,
  );
}

/** The failed AskEvent of a GL-5 result. */
export function toFailedEvent(r: Gl5Result): Extract<AskEvent, { t: 'failed' }> {
  return { t: 'failed', code: r.code, userText: r.userText, retry: r.retry };
}

/**
 * The failed event of an ask that R8 has stopped (#33, ADR 0084). R8 runs the RATE_LIMITED
 * countdown and its 5 tries (ADR 0075), so a `rate_limited` that reaches the brain is final: the
 * brain starts no second countdown. The card offers [Retry], a user action.
 */
export function finalAskFailure(
  e: Extract<AskEvent, { t: 'failed' }>,
): Extract<AskEvent, { t: 'failed' }> {
  return e.code === 'RATE_LIMITED'
    ? { t: 'failed', code: 'RATE_LIMITED', userText: RATE_LIMITED_FINAL_TEXT, retry: 'user' }
    : e;
}

/** One `mcp_call` of a Genie One tool. A refused call rejects with an `IpcError`. */
export type GenieToolCall = (
  tool: 'genie_ask',
  args: Record<string, unknown>,
) => Promise<McpCallReply>;

/**
 * Send one `genie_ask` and classify the outcome. It never sends the ask again, for any failure
 * (G-15b): a CONVERSATION_BUSY ask waits for [Wait] or [New conversation].
 */
export async function sendGenieAsk(
  call: GenieToolCall,
  args: Record<string, unknown>,
  ctx: Gl5Context = {},
): Promise<Extract<AskEvent, { t: 'accepted' } | { t: 'failed' }>> {
  let reply: McpCallReply;
  try {
    reply = await call('genie_ask', args);
  } catch (e) {
    const code = isRecord(e) && typeof e.error === 'string' ? e.error : 'host_failed';
    return toFailedEvent(classifyTransportCode(code, ctx));
  }
  const failed = classifyMcpReply(reply, ctx);
  if (failed) return toFailedEvent(failed);
  const sc = reply.structuredContent;
  if (
    isRecord(sc) &&
    typeof sc.conversation_id === 'string' &&
    sc.conversation_id !== '' &&
    typeof sc.response_id === 'string' &&
    sc.response_id !== ''
  ) {
    return { t: 'accepted', conversationId: sc.conversation_id, responseId: sc.response_id };
  }
  return toFailedEvent(classifyTransportCode('bad_reply', ctx));
}
