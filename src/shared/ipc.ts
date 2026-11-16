/**
 * The IPC command and event surface of requirements 4.5 (contract, #223).
 *
 * Mirrors `mog_core::ipc` in Rust. A test checks this file against the generated
 * `src-tauri/schemas/ipc-commands.json`. Only the foundation lead changes this file.
 *
 * Calls go through `call(name, args)`, which sends `invoke(name, { args })`. The Rust
 * gateway checks the window label and rejects unknown fields (ARC-4). Arguments never
 * carry a token, a header or a request URL (ARC-6).
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import type {
  CaptureMode,
  CaptureState,
  LeaseId,
  MogId,
  ProfileInfo,
  ProfileName,
  SessionPolicy,
  Tags,
  TokenStatus,
  Transcript,
} from './interfaces';

export type WindowClass = 'brain' | 'hub' | 'mog' | 'card';
export type IpcPhase = 'MVP-D' | 'v1';

type Spec = { readonly windows: readonly WindowClass[]; readonly phase: IpcPhase };
const B = { windows: ['brain'], phase: 'MVP-D' } as const satisfies Spec;
const BH = { windows: ['brain', 'hub'], phase: 'MVP-D' } as const satisfies Spec;

/** Every command of 4.5 in table order, with the windows that may call it. */
export const COMMANDS = {
  cli_status: BH,
  cli_install: BH,
  profiles_list: BH,
  profile_status: BH,
  profile_login: BH,
  profile_select: BH,
  capture_start: B,
  capture_stop: B,
  capture_pause: B,
  capture_status: B,
  capture_stop_all: B,
  capture_subscribe: B,
  session_attest: B,
  stt_transcribe: B,
  insert_text: B,
  agent_start: B,
  agent_stop: B,
  agent_send: B,
  agent_tool_result: B,
  model_invoke: B,
  mcp_call: B,
  dbx_http: B,
  mem_local_exec: B,
  mem_remote_exec: { windows: ['brain'], phase: 'v1' },
  mem_remote_status: { windows: ['brain', 'hub'], phase: 'v1' },
  config_list: BH,
  config_read: BH,
  config_write: BH,
  config_delete: BH,
  policy_get: BH,
  hotkeys_set: BH,
  mog_set_active: B,
  window_mog_open: B,
  window_mog_close: B,
  window_card_show: B,
  window_card_hide: B,
  view_emit: { windows: ['hub', 'mog', 'card'], phase: 'MVP-D' },
  audit_append: B,
  telemetry_span: B,
  /** Not in 4.5 (ADR 0003): the brain's private Channel for core-to-brain traffic. */
  brain_subscribe: B,
} as const satisfies Record<string, Spec>;

export type CommandName = keyof typeof COMMANDS;

/**
 * Event names. Tauri allows only `[A-Za-z0-9-/:_]`, so the dotted names of 4.5 map `.` to
 * `:` (ADR 0003). Keys are the names in the requirements.
 */
export const EVENTS = {
  'hotkey.ptt': { name: 'hotkey:ptt', direction: 'core_to_brain' },
  'hotkey.panic': { name: 'hotkey:panic', direction: 'core_to_brain' },
  'capture.state': { name: 'capture:state', direction: 'core_to_brain' },
  'capture.segment': { name: 'capture:segment', direction: 'core_to_brain' },
  'session.stop_signal': { name: 'session:stop_signal', direction: 'core_to_brain' },
  'auth.status': { name: 'auth:status', direction: 'core_to_brain' },
  'net.status': { name: 'net:status', direction: 'core_to_brain' },
  'mem.remote_status': { name: 'mem:remote_status', direction: 'core_to_brain' },
  'agent.tool_call': { name: 'agent:tool_call', direction: 'core_to_brain' },
  'agent.status': { name: 'agent:status', direction: 'core_to_brain' },
  'mog.view_state': { name: 'mog:view_state', direction: 'brain_to_view' },
  'card.update': { name: 'card:update', direction: 'brain_to_view' },
  'view.input': { name: 'view:input', direction: 'core_to_brain' },
} as const;

export type EventName = (typeof EVENTS)[keyof typeof EVENTS]['name'];

/**
 * A core-to-brain item on the brain Channel (`brain_subscribe`). `core_to_brain` items never
 * travel as Tauri events, because every window that may listen gets every Tauri event.
 * Only `brain_to_view` items are Tauri events.
 */
export interface BrainMessage {
  event: EventName;
  payload: unknown;
}

// ---------- argument types (camelCase, as Rust serializes them) ----------

export type JsonObject = Record<string, unknown>;
export type NoArgs = Record<string, never>;

/** The fixed REST route IDs of 4.6. `dbx_http` takes only these, never a URL. */
export type RouteId =
  | 'scim.me'
  | 'genie.start_conversation'
  | 'genie.message'
  | 'genie.get_message'
  | 'genie.query_result'
  | 'genie.feedback'
  | 'genie.download_viz'
  | 'genie.delete_conversation'
  | 'genie_one.list_conversations'
  | 'genie_one.delete_conversation'
  | 'apps.invocations'
  | 'apps.events'
  | 'apps.responses'
  | 'knowledge.create_page'
  | 'knowledge.get_page'
  | 'knowledge.list_domains'
  | 'knowledge.update_page'
  | 'knowledge.request_publish'
  | 'postgres.credentials';

export type ViewEmitKind =
  'poked' | 'hover' | 'drag_end' | 'chip_click' | 'cancel' | 'expand' | 'click';

interface MogArgs {
  mogId: MogId;
}

export interface CommandArgs {
  cli_status: NoArgs;
  cli_install: { consent: true };
  profiles_list: { refresh?: boolean };
  profile_status: { profile: ProfileName };
  profile_login: { workspaceUrl: string; profile: ProfileName; clientId?: string };
  profile_select: { mogId: MogId; profile: ProfileName; scope: 'question' | 'session' };
  capture_start: {
    mogId: MogId;
    mode: CaptureMode;
    policy?: SessionPolicy;
    consentToken?: string;
    othersTrack?: boolean;
  };
  /** CAP-02a: name the lease (ADR 0005). */
  capture_stop: { mogId: MogId; leaseId: LeaseId };
  capture_pause: { mogId: MogId; leaseId: LeaseId };
  capture_status: NoArgs;
  capture_stop_all: NoArgs;
  capture_subscribe: { onLevel: Channel<number>; onPartial: Channel<string> };
  session_attest: { mogId: MogId; kind: 'only_me' | 'others_agreed'; noticeVersion: number };
  stt_transcribe: {
    segmentId: string;
    provider: Transcript['provider'];
    profile?: ProfileName;
    model?: string;
    language?: string;
    tags: Tags;
  };
  insert_text: { mogId: MogId; text: string };
  agent_start: { mogId: MogId; harness: 'claude' | 'codex'; model?: string };
  agent_stop: MogArgs;
  agent_send: { mogId: MogId; turn: JsonObject };
  agent_tool_result: { mogId: MogId; callId: string; result: unknown };
  model_invoke: {
    role: 'cleanup' | 'router' | 'writer' | 'judge' | 'embed' | 'merge';
    profile: ProfileName;
    request: JsonObject;
    deadlineMs?: number;
    tags: Tags;
  };
  mcp_call: {
    profile: ProfileName;
    server: 'genie_one';
    tool: string;
    args: JsonObject;
    meta?: JsonObject;
    opId: string;
    tags: Tags;
    timeoutMs?: number;
  };
  dbx_http: {
    profile: ProfileName;
    routeId: RouteId;
    params: JsonObject;
    body?: unknown;
    stream?: Channel<unknown>;
    tags: Tags;
  };
  mem_local_exec: { stmtId: string; params: JsonObject };
  mem_remote_exec: { stmtId: string; params: JsonObject };
  mem_remote_status: NoArgs;
  config_list: NoArgs;
  config_read: { id: string };
  config_write: { id: string; json: JsonObject };
  config_delete: { id: string };
  policy_get: NoArgs;
  hotkeys_set: { bindings: JsonObject };
  mog_set_active: MogArgs;
  window_mog_open: {
    mogId: MogId;
    geometry?: { x: number; y: number; width: number; height: number };
  };
  window_mog_close: MogArgs;
  window_card_show: MogArgs;
  window_card_hide: MogArgs;
  view_emit: { mogId: MogId; kind: ViewEmitKind; payload?: unknown };
  audit_append: { kind: string; mogId: MogId; opId: string; sizes: Record<string, number> };
  telemetry_span: { name: string; startMs: number; endMs: number; attrs?: Record<string, number> };
  brain_subscribe: { onEvent: Channel<BrainMessage> };
}

/** The gateway's refusal reasons (`mog_core::ipc::IpcError`). They never carry argument values (4.6 rule 7). */
export type IpcError =
  | { error: 'unknown_command'; command: string }
  | { error: 'window_not_allowed'; command: string; window: string }
  | { error: 'invalid_args'; command: string; reason: string }
  | { error: 'not_implemented'; command: string }
  | { error: 'host_failed'; command: string }
  | { error: 'stale_lease'; command: string };

type Empty = Record<string, never>;

/** Return types of 4.5. `unknown` where a feature issue still defines the shape. */
export interface CommandReturns {
  cli_status: { path: string; version: string; found: boolean; plaintextStorage: boolean };
  cli_install: { path: string; version: string; found: boolean; plaintextStorage: boolean };
  profiles_list: ProfileInfo[];
  profile_status: TokenStatus;
  profile_login: { loginId: string };
  profile_select: Empty;
  capture_start: CaptureState;
  capture_stop: CaptureState;
  capture_pause: CaptureState;
  capture_status: CaptureState;
  capture_stop_all: CaptureState;
  capture_subscribe: Empty;
  session_attest: { consentToken: string };
  stt_transcribe: Transcript;
  insert_text: { result: 'inserted' | 'no_permission' | 'no_focus' | 'secure_field' };
  agent_start: { agentSessionId: string };
  agent_stop: Empty;
  agent_send: Empty;
  agent_tool_result: Empty;
  model_invoke: { json: unknown };
  mcp_call: {
    structuredContent: unknown;
    text: string;
    isError: boolean;
    httpStatus: number;
    rpcError?: unknown;
  };
  dbx_http: { status: number; json?: unknown };
  mem_local_exec: { rows: unknown[] };
  mem_remote_exec: { rows: unknown[] };
  mem_remote_status: {
    endpoint: string;
    state: 'ready' | 'cold' | 'auth_error' | 'unreachable';
    lastOkAt?: string;
  };
  config_list: unknown;
  config_read: unknown;
  config_write: unknown;
  config_delete: unknown;
  policy_get: unknown;
  hotkeys_set: { conflicts: unknown[] };
  mog_set_active: Empty;
  window_mog_open: Empty;
  window_mog_close: Empty;
  window_card_show: Empty;
  window_card_hide: Empty;
  view_emit: Empty;
  audit_append: Empty;
  telemetry_span: Empty;
  brain_subscribe: Empty;
}

/** Call a command. The gateway rejects it with an `IpcError` until its feature issue lands. */
export function call<K extends CommandName>(
  name: K,
  args: CommandArgs[K],
): Promise<CommandReturns[K]> {
  return invoke<CommandReturns[K]>(name, { args });
}
