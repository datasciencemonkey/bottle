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

/** Views build a Channel argument with this, so no view imports the Tauri core module. */
export { Channel };
import type {
  CaptureMode,
  CaptureState,
  LeaseId,
  MogId,
  Mood,
  MoodMapper,
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
  /** Not in 4.5 (#162): Cancel and "Open the browser again" stop the `auth login` child (W-07h). */
  profile_login_cancel: BH,
  profile_select: BH,
  capture_start: B,
  capture_stop: B,
  capture_pause: B,
  /** #279 (ADR 0039): its own row, so a lost reply cannot flip the pause state. */
  capture_resume: B,
  capture_status: B,
  capture_stop_all: B,
  capture_subscribe: B,
  session_attest: B,
  /** #430 (ADR 0060): T12 in the brain writes and reads the consent records (SEC-32a). */
  consent_notice_record: B,
  consent_records_list: B,
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
  /**
   * #312 (ADR 0039): ask for an OS permission after a click. ADR 0101: the brain relays the
   * click of the hub or the pop-up, because views call only `view_*` (MOG-08a).
   */
  permission_request: BH,
  /** #552 (ADR 0101): the state of an OS permission, with no prompt. */
  permission_status: BH,
  /** #453 (CAP-22): the app-wide settings (Appendix F, scope App), like the `config_*` rows. */
  app_setting_read: BH,
  app_setting_write: BH,
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
  /** Not in 4.5 (#509, ADR 0091): the brain gives the host a notice for the hub only. */
  hub_notice_send: B,
  /** Not in 4.5 (#509, ADR 0091): the hub's private Channel for its notices. */
  view_notice_subscribe: { windows: ['hub'], phase: 'MVP-D' },
} as const satisfies Record<string, Spec>;

export type CommandName = keyof typeof COMMANDS;

/**
 * Event names. Tauri allows only `[A-Za-z0-9-/:_]`, so the dotted names of 4.5 map `.` to
 * `:` (ADR 0003). Keys are the names in the requirements.
 */
export const EVENTS = {
  /**
   * Payload `{ mogId, phase, ask?: true }` (section 4.5). `ask` is only on `up`, after the user
   * tapped G in the hold (RTR-17a, ADR 0098). The `shift` field of ADR 0053 is retired.
   */
  'hotkey.ptt': { name: 'hotkey:ptt', direction: 'core_to_brain' },
  'hotkey.panic': { name: 'hotkey:panic', direction: 'core_to_brain' },
  /**
   * Not in 4.5 yet (ADR 0039): the talk-key status (`TalkKeyStatus`). Sticky: the host sends
   * the last one again when the brain subscribes. One exception: `reason: 'elevated_foreground'`
   * is a one-shot CAP-42 tooltip notice. It is not sticky, and the brain must not change its
   * talk-key state for it: the brain shows the tooltip and keeps the last other status.
   */
  'hotkey.status': { name: 'hotkey:status', direction: 'core_to_brain' },
  'capture.state': { name: 'capture:state', direction: 'core_to_brain' },
  'capture.segment': { name: 'capture:segment', direction: 'core_to_brain' },
  /** Not in 4.5 (ADR 0055): one lease stopped, with its reason. Payload: `CaptureStopped`. */
  'capture.stopped': { name: 'capture:stopped', direction: 'core_to_brain' },
  'session.stop_signal': { name: 'session:stop_signal', direction: 'core_to_brain' },
  'auth.status': { name: 'auth:status', direction: 'core_to_brain' },
  /** Payload: `NetStatus` (ARC-F7, #466). Sticky: the host sends the last one again on subscribe. */
  'net.status': { name: 'net:status', direction: 'core_to_brain' },
  'mem.remote_status': { name: 'mem:remote_status', direction: 'core_to_brain' },
  'agent.tool_call': { name: 'agent:tool_call', direction: 'core_to_brain' },
  'agent.status': { name: 'agent:status', direction: 'core_to_brain' },
  'mog.view_state': { name: 'mog:view_state', direction: 'brain_to_view' },
  'card.update': { name: 'card:update', direction: 'brain_to_view' },
  /** Not in 4.5 (ADR 0040): mic frames for the MOG level meter (UX-06a), sent to `mog-<owner>`. */
  'mog.level': { name: 'mog:level', direction: 'brain_to_view' },
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

/**
 * The `agent.status` payload (#307, `mog_core::agent_host::ipc_route::BrainStatusSink`): the
 * IDs and the session state only, never content. `failed` means the session failed after its
 * one restart (ARC-F14): the brain routes with the fallback classifier (RTR-15b).
 */
export interface AgentStatusPayload {
  mogId: MogId;
  agentSessionId: string;
  state: 'running' | 'restarted' | 'failed';
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

/** The OS permission that `permission_request` asks for and `permission_status` reads. */
export type PermissionKind = 'input_monitoring' | 'accessibility';

/** The reply of `permission_request` and `permission_status`. */
export type PermissionState = 'granted' | 'denied' | 'unknown';

/**
 * The `view_emit` kinds, in the order of the Rust enum. `ready` (#507, ADR 0091): the view
 * registered its listeners; it has no payload, and the brain sends that window its state again.
 */
export const VIEW_EMIT_KINDS = [
  'poked',
  'hover',
  'drag_end',
  'chip_click',
  'cancel',
  'expand',
  'click',
  'ready',
] as const;

export type ViewEmitKind = (typeof VIEW_EMIT_KINDS)[number];

/**
 * A text-bearing hub notice (#509, ADR 0091). It travels only on the hub's private Channel
 * (`view_notice_subscribe`), never as a Tauri event. `text` is user content: never log it.
 */
export interface HubTextNotice {
  kind: 'dictation_unshown';
  mogId: MogId;
  text: string;
}

interface MogArgs {
  mogId: MogId;
}

export interface CommandArgs {
  cli_status: NoArgs;
  cli_install: { consent: true };
  profiles_list: { refresh?: boolean };
  profile_status: { profile: ProfileName };
  /**
   * Blocks until `databricks auth login` and the W-06a check end (#162). Refusals:
   * `host_refused`, `profile_exists`, `invalid_args`, `login_cancelled`, `timeout`, `cli_failed`
   * and the Table W-ERR codes `auth_expired`, `no_grant`, `non_json_reply`, `offline`.
   */
  profile_login: { workspaceUrl: string; profile: ProfileName; clientId?: string };
  profile_login_cancel: { profile: ProfileName };
  profile_select: { mogId: MogId; profile: ProfileName; scope: 'question' | 'session' };
  capture_start: {
    mogId: MogId;
    mode: CaptureMode;
    policy?: SessionPolicy;
    consentToken?: string;
    othersTrack?: boolean;
  };
  /** CAP-02a: name the lease (ADR 0007). */
  capture_stop: { mogId: MogId; leaseId: LeaseId };
  capture_pause: { mogId: MogId; leaseId: LeaseId };
  capture_resume: { mogId: MogId; leaseId: LeaseId };
  capture_status: NoArgs;
  capture_stop_all: NoArgs;
  capture_subscribe: { onLevel: Channel<number>; onPartial: Channel<string> };
  session_attest: { mogId: MogId; kind: 'only_me' | 'others_agreed'; noticeVersion: number };
  /** SEC-32a: only the notice version. The Rust side fills the user, the device and the time. */
  consent_notice_record: { noticeVersion: number };
  consent_records_list: NoArgs;
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
  /** The brain sends it only for a click on a control of this permission (CAP-36, CAP-37). */
  permission_request: { permission: PermissionKind };
  /** No prompt (CAP-36, CAP-37). */
  permission_status: { permission: PermissionKind };
  app_setting_read: { key: AppSettingKey };
  app_setting_write: AppSetting;
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
  hub_notice_send: HubTextNotice;
  view_notice_subscribe: { onNotice: Channel<unknown> };
}

/**
 * Every code a rejection can carry (`mog_core::ipc_gateway::ERROR_CODES`). A test checks this
 * list against the generated schema document.
 */
export const IPC_ERROR_CODES = [
  'unknown_command',
  'window_not_allowed',
  'invalid_args',
  'not_implemented',
  'host_failed',
  'unknown_lease',
  'mic_busy',
  'consent_required',
  'invalid_mode',
  'mog_cap_reached',
  'unknown_mog',
  'window_closing',
  /** `config_*`: the config store is missing or failed (routed in #379). */
  'config_storage_failed',
  /** `consent_*`: no consent records are managed, or the store failed (#430, ADR 0060). */
  'consent_storage_failed',
  /** `config_*` refusals of the MOG config store (#379, MB-02). */
  'invalid_config_id',
  'config_too_large',
  'unsupported_schema_version',
  'invalid_config',
  'config_not_found',
  'config_limit_reached',
  'secret_named_key',
  'token_like_value',
  /** `config_write` G-03 refusals of a rejected Genie backend URL (#28, #379). */
  'genie_beta_mcp_rejected',
  'genie_space_mcp_rejected',
  'genie_one_sse_rejected',
  'mcp_url_not_canonical',
  /** `mcp_call` R8 failure codes (#30, ADR 0069) for the GL-5 mapper (#32, #479). */
  'genie_one_off',
  'auth_expired',
  'rate_limited',
  'non_json_reply',
  'timeout',
  'offline',
  'host_refused',
  'redirect_refused',
  'reply_too_large',
  'bad_reply',
  'protocol_unsupported',
  'tool_not_allowed',
  'invalid_tool_args',
  'invalid_meta',
  'profile_invalid',
  /** `mcp_call`: a poll is already open for the conversation (ARC-F1, G-07b, #31). */
  'poll_inflight',
  /**
   * `agent_*`: the R16 AgentHost codes (#307). `agent_unavailable` tells the brain to route with
   * the fallback classifier (ARC-F14, RTR-15b).
   */
  'agent_unknown_mog',
  'agent_unavailable',
  'agent_launch_failed',
  'agent_target_unavailable',
  'agent_invalid_target',
  'agent_harness_mismatch',
  /** `profile_login` (#162): the sign-in codes that have no older code (W-07g, W-07h). */
  'profile_exists',
  'login_cancelled',
  'cli_failed',
  'no_grant',
] as const;

export type IpcErrorCode = (typeof IPC_ERROR_CODES)[number];

/**
 * What a refused call rejects with (#333, ADR 0040): only a fixed code. It never names the
 * command, the window label or an argument value (SEC-03, 4.6 rule 7).
 */
export interface IpcError {
  error: IpcErrorCode;
}

// ---------- brain-to-view payloads (#285, ADR 0040; Rust mirror in `mog_core::types`) ----------

/** Frame-probe times of one `mog.view_state` (#13, ADR 0031). Numbers only. */
export interface ViewTrace {
  seq: number;
  /** When the event that caused this state reached the brain. */
  eventAt: number;
  /** When the brain sent this state. */
  sentAt: number;
  /** For `listening`: when the key-down reached the brain (UX-05b). */
  keyDownAt?: number;
}

/** One MOG badge (MOG-07, UX-38, UX-39a). `hoverOnly` badges show while the pointer is on the MOG. */
export interface MogBadgeView {
  kind: string;
  text: string;
  label: string;
  detail?: string;
  hoverOnly: boolean;
  /** Set when a click on the badge does something; the brain decides what (ADR 0029). */
  action?: string;
}

/** `mog.view_state` payload: what the brain computed for one MOG. */
export interface MogViewState {
  mogId: MogId;
  mood: Mood;
  overlays: ReturnType<MoodMapper>['overlays'];
  /** The badges the brain computed (`mapBadges`, ADR 0029). Each has text and a label. */
  badges?: MogBadgeView[];
  /** The home domain picker of this MOG in its home workspace (PG-18, PG-20). */
  homeDomain?: DomainPickerView;
  /** Only with `VITE_BOTTLE_LATENCY_TRACE=1`: times for the frame probe (#13, ADR 0031). */
  trace?: ViewTrace;
}

/**
 * The domain picker (PG-20): the domain list, or `paste` (the reason text) when the list cannot
 * be read. `scope` 'home' sets the MOG home domain (PG-18); 'draft' changes one draft (PG-19).
 */
export interface DomainPickerView {
  scope: 'home' | 'draft';
  /** Set for scope 'draft': the Page review that the change applies to. */
  reviewId?: string;
  current?: { id: string; name?: string };
  options?: { id: string; name: string }[];
  paste?: string;
  error?: string;
}

/** The admin checklist for a failed Pages preflight (PG-11, PG-12): the reason and the items. */
export interface PagesChecklistView {
  text: string;
  items: string[];
}

/** One card slot (ADR 0028): a blurb, or a split answer with its parts in one slot (UX-11b). */
export interface CardBlurbView {
  id: string;
  text: string;
  pinned?: boolean;
  completed?: boolean;
  parts?: { id: string; text: string }[];
}

/** One expanded card section (mirrors `ExpandedSection` of the brain). */
export type CardSection =
  | { id: 'answer'; md: string }
  | { id: 'keyNumbers'; items: string[] }
  | { id: 'sql'; sql: string; collapsed: true }
  | {
      id: 'tablePreview';
      columns: string[];
      rows: string[][];
      moreRows: number;
      moreColumns?: number;
    }
  | { id: 'sentToGenie'; text: string }
  | { id: 'resend' }
  | { id: 'copy'; formats: ('markdown' | 'csv')[] }
  | { id: 'footer'; sttBadge?: string; deepLink?: string }
  | { id: 'dataFlow'; text: string };

/**
 * `card.update` payload: what the card of one MOG shows. The brain orders and limits the blurbs
 * (`src/brain/blurb-stack.ts`); `historyCount` drives "N more in History" (UX-11a).
 */
export interface CardUpdate {
  mogId: MogId;
  blurbs: CardBlurbView[];
  chips: { id: string; label: string }[];
  /** Chips after the 4th (UX-20a): the view shows them in the expanded card. */
  overflowChips?: { id: string; label: string }[];
  /** The expanded card sections in the UX-EXP order (UX-18), from `src/brain/card-sections.ts`. */
  sections?: CardSection[];
  answerMd?: string;
  historyCount?: number;
  /** The STT badge for the card footer (UX-38), for example "STT: local". */
  sttBadge?: string;
  /** The domain of the Page draft in review (PG-19). */
  pageDomain?: DomainPickerView;
  pagesChecklist?: PagesChecklistView;
}

/** Why a capture lease stopped (ADR 0055). Mirrors `mog_core::types::CaptureStopReason`. */
export type CaptureStopReason =
  | 'manual'
  | 'hold_cap'
  | 'hands_free_cap'
  | 'preempted'
  | 'panic'
  | 'device_lost'
  | 'released'
  | 'discarded'
  | 'tap'
  | 'escape'
  | 'vad_end'
  | 'zero_buffers';

/**
 * The `capture.stopped` payload (#425, ADR 0055): one lease stopped. Content-free (CAP-01b).
 * Mirrors `mog_core::types::CaptureStopped`.
 */
export interface CaptureStopped {
  leaseId: LeaseId;
  mogId: MogId;
  mode: CaptureMode;
  reason: CaptureStopReason;
}

/**
 * `mog.level` payload: one mic level sample (UX-06a). `owner` is the MOG that owns the open
 * mic, or `null` when the mic closed: the brain sends one such frame on close (UX-06b).
 */
export interface MicLevelFrame {
  owner: MogId | null;
  dbfs: number;
  /** Milliseconds on a monotonic clock. */
  t: number;
}

// ---------- core-to-brain payloads (#297, #294, ADR 0039; Rust mirror in `mog_core::types`) ----------

/** The talk key now. */
export type TalkKeyState = 'active' | 'paused' | 'unavailable';

/** Why the talk key is not active. A fixed value: never a window title, process or key. */
export type TalkKeyReason =
  'secure_input' | 'elevated_foreground' | 'fallback_chord_unavailable' | 'hook_failed';

/**
 * `hotkey.status` payload. `reason` is required and `null` when the talk key is active. The
 * brain picks the pill (CAP-39c) or the tooltip (CAP-42) by `reason`.
 */
export interface TalkKeyStatus {
  talkKey: TalkKeyState;
  reason: TalkKeyReason | null;
}

/**
 * `net.status` payload (ARC-F7). Two failed connects mark the host offline. Sticky: the host
 * sends the last one again when the brain subscribes. No host name, URL or error text.
 */
export interface NetStatus {
  online: boolean;
}

// ---------- consent records (#430, ADR 0060; Rust mirror in `mog_core::types`) ----------

/** What a consent record is for: the notice, or the attestation choice of a session. */
export type ConsentRecordKind = 'notice' | 'only_me' | 'others_agreed';

// ---------- app settings (#453, ADR 0063; Rust mirror in `mog_core::types`) ----------

/** The key of an app-wide setting (Appendix F, scope App; #453). Mirrors `AppSettingKey`. */
export type AppSettingKey = 'app.capture.preferBuiltInMic';

/**
 * One app-wide setting with its typed value (#453): the `app_setting_write` args and the
 * `app_setting_*` reply. Mirrors `mog_core::types::AppSetting`.
 * CAP-22 `app.capture.preferBuiltInMic`: "Prefer the built-in mic when a Bluetooth headset
 * connects". Default off.
 */
export type AppSetting = { key: 'app.capture.preferBuiltInMic'; value: boolean };

/** One record of `consent_records_list`. Content-free: no user, no device and no MOG ID. */
export interface ConsentRecordView {
  recordId: string;
  kind: ConsentRecordKind;
  noticeVersion: number;
  /** `null` for a notice record. */
  sessionId: string | null;
  /** SEC-32a `accepted_at`, Unix ms. */
  acceptedAt: number;
}

type Empty = Record<string, never>;

/** Return types of 4.5. `unknown` where a feature issue still defines the shape. */
export interface CommandReturns {
  cli_status: { path: string; version: string; found: boolean; plaintextStorage: boolean };
  cli_install: { path: string; version: string; found: boolean; plaintextStorage: boolean };
  profiles_list: ProfileInfo[];
  profile_status: TokenStatus;
  profile_login: { loginId: string };
  profile_login_cancel: Empty;
  profile_select: Empty;
  capture_start: CaptureState;
  capture_stop: CaptureState;
  capture_pause: CaptureState;
  capture_resume: CaptureState;
  capture_status: CaptureState;
  capture_stop_all: CaptureState;
  capture_subscribe: Empty;
  session_attest: { consentToken: string };
  consent_notice_record: { recordId: string };
  /** Oldest first. */
  consent_records_list: { records: ConsentRecordView[] };
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
  permission_request: { status: PermissionState };
  permission_status: { status: PermissionState };
  app_setting_read: AppSetting;
  app_setting_write: AppSetting;
  mog_set_active: Empty;
  window_mog_open: Empty;
  window_mog_close: Empty;
  window_card_show: Empty;
  window_card_hide: Empty;
  view_emit: Empty;
  audit_append: Empty;
  telemetry_span: Empty;
  brain_subscribe: Empty;
  hub_notice_send: Empty;
  view_notice_subscribe: Empty;
}

/** Call a command. A refused call rejects with an `IpcError` (a code only). */
export function call<K extends CommandName>(
  name: K,
  args: CommandArgs[K],
): Promise<CommandReturns[K]> {
  return invoke<CommandReturns[K]>(name, { args });
}
