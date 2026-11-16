// Genie One MCP test double (#229, AT-22). A scripted, in-memory server for the bridge tests.
//
// It speaks JSON-RPC 2.0 (`initialize`, `tools/list`, `tools/call`) and also returns the IPC
// `mcp_call` result shape `{ structuredContent, text, isError, httpStatus, rpcError? }`
// (section 4.4). Facts come from research/connector-spec-2026-11-14.md and the live probe of
// 2026-11-14 (D56):
// - tools: genie_ask, genie_poll_response, genie_get_query_result, genie_cancel_response
//   (view_ask is for MCP Apps clients only, so the double does not list it);
// - poll status: in_progress | completed | incomplete | failed | unknown;
// - progress_steps is string[] or null; query_items is [{ item_id, sql? }] or null;
// - deep_link is https://<host>/one/chat/threads/<cid>?o=<wsid>&utm_source=genie_chat_mcp, or null;
// - a second ask on a busy conversation fails with BUSY_TEXT (not a 409).
// TODO(#29): SP-09 records genie_tools.json and the busy envelope (isError or JSON-RPC error).
// Switch the tool list and schemas to genie_tools.json, and set the default busyEnvelope.

import type { CommandReturns } from '../shared/ipc';

export const GENIE_ONE_TOOLS = [
  'genie_ask',
  'genie_poll_response',
  'genie_get_query_result',
  'genie_cancel_response',
] as const;
export type GenieOneTool = (typeof GENIE_ONE_TOOLS)[number];

/** The live busy text of 2026-11-14 (G-15a maps it to CONVERSATION_BUSY). */
export const BUSY_TEXT =
  'asking Genie failed: OneChat stream closed before response.created event. Retry may succeed.';

export type PollStatus = 'in_progress' | 'completed' | 'incomplete' | 'failed' | 'unknown';
export interface QueryItem {
  item_id: string;
  sql?: string | null;
}
export interface PollStep {
  status: PollStatus;
  progress_steps?: string[] | null;
  final_answer?: string | null;
  narration_instruction?: string | null;
  query_items?: QueryItem[] | null;
  /** Omit for the default link on terminal steps; null for the D56 null case. */
  deep_link?: string | null;
}
export interface QueryResult {
  columns: { name: string; type_text: string }[];
  rows: unknown[][];
}
export interface ScriptedResponse {
  /** Poll replies in order. After the last one, the last one repeats. */
  polls: PollStep[];
  queryResults?: Record<string, QueryResult>;
}

/** The IPC `mcp_call` result (src/shared/ipc.ts). */
export type McpCallResult = CommandReturns['mcp_call'];
export interface JsonRpcRequest {
  jsonrpc: '2.0';
  id: number | string;
  method: string;
  params?: unknown;
}
export interface JsonRpcError {
  code: number;
  message: string;
  data?: unknown;
}
export interface JsonRpcResponse {
  jsonrpc: '2.0';
  id: number | string;
  result?: unknown;
  error?: JsonRpcError;
}
export interface RecordedCall {
  tool: string;
  args: Record<string, unknown>;
}

export interface GenieOneMcpDoubleOptions {
  host: string;
  workspaceId: string;
  /** One script per accepted ask, in order. After the last one, the last one repeats. */
  responses?: ScriptedResponse[];
  /** How the busy error comes back. TODO(#29): SP-09 decides the real one. */
  busyEnvelope?: 'isError' | 'jsonrpc';
  /** Await this before a reply, to hold a call open (for overlap tests). */
  beforeReply?: (tool: string) => Promise<void> | undefined;
}

const TERMINAL: ReadonlySet<PollStatus> = new Set(['completed', 'incomplete', 'failed']);

interface ResponseState {
  conversationId: string;
  responseId: string;
  script: ScriptedResponse;
  next: number;
  done: boolean;
  cancelled: boolean;
}

export const scenarios = {
  /** Two polls: one progress step, then a completed answer with a query item. */
  normalAnswer(): ScriptedResponse {
    return {
      polls: [
        { status: 'in_progress', progress_steps: ['Reading the tables'] },
        {
          status: 'completed',
          progress_steps: null,
          final_answer: 'Revenue last quarter was **$1.2M**.',
          query_items: [{ item_id: 'q1', sql: 'SELECT 1' }],
        },
      ],
      queryResults: {
        q1: { columns: [{ name: 'revenue', type_text: 'BIGINT' }], rows: [[1200000]] },
      },
    };
  },
  /** `n` polls with status unknown, then completed (G-18a: unknown is not terminal). */
  unknownStatus(n: number): ScriptedResponse {
    return {
      polls: [
        ...Array.from({ length: n }, (): PollStep => ({ status: 'unknown', progress_steps: null })),
        { status: 'completed', final_answer: 'Done.', query_items: null },
      ],
    };
  },
  /** A completed answer whose deep_link is null (G-19a). */
  nullDeepLink(): ScriptedResponse {
    return {
      polls: [{ status: 'completed', final_answer: 'Done.', deep_link: null, query_items: null }],
    };
  },
  /** In-progress polls with progress_steps null, then completed (G-17a). */
  nullProgressSteps(): ScriptedResponse {
    return {
      polls: [
        { status: 'in_progress', progress_steps: null },
        { status: 'completed', progress_steps: null, final_answer: 'Done.', query_items: null },
      ],
    };
  },
};

function asArgs(v: unknown): Record<string, unknown> {
  return v && typeof v === 'object' ? (v as Record<string, unknown>) : {};
}

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v ? v : undefined;
}

export function createGenieOneMcpDouble(opts: GenieOneMcpDoubleOptions) {
  const scripts = opts.responses ?? [scenarios.normalAnswer()];
  const responses = new Map<string, ResponseState>();
  const active = new Map<string, ResponseState>(); // conversation id -> unfinished response
  const log: RecordedCall[] = [];
  const accepted: RecordedCall[] = [];
  let seq = 0;
  let asked = 0;
  let pollsInFlight = 0;
  let peakPolls = 0;

  const ok = (structuredContent: unknown): McpCallResult => ({
    structuredContent,
    text: JSON.stringify(structuredContent),
    isError: false,
    httpStatus: 200,
  });
  const toolError = (text: string): McpCallResult => ({
    structuredContent: null,
    text,
    isError: true,
    httpStatus: 200,
  });
  const key = (cid: string, rid: string) => `${cid}/${rid}`;
  const deepLink = (cid: string) =>
    `https://${opts.host}/one/chat/threads/${cid}?o=${opts.workspaceId}&utm_source=genie_chat_mcp`;

  function genieAsk(args: Record<string, unknown>): McpCallResult {
    if (!str(args.question)) return toolError('question is required');
    const given = str(args.conversation_id);
    if (given && active.has(given)) {
      return opts.busyEnvelope === 'jsonrpc'
        ? {
            structuredContent: null,
            text: BUSY_TEXT,
            isError: true,
            httpStatus: 200,
            rpcError: { code: -32603, message: BUSY_TEXT },
          }
        : toolError(BUSY_TEXT);
    }
    const conversationId = given ?? `conv-${++seq}`;
    const responseId = `resp-${++seq}`;
    const script = scripts[Math.min(asked, scripts.length - 1)] ?? scenarios.normalAnswer();
    asked++;
    accepted.push({ tool: 'genie_ask', args: { ...args } });
    const state: ResponseState = {
      conversationId,
      responseId,
      script,
      next: 0,
      done: false,
      cancelled: false,
    };
    responses.set(key(conversationId, responseId), state);
    active.set(conversationId, state);
    return ok({ conversation_id: conversationId, response_id: responseId, status: 'in_progress' });
  }

  function geniePoll(args: Record<string, unknown>): McpCallResult {
    const state = responses.get(key(String(args.conversation_id), String(args.response_id)));
    if (!state) return toolError('response not found');
    if (state.cancelled)
      return ok({
        status: 'failed',
        final_answer: null,
        progress_steps: null,
        query_items: null,
        deep_link: null,
      });
    const step = state.script.polls[Math.min(state.next, state.script.polls.length - 1)];
    if (!step) return toolError('empty script');
    state.next++;
    const terminal = TERMINAL.has(step.status);
    if (terminal) {
      state.done = true;
      if (active.get(state.conversationId) === state) active.delete(state.conversationId);
    }
    return ok({
      status: step.status,
      final_answer: step.final_answer ?? null,
      progress_steps: step.progress_steps === undefined ? null : step.progress_steps,
      narration_instruction: step.narration_instruction ?? null,
      query_items: step.query_items ?? null,
      deep_link:
        step.deep_link === undefined
          ? terminal
            ? deepLink(state.conversationId)
            : null
          : step.deep_link,
    });
  }

  function genieQueryResult(args: Record<string, unknown>): McpCallResult {
    const state = responses.get(key(String(args.conversation_id), String(args.response_id)));
    const result = state?.script.queryResults?.[String(args.item_id)];
    return result ? ok(result) : toolError('query item not found');
  }

  function genieCancel(args: Record<string, unknown>): McpCallResult {
    const cid = String(args.conversation_id);
    const state = active.get(cid);
    if (state) {
      state.cancelled = true;
      active.delete(cid);
    }
    return ok({ conversation_id: cid, cancelled: Boolean(state) });
  }

  async function call(tool: string, rawArgs: unknown): Promise<McpCallResult> {
    const args = asArgs(rawArgs);
    log.push({ tool, args: { ...args } });
    const isPoll = tool === 'genie_poll_response';
    if (isPoll) peakPolls = Math.max(peakPolls, ++pollsInFlight);
    try {
      await opts.beforeReply?.(tool);
      switch (tool) {
        case 'genie_ask':
          return genieAsk(args);
        case 'genie_poll_response':
          return geniePoll(args);
        case 'genie_get_query_result':
          return genieQueryResult(args);
        case 'genie_cancel_response':
          return genieCancel(args);
        default:
          return toolError(`unknown tool ${tool}`);
      }
    } finally {
      if (isPoll) pollsInFlight--;
    }
  }

  async function handle(req: JsonRpcRequest): Promise<JsonRpcResponse> {
    const reply = (result: unknown): JsonRpcResponse => ({ jsonrpc: '2.0', id: req.id, result });
    const fail = (code: number, message: string): JsonRpcResponse => ({
      jsonrpc: '2.0',
      id: req.id,
      error: { code, message },
    });
    switch (req.method) {
      case 'initialize':
        return reply({
          protocolVersion: '2025-06-18',
          capabilities: { tools: {} },
          serverInfo: { name: 'genie_one_mcp', version: 'double' },
        });
      case 'tools/list':
        return reply({
          tools: GENIE_ONE_TOOLS.map((name) => ({ name, inputSchema: { type: 'object' } })),
        });
      case 'tools/call': {
        const p = asArgs(req.params);
        const name = String(p.name);
        if (!(GENIE_ONE_TOOLS as readonly string[]).includes(name)) {
          return fail(-32602, `unknown tool ${name}`);
        }
        const r = await call(name, p.arguments);
        if (r.rpcError) return { jsonrpc: '2.0', id: req.id, error: r.rpcError as JsonRpcError };
        return reply({
          content: [{ type: 'text', text: r.text }],
          ...(r.isError ? {} : { structuredContent: r.structuredContent }),
          isError: r.isError,
        });
      }
      default:
        return fail(-32601, `method not found: ${req.method}`);
    }
  }

  return {
    handle,
    call,
    /** Every tools/call in order. */
    calls: (): RecordedCall[] => log.map((c) => ({ tool: c.tool, args: { ...c.args } })),
    /** The genie_ask calls that the double accepted. */
    asks: (): RecordedCall[] => accepted.map((c) => ({ tool: c.tool, args: { ...c.args } })),
    polls: (): RecordedCall[] => log.filter((c) => c.tool === 'genie_poll_response'),
    /** The peak number of genie_poll_response calls open at one time (G-07b wants 1). */
    maxConcurrentPolls: () => peakPolls,
  };
}
