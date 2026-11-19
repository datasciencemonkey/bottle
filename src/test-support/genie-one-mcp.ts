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
// Adapted from datasciencemonkey/antigravity-bricks-plugin plugins/databricks-genie/scripts/genie_tools.json @ a31fd2c (MIT).
// The tool contracts (names, descriptions, input schemas) are `genie-tools.json`, a verbatim copy
// of datasciencemonkey/antigravity-bricks-plugin plugins/databricks-genie/scripts/genie_tools.json
// @ a31fd2c (MIT, see THIRD_PARTY_NOTICES.md). From it: `genie_cancel_response` takes only
// `conversation_id`, is best-effort, and its result status is always `cancellation_requested`.
// TODO(#29): the SP-09 live capture confirms the contracts and records the busy envelope (isError
// or JSON-RPC error); then set the default busyEnvelope.

import { readFileSync } from 'node:fs';
import type { CommandReturns } from '../shared/ipc';

export interface ToolContract {
  name: string;
  description: string;
  inputSchema: {
    type: 'object';
    properties: Record<string, { type: string; description?: string }>;
    required?: string[];
  };
}

/** The Genie One tool contracts (genie-tools.json). */
export const GENIE_TOOL_CONTRACTS: readonly ToolContract[] = JSON.parse(
  readFileSync(new URL('./genie-tools.json', import.meta.url), 'utf8'),
) as ToolContract[];

/**
 * The ways `args` breaks the input schema of `tool`: an unknown tool, a missing required
 * property, a property that the schema does not name, or a value of the wrong JSON type.
 */
export function contractProblems(tool: string, args: Record<string, unknown>): string[] {
  const c = GENIE_TOOL_CONTRACTS.find((t) => t.name === tool);
  if (!c) return [`${tool}: not a Genie One tool`];
  const { properties, required = [] } = c.inputSchema;
  const out: string[] = [];
  for (const k of required) if (!(k in args)) out.push(`${tool}: ${k} is required`);
  for (const [k, v] of Object.entries(args)) {
    const prop = properties[k];
    if (!prop) out.push(`${tool}: ${k} is not in the schema`);
    else if (prop.type === 'string' && typeof v !== 'string')
      out.push(`${tool}: ${k} must be a string`);
  }
  return out;
}

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
  /** Set when Genie cut the result (G-24c, TablePreview.truncated). Omitted when not cut. */
  truncated?: boolean;
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
  /** Status incomplete with a partial answer (G-23a: "Partial answer"). */
  incompleteWithAnswer(): ScriptedResponse {
    return {
      polls: [
        { status: 'in_progress', progress_steps: ['Reading the tables'] },
        {
          status: 'incomplete',
          final_answer: 'Revenue in EMEA was **$410,500** in Q3; other regions did not finish.',
          query_items: null,
        },
      ],
    };
  },
  /** Status incomplete with no answer (G-23b: the card shows a failure). */
  incompleteNoAnswer(): ScriptedResponse {
    return { polls: [{ status: 'incomplete', final_answer: null, query_items: null }] };
  },
  /** Status failed with no answer. */
  failed(): ScriptedResponse {
    return { polls: [{ status: 'failed', final_answer: null, query_items: null }] };
  },
  /** A completed answer whose query result has `rows` rows and is cut (G-24b, G-24c). */
  truncated(rows = 60): ScriptedResponse {
    return {
      polls: [
        {
          status: 'completed',
          final_answer: `The top ${rows} accounts by spend are in the table.`,
          query_items: [{ item_id: 'pub_q1_output', sql: 'SELECT account, spend FROM t' }],
        },
      ],
      queryResults: {
        pub_q1_output: {
          columns: [
            { name: 'account', type_text: 'STRING' },
            { name: 'spend', type_text: 'DECIMAL(12,2)' },
          ],
          rows: Array.from({ length: rows }, (_, i) => [`acct-${i + 1}`, `${1000 + i}.50`]),
          truncated: true,
        },
      },
    };
  },
  /** A short completed answer that ends with "?" and has no query items (G-22: clarify). */
  needsInput(): ScriptedResponse {
    return {
      polls: [
        {
          status: 'completed',
          final_answer: 'Do you mean gross revenue or net revenue?',
          query_items: null,
        },
      ],
    };
  },
  /**
   * #573: the shape of the live run of 2026-11-19 (profile DEFAULT, "What tables can you see?"),
   * with fake values. Polls 1-2 have `progress_steps` null; then the steps grow, each one a
   * "Running SQL:", "Query result:" or "Thinking:" line with raw SQL or rows; the terminal poll
   * has `progress_steps` null again, a markdown `final_answer` (heading, bold numbers) and two
   * query items. `narration_instruction` is null in every poll.
   */
  liveShape(): ScriptedResponse {
    const steps = [
      'Running SQL: SELECT `fake_catalog`, COUNT(*) AS tables FROM `fake`.`tables`',
      'Query result: Query returned 2 rows: | fake_catalog | tables | | --- | --- | | a | 7 |',
      'Thinking: Get totals.',
      'Running SQL: SELECT COUNT(*) AS tables FROM `fake`.`tables`',
    ];
    return {
      polls: [
        { status: 'in_progress', progress_steps: null, narration_instruction: null },
        { status: 'in_progress', progress_steps: null, narration_instruction: null },
        { status: 'in_progress', progress_steps: steps.slice(0, 1), narration_instruction: null },
        { status: 'in_progress', progress_steps: steps.slice(0, 2), narration_instruction: null },
        { status: 'in_progress', progress_steps: steps.slice(0, 3), narration_instruction: null },
        { status: 'in_progress', progress_steps: steps, narration_instruction: null },
        {
          status: 'completed',
          progress_steps: null,
          narration_instruction: null,
          final_answer:
            '## Tables you can access\n\nYou can see about **8,800 tables** in **150 catalogs**.\n\n- Most are in `fake_catalog`.',
          query_items: [
            { item_id: 'fake_item_1', sql: 'SELECT 1' },
            { item_id: 'fake_item_2', sql: 'SELECT 2' },
          ],
        },
      ],
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
  const violations: string[] = [];
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
    // Best-effort and conversation-scoped: the status never says that the turn stopped.
    return ok({ conversation_id: cid, status: 'cancellation_requested' });
  }

  async function call(tool: string, rawArgs: unknown): Promise<McpCallResult> {
    const args = asArgs(rawArgs);
    log.push({ tool, args: { ...args } });
    violations.push(...contractProblems(tool, args));
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
          tools: GENIE_TOOL_CONTRACTS.map((t) => ({
            name: t.name,
            description: t.description,
            inputSchema: t.inputSchema,
          })),
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
    /** Every tools/call that broke its genie-tools.json input schema, in order. */
    contractViolations: (): string[] => [...violations],
  };
}
