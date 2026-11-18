/**
 * GL-5 error matrix tests (#32, ADR 0072). One test double per GL-5 row (G-14), the busy text
 * in every source (G-15a), no second ask after a busy reply (G-15b), and the G-05b card text.
 * Every user text is a fixed Bottle text: an upstream marker never shows (W-14, SEC-05a).
 */
import type { BackendErrorCode } from '../../shared/interfaces';
import {
  BUSY_TEXT,
  createGenieOneMcpDouble,
  scenarios,
  type McpCallResult,
} from '../../test-support/genie-one-mcp';
import { createGenieBridge } from './genie-bridge';
import {
  GL5_MATRIX,
  classifyGenieError,
  classifyMcpReply,
  classifyTransportCode,
  failureActions,
  isBusyText,
  sendGenieAsk,
  toFailedEvent,
  userTextFor,
  type GenieErrorInput,
} from './genie-errors';

const HOST = 'example.cloud.databricks.com';
const ctx = { host: HOST };
/** Upstream text that must never reach the card. */
const UP = 'upstream-detail-tok-not-real-7';

interface RowCase {
  row: string;
  e: GenieErrorInput;
  code: BackendErrorCode;
}

/** One test double per GL-5 row (G-14). A meta test checks that every row is here. */
const ROW_CASES: RowCase[] = [
  {
    row: 'busy-text',
    e: { httpStatus: 200, isErrorText: `${BUSY_TEXT} ${UP}` },
    code: 'CONVERSATION_BUSY',
  },
  { row: 'http-401', e: { httpStatus: 401, isErrorText: UP }, code: 'AUTH_EXPIRED' },
  {
    row: 'http-403-ip-acl',
    e: {
      httpStatus: 403,
      rpcError: {
        error_code: 'PERMISSION_DENIED',
        message: `Source IP address is blocked by the IP access list ${UP}`,
      },
    },
    code: 'NETWORK_BLOCKED',
  },
  {
    row: 'http-403',
    e: { httpStatus: 403, rpcError: { error_code: 'PERMISSION_DENIED', message: UP } },
    code: 'NO_GRANT',
  },
  { row: 'http-404', e: { httpStatus: 404, isErrorText: UP }, code: 'GENIE_ONE_OFF' },
  { row: 'http-409', e: { httpStatus: 409, rpcError: { message: UP } }, code: 'CONVERSATION_BUSY' },
  { row: 'http-429', e: { httpStatus: 429, rpcError: { message: UP } }, code: 'RATE_LIMITED' },
  { row: 'http-504', e: { httpStatus: 504, isErrorText: UP }, code: 'ASK_UNCERTAIN' },
  {
    row: 'http-5xx',
    e: { httpStatus: 503, rpcError: { error_code: 'TEMPORARILY_UNAVAILABLE', message: UP } },
    code: 'GENIE_FAILED',
  },
  {
    row: 'http-4xx',
    e: { httpStatus: 400, rpcError: { error_code: 'INVALID_PARAMETER_VALUE', message: UP } },
    code: 'CLIENT_BUG',
  },
  {
    row: 'rpc-protocol',
    e: { httpStatus: 200, rpcError: { code: -32602, message: UP } },
    code: 'CLIENT_BUG',
  },
  {
    row: 'ec-unauthenticated',
    e: { httpStatus: 200, rpcError: { error_code: 'UNAUTHENTICATED', message: UP } },
    code: 'AUTH_EXPIRED',
  },
  {
    row: 'ec-permission-denied',
    e: { httpStatus: 200, rpcError: { error_code: 'PERMISSION_DENIED', message: UP } },
    code: 'NO_GRANT',
  },
  {
    row: 'ec-rate-limit',
    e: { httpStatus: 200, rpcError: { error_code: 'RATE_LIMIT_EXCEEDED_WORKSPACE', message: UP } },
    code: 'RATE_LIMITED',
  },
  {
    row: 'ec-feature-disabled',
    e: { httpStatus: 200, rpcError: { error_code: 'FEATURE_DISABLED', message: UP } },
    code: 'GENIE_ONE_OFF',
  },
  {
    row: 'text-ip-acl',
    e: { httpStatus: 200, isErrorText: `blocked by the IP access list ${UP}` },
    code: 'NETWORK_BLOCKED',
  },
  {
    row: 'text-policy',
    e: { httpStatus: 200, isErrorText: `The request was blocked by a guardrail ${UP}` },
    code: 'POLICY_DENIED',
  },
  {
    row: 'text-rate-limit',
    e: { httpStatus: 200, isErrorText: `Too many requests ${UP}` },
    code: 'RATE_LIMITED',
  },
  { row: 'tool-error', e: { httpStatus: 200, isErrorText: UP }, code: 'GENIE_FAILED' },
  { row: 'no-signal', e: {}, code: 'CLIENT_BUG' },
];

describe('GL-5 matrix: HTTP status, JSON-RPC error and isError text (G-14)', () => {
  it('[G-14] [AT-33] every GL-5 row has a test double here', () => {
    expect(ROW_CASES.map((c) => c.row).sort()).toEqual(GL5_MATRIX.map((r) => r.id).sort());
  });

  it.each(ROW_CASES)('[G-14] [AT-33] row $row maps to $code', ({ row, e, code }) => {
    const r = classifyGenieError(e, ctx);
    expect(r.row).toBe(row);
    expect(r.code).toBe(code);
    expect(GL5_MATRIX.find((m) => m.id === row)?.code).toBe(code);
  });

  it.each(ROW_CASES)('[G-14] [AT-33] row $row never echoes the upstream text', ({ e }) => {
    const r = classifyGenieError(e, ctx);
    expect(r.userText).not.toContain(UP);
    expect(r.userText).not.toMatch(/upstream|tok-not-real/);
    expect(r.userText).toBe(userTextFor(r.code, ctx));
  });

  it('[G-14] the HTTP status wins over the JSON-RPC error and the text, except for the busy text', () => {
    expect(
      classifyGenieError({ httpStatus: 401, rpcError: { error_code: 'PERMISSION_DENIED' } }).code,
    ).toBe('AUTH_EXPIRED');
    expect(classifyGenieError({ httpStatus: 500, isErrorText: 'too many requests' }).code).toBe(
      'GENIE_FAILED',
    );
    expect(classifyGenieError({ httpStatus: 500, isErrorText: BUSY_TEXT }).code).toBe(
      'CONVERSATION_BUSY',
    );
  });

  it('[G-14] the JSON-RPC protocol codes are a Bottle bug; a server error code uses the message', () => {
    for (const code of [-32700, -32600, -32601, -32602]) {
      expect(classifyGenieError({ rpcError: { code, message: 'x' } }).code).toBe('CLIENT_BUG');
    }
    expect(classifyGenieError({ rpcError: { code: -32603, message: 'internal' } }).code).toBe(
      'GENIE_FAILED',
    );
    expect(
      classifyGenieError({ rpcError: { code: -32000, message: 'Too many requests' } }).code,
    ).toBe('RATE_LIMITED');
  });

  it('[G-14] a malformed rpcError or a huge text does not throw and does not leak', () => {
    for (const rpcError of [
      null,
      'a string error',
      42,
      [1, 2],
      { message: 7 },
      { error_code: 3 },
    ]) {
      const r = classifyGenieError({ httpStatus: 200, rpcError, isErrorText: UP });
      expect(r.code).toBe('GENIE_FAILED');
      expect(r.userText).not.toContain(UP);
    }
    const big = `${'x'.repeat(200_000)} ${BUSY_TEXT}`;
    expect(classifyGenieError({ isErrorText: big }).code).toBe('CONVERSATION_BUSY');
  });

  it('[G-14] RATE_LIMITED keeps a numeric retry_after and only RATE_LIMITED retries by itself (ARC-F3)', () => {
    const r = classifyGenieError({ httpStatus: 429, rpcError: { retry_after: 12 } }, ctx);
    expect(r).toMatchObject({ code: 'RATE_LIMITED', autoRetry: true, retryAfterS: 12 });
    expect(r.userText).toBe('Genie busy, retrying in 12 s');
    expect(
      classifyGenieError({ httpStatus: 429, rpcError: { retry_after: 'soon' } }).retryAfterS,
    ).toBeUndefined();
    expect(
      classifyGenieError({ httpStatus: 429, rpcError: { retry_after: -1 } }).retryAfterS,
    ).toBeUndefined();
    for (const c of ROW_CASES.filter((x) => x.code !== 'RATE_LIMITED')) {
      expect(classifyGenieError(c.e).autoRetry).toBe(false);
    }
  });

  it('[G-14] a host that is not a plain host name never goes into the text', () => {
    const r = classifyGenieError({ httpStatus: 404 }, { host: '<img src=x>.example' });
    expect(r.userText).toBe("Genie One isn't available in this workspace.");
  });
});

describe('GL-5 actions and the failed event (Appendix E failure action map)', () => {
  it.each<[BackendErrorCode, string[]]>([
    ['AUTH_EXPIRED', ['sign_in']],
    ['GENIE_ONE_OFF', ['admin_checklist']],
    ['NETWORK_BLOCKED', ['admin_checklist', 'check_vpn']],
    ['NO_GRANT', ['admin_checklist', 'check_vpn']],
    ['OFFLINE', ['retry']],
    ['PROXY_PAGE', ['retry']],
    ['RATE_LIMITED', []],
    ['POLICY_DENIED', ['resend_without_context']],
    ['CONVERSATION_BUSY', ['wait', 'new_conversation']],
    ['GENIE_FAILED', ['retry', 'open_in_databricks']],
  ])('[G-14] %s shows %j', (code, actions) => {
    expect(failureActions(code)).toEqual(actions);
  });

  it('[G-14] the failed AskEvent carries the code, the fixed text and the retry kind', () => {
    expect(toFailedEvent(classifyGenieError({ httpStatus: 401 }, ctx))).toEqual({
      t: 'failed',
      code: 'AUTH_EXPIRED',
      userText: `Sign in to ${HOST} to ask Genie.`,
      retry: 'relogin',
    });
    expect(toFailedEvent(classifyGenieError({ isErrorText: 'blocked by a guardrail' })).retry).toBe(
      'without_preamble',
    );
  });
});

describe('G-05b: 404 without a session is GENIE_ONE_OFF', () => {
  it('[G-05b] [AT-07] the R8 genie_one_off code shows "Genie One isn\'t available in <host>."', () => {
    const r = classifyTransportCode('genie_one_off', ctx);
    expect(r.code).toBe('GENIE_ONE_OFF');
    expect(r.userText).toBe(`Genie One isn't available in ${HOST}.`);
    expect(r.actions).toEqual(['admin_checklist']);
  });

  it('[G-05b] [AT-07] an HTTP 404 reply shows the same text', () => {
    expect(classifyGenieError({ httpStatus: 404 }, ctx).userText).toBe(
      `Genie One isn't available in ${HOST}.`,
    );
  });
});

describe('R8 transport codes (ADR 0069 decision 4)', () => {
  it.each<[string, BackendErrorCode]>([
    ['genie_one_off', 'GENIE_ONE_OFF'],
    ['auth_expired', 'AUTH_EXPIRED'],
    ['rate_limited', 'RATE_LIMITED'],
    ['non_json_reply', 'PROXY_PAGE'],
    ['redirect_refused', 'PROXY_PAGE'],
    ['timeout', 'ASK_UNCERTAIN'],
    ['offline', 'OFFLINE'],
    ['tool_not_allowed', 'TOOL_BLOCKED'],
    ['invalid_tool_args', 'TOOL_BLOCKED'],
    ['invalid_meta', 'TOOL_BLOCKED'],
    ['host_refused', 'TOOL_BLOCKED'],
    ['profile_invalid', 'CLIENT_BUG'],
    ['reply_too_large', 'GENIE_FAILED'],
    ['bad_reply', 'GENIE_FAILED'],
    ['protocol_unsupported', 'GENIE_FAILED'],
    ['host_failed', 'GENIE_FAILED'],
    ['not_implemented', 'CLIENT_BUG'],
    ['poll_inflight', 'CONVERSATION_BUSY'],
    ['some_new_code', 'GENIE_FAILED'],
  ])('[G-14] %s maps to %s', (code, want) => {
    const r = classifyTransportCode(code, ctx);
    expect(r.code).toBe(want);
    expect(r.userText).toBe(userTextFor(want, ctx));
  });
});

describe('G-15a: the busy text is CONVERSATION_BUSY in every source', () => {
  it('[G-15a] [AT-33] the live busy text matches, in any case', () => {
    expect(isBusyText(BUSY_TEXT)).toBe(true);
    expect(isBusyText(BUSY_TEXT.toUpperCase())).toBe(true);
    expect(isBusyText('stream closed before response.created')).toBe(true);
    expect(isBusyText('stream closed')).toBe(false);
    expect(isBusyText('stream closed before responseXcreated')).toBe(false);
  });

  it.each<[string, GenieErrorInput]>([
    ['isError text', { httpStatus: 200, isErrorText: BUSY_TEXT }],
    ['JSON-RPC error', { httpStatus: 200, rpcError: { code: -32603, message: BUSY_TEXT } }],
    [
      'HTTP error body',
      { httpStatus: 500, rpcError: { error_code: 'INTERNAL_ERROR', message: BUSY_TEXT } },
    ],
  ])('[G-15a] [AT-33] busy as %s', (_name, e) => {
    const r = classifyGenieError(e, ctx);
    expect(r.code).toBe('CONVERSATION_BUSY');
    expect(r.userText).toBe('Genie is still answering your last question here.');
    expect(r.actions).toEqual(['wait', 'new_conversation']);
    expect(r.retry).toBe('user');
    expect(r.autoRetry).toBe(false);
    expect(r.userText).not.toContain('Retry may succeed');
  });

  it.each(['isError', 'jsonrpc'] as const)(
    '[G-15a] [AT-33] the double busy reply (%s envelope) classifies as CONVERSATION_BUSY',
    async (busyEnvelope) => {
      const d = createGenieOneMcpDouble({
        host: HOST,
        workspaceId: '1',
        busyEnvelope,
        responses: [scenarios.normalAnswer()],
      });
      const first = (await d.call('genie_ask', { question: 'q1' })).structuredContent as {
        conversation_id: string;
      };
      const busy = await d.call('genie_ask', {
        question: 'q2',
        conversation_id: first.conversation_id,
      });
      expect(classifyMcpReply(busy, ctx)?.code).toBe('CONVERSATION_BUSY');
    },
  );

  it('[G-14] a reply without isError is not an error', () => {
    const ok: McpCallResult = {
      structuredContent: {},
      text: '{}',
      isError: false,
      httpStatus: 200,
    };
    expect(classifyMcpReply(ok, ctx)).toBeUndefined();
  });
});

describe('G-15b: a CONVERSATION_BUSY ask is never sent again without a user action', () => {
  it('[G-15b] [AT-33] sendGenieAsk sends one genie_ask on the busy conversation and no second one', async () => {
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: '1',
      responses: [scenarios.normalAnswer()],
    });
    const first = await sendGenieAsk(d.call, { question: 'q1' }, ctx);
    expect(first.t).toBe('accepted');
    const cid = first.t === 'accepted' ? first.conversationId : '';

    const busy = await sendGenieAsk(d.call, { question: 'q2', conversation_id: cid }, ctx);
    expect(busy).toEqual({
      t: 'failed',
      code: 'CONVERSATION_BUSY',
      userText: 'Genie is still answering your last question here.',
      retry: 'user',
    });
    const asksOnBusy = d
      .calls()
      .filter((c) => c.tool === 'genie_ask' && c.args.conversation_id === cid);
    expect(asksOnBusy).toHaveLength(1);
    expect(d.calls().filter((c) => c.tool === 'genie_ask')).toHaveLength(2);
    expect(d.asks()).toHaveLength(1);
  });

  it('[G-15b] sendGenieAsk never sends again for any failure, and maps a thrown IPC code', async () => {
    let n = 0;
    const failing = async (): Promise<McpCallResult> => {
      n++;
      return { structuredContent: null, text: UP, isError: true, httpStatus: 503 };
    };
    expect(await sendGenieAsk(failing, { question: 'q' }, ctx)).toMatchObject({
      code: 'GENIE_FAILED',
    });
    expect(n).toBe(1);

    let m = 0;
    const refused = (): Promise<McpCallResult> => {
      m++;
      return Promise.reject({ error: 'genie_one_off' });
    };
    expect(await sendGenieAsk(refused, { question: 'q' }, ctx)).toEqual({
      t: 'failed',
      code: 'GENIE_ONE_OFF',
      userText: `Genie One isn't available in ${HOST}.`,
      retry: 'none',
    });
    expect(m).toBe(1);

    const crashed = (): Promise<McpCallResult> => Promise.reject(new Error(UP));
    const r = await sendGenieAsk(crashed, { question: 'q' }, ctx);
    expect(r).toMatchObject({ t: 'failed', code: 'GENIE_FAILED' });
    expect(JSON.stringify(r)).not.toContain(UP);
  });

  it('[G-15b] an ok reply without the IDs is GENIE_FAILED, not accepted', async () => {
    const odd = async (): Promise<McpCallResult> => ({
      structuredContent: { conversation_id: 7 },
      text: '',
      isError: false,
      httpStatus: 200,
    });
    expect(await sendGenieAsk(odd, { question: 'q' }, ctx)).toMatchObject({ code: 'GENIE_FAILED' });
  });
});

describe('GenieBridge.mapError (4.7) uses the GL-5 matrix', () => {
  it('[G-14] [G-15a] mapError gives the GL-5 code and the fixed text', () => {
    const b = createGenieBridge();
    expect(b.mapError({ isErrorText: BUSY_TEXT })).toEqual({
      code: 'CONVERSATION_BUSY',
      userText: 'Genie is still answering your last question here.',
    });
    expect(b.mapError({ httpStatus: 403, isErrorText: UP })).toEqual({
      code: 'NO_GRANT',
      userText: userTextFor('NO_GRANT', {}),
    });
    expect(b.mapError({ httpStatus: 404 }).userText).toBe(
      "Genie One isn't available in this workspace.",
    );
  });
});
