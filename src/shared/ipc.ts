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
  clipboard_write_text: B,
  agent_start: B,
  agent_stop: B,
  agent_send: B,
  agent_tool_result: B,
  model_invoke: B,
  mcp_call: B,
  dbx_http: B,
  mem_local_exec: B,
  mem_wipe: B,
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
  /**
   * ADR 0103: call recording. Brain and hub, like `app_setting_*`: the hub view sends only
   * `view_emit` (ARC-2, MOG-08a), so the brain relays each click. A MOG "Record" click and the
   * record key reach `recording_start` and `recording_stop` through the brain too.
   */
  recording_settings_read: BH,
  recording_settings_write: BH,
  recording_storage_probe: BH,
  recording_volume_create: BH,
  /** Rust refuses it with `recording_off` while the knob is off, then `consent_required`. */
  recording_start: BH,
  recording_stop: BH,
  recording_state: BH,
  recording_list: BH,
  recording_detail: BH,
  recording_retry: BH,
  /** Delete a recording that is only on this Mac (failed upload). Local files only. */
  recording_delete: BH,
  /** Rust builds the Catalog Explorer URL from the profile host and opens it. */
  recording_open: BH,
  /** The "Allow System Audio Recording" fix-it: Rust opens the OS privacy pane. */
  recording_open_system_audio_settings: BH,
  /**
   * Not in 4.5 (#413, #45, #529, ADR 0104): the hub's Local speech step. Hub-only, like
   * `view_notice_subscribe`: `stt_model_install`'s progress Channel always resolves on the hub
   * webview.
   */
  stt_model_status: { windows: ['hub'], phase: 'MVP-D' },
  stt_model_consent: { windows: ['hub'], phase: 'MVP-D' },
  stt_model_install: { windows: ['hub'], phase: 'MVP-D' },
  stt_model_verify: { windows: ['hub'], phase: 'MVP-D' },
  stt_model_cancel: { windows: ['hub'], phase: 'MVP-D' },
  /** #523: brain only. Rust opens an https link on the profile's workspace host in the browser. */
  link_open: B,
  // ADR 0117 (one MOG with workspaces).
  profile_reauth: B,
  workspace_list: B,
  workspace_set_available: B,
  workspace_make_home: B,
  workspace_settings_write: B,
  // ADR 0122: the orchestrator model and the ask loop. All brain-only: the hub uses
  // `view_emit` only (ARC-4).
  orchestrator_models: B,
  orchestrator_status: B,
  agent_turn_submit: B,
  agent_turn_stop: B,
  agent_turn_call: B,
  agent_turn_result: B,
  agent_session_state: B,
  agent_session_end: B,
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
  /**
   * Payload `{ from, input, actionTicket? }`. `actionTicket` (ADR 0113): the host minted a #574
   * ticket for this click (a card "remember", a hub add, edit or rating).
   */
  'view.input': { name: 'view:input', direction: 'core_to_brain' },
  /** Local memory slice 1 (ADR 0113): payload `MemorySaved`, content-free, to one card. */
  'memory.saved': { name: 'memory:saved', direction: 'brain_to_view' },
  /**
   * Not in 4.5 (lamp-alive): where one MOG's eyes look. Payload `{ mogId, dx, dy }`, a vector in
   * the unit disc from the eye centre. Never a pointer position. The brain passes `{ dx, dy }` on
   * to that one MOG view as `mog:look` (`src/shared/mog-look.ts`).
   */
  'mog.look': { name: 'mog:look', direction: 'core_to_brain' },
  /**
   * Not in 4.5 (ADR 0103): the record key (⌥R) in a talk-key hold, with the knob on. Payload:
   * `RecordToggle`. The host sends a `hotkey.ptt` `cancel` first, so no dictation and no paste
   * follow. The brain calls `recording_start`, or `recording_stop` while a recording runs.
   */
  'hotkey.record_toggle': { name: 'hotkey:record_toggle', direction: 'core_to_brain' },
  /**
   * Not in 4.5 (ADR 0103): the one recording of the device. Payload: `RecordingState`. Not
   * sticky: a brain that subscribes later calls `recording_state`.
   */
  'recording.state': { name: 'recording:state', direction: 'core_to_brain' },
  /** Not in 4.5 (ADR 0103): one job status changed. Payload: `RecordingSummary`. */
  'recording.job': { name: 'recording:job', direction: 'core_to_brain' },
  /** The one agent session started or ended. Payload: `AgentSessionEvent`, content-free. */
  'agent.session': { name: 'agent:session', direction: 'core_to_brain' },
  /** Slice 2a: one agent turn moved. Payload: `AgentTurnEvent`, content-free. */
  'agent.turn': { name: 'agent:turn', direction: 'core_to_brain' },
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

/**
 * The hub's Local speech step (#413, #45, #529, ADR 0104). Mirrors
 * `mog_core::stt_setup::SetupStatus`. `model` and `consent` sizes/answers only: `install` and
 * `verify` are what check the SHA-256s.
 */
export interface SttModelStatus {
  model: 'missing' | 'partial' | 'installed';
  /** Bytes of the `.partial` files on disk, when `model` is `'partial'`. */
  partialBytes: number;
  /** The pinned archive size: the "487 MB" text. */
  totalBytes: number;
  consent: 'not_asked' | 'granted' | 'declined';
  installing: boolean;
}

/** One chunk of `stt_model_install` progress, on the hub's private Channel. */
export interface SttModelProgress {
  file: string;
  done: number;
  total: number;
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
  insert_text: { mogId: MogId; text: string; leaseId: LeaseId };
  /** The owner's "attach Genie's answer to the clipboard" request. `html` (ADR 0124 amendment:
   * paste formatted into Docs, email and Excel) is optional; on macOS it is written alongside
   * `text` as `public.html`, next to the plain text. `text` and `html` combined are bound by
   * the Rust `CLIPBOARD_WRITE_MAX_BYTES` (100 KB); a longer pair answers `too_large`. */
  clipboard_write_text: { text: string; html?: string };
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
  /** `actionTicket`: a host-minted #574 ticket the brain passes on, never makes (ADR 0113). */
  mem_local_exec: { stmtId: string; params: JsonObject; actionTicket?: string };
  /** #525 (ADR 0087): the confirm click of the hub Wipe dialog (MEM-33a). */
  mem_wipe: { scope: 'older_than_30d' | 'older_than_90d' | 'everything' };
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
  recording_settings_read: NoArgs;
  recording_settings_write: { patch: RecordingSettingsPatch };
  /**
   * `profile`: the brain's home profile (`src/brain/home-profile.ts`). Rust has no home
   * profile of its own and never picks one; every recording command that reaches Databricks
   * carries it.
   */
  recording_storage_probe: { profile: ProfileName };
  /** Creates the managed volume `bottle_audio` after the user confirmed. `[A-Za-z0-9_-]` names. */
  recording_volume_create: { profile: ProfileName; catalog: string; schema: string };
  /**
   * `profile`: the home profile the pipeline uses for every cloud call of this recording,
   * including after `recording_stop`. `instructions`: for this recording only. Absent: the
   * Settings preset.
   */
  recording_start: { profile: ProfileName; instructions?: string };
  recording_stop: NoArgs;
  recording_state: NoArgs;
  /**
   * `reconcile`: owner authorization 2026-11-25. Absent or `false`: the local recording index,
   * merged with the live overlays, with no network call. `true`: also read the volume once and
   * refresh the index (the hub's first open, or after a recording reaches `done` or `failed`).
   */
  recording_list: { profile: ProfileName; reconcile?: boolean };
  recording_detail: { profile: ProfileName; id: RecordingId };
  recording_retry: { profile: ProfileName; id: RecordingId };
  recording_delete: { profile: ProfileName; id: RecordingId };
  recording_open: {
    profile: ProfileName;
    id: RecordingId;
    target: 'folder' | 'transcript' | 'notes';
  };
  recording_open_system_audio_settings: NoArgs;
  stt_model_status: NoArgs;
  stt_model_consent: { consent: 'granted' | 'declined' };
  stt_model_install: { source: 'github' | 'hf_mirror'; onProgress: Channel<SttModelProgress> };
  stt_model_verify: NoArgs;
  stt_model_cancel: NoArgs;
  link_open: { profile: ProfileName; url: string };
  /**
   * ADR 0117: runs `databricks auth login --profile <p>` on the profile's own host. Refusals:
   * profile_invalid, login_cancelled, timeout, cli_failed, auth_expired, no_grant, offline.
   */
  profile_reauth: { profile: ProfileName };
  workspace_list: { refresh?: boolean };
  workspace_set_available: { profile: ProfileName; available: boolean; mogId?: MogId };
  workspace_make_home: { profile: ProfileName; mogId?: MogId };
  workspace_settings_write: {
    genieCallBudget?: GenieCallBudget;
    /** ADR 0122: the orchestrator model, a `system.ai.<name>` id. Anything else is refused. */
    agentEndpoint?: string;
    /** ADR 0122: "Clean up dictation". A bool only. */
    cleanDictation?: boolean;
    /** ADR 0123: "Dictation engine: Databricks". A bool only. */
    cloudDictation?: boolean;
    /** "Copy Genie answers to the clipboard" (Settings → Workspace). A bool only. */
    copyGenieAnswer?: boolean;
  };
  /** ADR 0122: always the home profile. Bottle never picks one. */
  orchestrator_models: NoArgs;
  /** ADR 0122: `recheck` clears the cached state. */
  orchestrator_status: { recheck?: boolean };
  /**
   * Needs the talk-key latch or an Ask ticket. Starts a session if none is open. `question` is
   * 1 to 1,000 Unicode code points (the minimum is checked after trimming white space), `opId`
   * 1 to 64 and `actionTicket` 1 to 36. Refusals: invalid_args, ask_not_approved,
   * agent_queue_full, agent_rate_limited, agent_session_unavailable,
   * agent_profile_unsupported, agent_signed_out, agent_not_eligible.
   */
  agent_turn_submit: {
    mogId: MogId;
    opId: string;
    question: string;
    mode?: 'ask' | 'follow_up';
    actionTicket?: string;
  };
  /** A live turn: interrupt and cancel its calls. A queued turn: remove it. */
  agent_turn_stop: { turnId: string };
  agent_turn_call: { turnId: string; callId: string };
  agent_turn_result: { turnId: string };
  agent_session_state: NoArgs;
  /** The New session button. Rust ends the session itself on quit, Wipe and a setting change. */
  agent_session_end: { reason: 'new_session' };
}

// ---------- workspaces (ADR 0117) ----------

/** D2 "Genie calls per question". */
export type GenieCallBudget = 5 | 10 | 20;
export type WorkspaceSignIn = 'signed_in' | 'expired' | 'unknown' | 'unsupported' | 'missing';
/** One Databricks CLI profile as a workspace of the one MOG. `mogIds[0]` owns its name and colour. */
export interface WorkspaceRow {
  profile: ProfileName;
  host: string;
  mogIds: MogId[];
  available: boolean;
  isHome: boolean;
  signIn: WorkspaceSignIn;
}
/** D2, the D9 orchestrator model and "Clean up dictation". D1 "Combine across workspaces" is
 * removed (ADR 0122 amendment, owner decision 2026-11-24): cross-workspace data is always on. */
export interface AgentSettings {
  genieCallBudget: GenieCallBudget;
  /** ADR 0122: the orchestrator model, a `system.ai.<name>` id. Default `system.ai.claude-haiku-5-5`. */
  endpoint: string;
  /** ADR 0122: "Clean up dictation". Default true. */
  cleanDictation: boolean;
  /** ADR 0123: "Dictation engine: Databricks". Default false (on this Mac). */
  cloudDictation: boolean;
  /** "Copy Genie answers to the clipboard" (Settings → Workspace). Default true. */
  copyGenieAnswer: boolean;
}
/** The answer of every `workspace_*` command. */
export interface WorkspaceList {
  home: ProfileName | null;
  lampMogId: MogId | null;
  workspaces: WorkspaceRow[];
  agent: AgentSettings;
}

// ---------- the orchestrator model (ADR 0122; Rust mirror in `mog_core::types`) ----------

/** One chat model the home AI Gateway serves. `id` is a `system.ai.<name>` id. */
export interface OrchestratorModelInfo {
  id: string;
  label: string;
  /** There is an eval record for the model (NFR-22a). */
  tested: boolean;
  /** The model supports tool calls. */
  tools: boolean;
}
/** The `orchestrator_models` reply. */
export interface OrchestratorModels {
  models: OrchestratorModelInfo[];
  selected: string;
  defaultModel: string;
}
export type OrchestratorState = 'ready' | 'no_home' | 'signed_out' | 'no_model' | 'unreachable';
/** The `orchestrator_status` reply. Content-free. */
export interface OrchestratorStatus {
  state: OrchestratorState;
  model: string;
}

// ---------- the ask loop (ADR 0122; Rust mirror in `mog_core::types`) ----------

/** One workspace in the catalog of a session. */
export interface AgentWorkspace {
  /** The model alias (`CatalogEntry.alias`), not a Databricks workspace ID. */
  workspaceId: string;
  profile: ProfileName;
  name: string;
  isHome: boolean;
  isAgentWorkspace: boolean;
}
export interface AgentTurnAccepted {
  turnId: string;
  sessionId: string;
  /** 0 is the live turn. */
  position: number;
  newSession: boolean;
  budget: GenieCallBudget;
  catalog: AgentWorkspace[];
}

export type AgentCallState =
  'queued' | 'asking' | 'answered' | 'no_answer' | 'failed' | 'timeout' | 'signed_out' | 'stopped';
/** One Genie call of a turn: the question and the reply, for the card and the history. */
export interface AgentTurnCall {
  callId: string;
  turnId: string;
  workspaceId: string;
  profile: ProfileName;
  question: string;
  state: AgentCallState;
  reply?: CommandReturns['mcp_call'];
  withheld?: 'injection_suspected';
}
export interface AgentTurnResult {
  answer?: string;
  answerKind: 'note' | 'combined' | 'hidden';
  sources: { workspaceId: string; callId: string; turnId: string }[];
  usage?: {
    inputTokens: number;
    cachedInputTokens: number;
    outputTokens: number;
    reasoningTokens: number;
  };
}

export type AgentSessionEndReason =
  'new_session' | 'quit' | 'wipe' | 'profile_changed' | 'catalog_changed' | 'idle';
export interface AgentSessionState {
  state: 'none' | 'idle' | 'busy';
  sessionId?: string;
  startedAt?: number;
  /** The home profile, whose AI Gateway runs the orchestrator model. */
  agentProfile: ProfileName | null;
  turns: number;
  queued: number;
  tokens: number;
}

/** `agent.session` payload (core to brain only, content-free). */
export type AgentSessionEvent =
  | { kind: 'started'; sessionId: string }
  | { kind: 'ended'; sessionId: string; reason: AgentSessionEndReason };

/** `agent.turn` payload (core to brain only, content-free). */
export type AgentTurnEvent =
  | { kind: 'queued'; turnId: string; position: number }
  | { kind: 'started'; turnId: string }
  | { kind: 'thinking'; turnId: string }
  | {
      kind: 'call';
      turnId: string;
      callId: string;
      /** The model alias (`CatalogEntry.alias`), not a Databricks workspace ID. */
      workspaceId: string;
      state: AgentCallState;
      used: number;
      budget: number;
    }
  | {
      kind: 'refused';
      turnId: string;
      code: 'workspace_not_available' | 'budget_reached' | 'invalid_args';
      used: number;
    }
  | {
      /**
       * ADR 0122 amendment (owner decision 2026-11-24): a call to `workspaceId` signed out.
       * Bottle starts that workspace's browser sign-in (U2M OAuth) and retries the call once.
       */
      kind: 'signing_in';
      turnId: string;
      /** The model alias (`CatalogEntry.alias`), not a Databricks workspace ID. */
      workspaceId: string;
    }
  | { kind: 'slow'; turnId: string }
  | {
      kind: 'ended';
      turnId: string;
      outcome:
        | 'answered'
        | 'stopped'
        | 'removed'
        | 'timeout'
        | 'offline'
        | 'signed_out'
        | 'failed'
        | 'refusals'
        | 'session_ended';
      calls: number;
      tokens?: number;
    };

// Card and hub views of an agent turn (brain to view).
export interface AgentWsChip {
  name: string;
  colorVariant: string;
}
/** ADR 0122: one `[n]` citation in a combined summary. */
export interface AgentCitation {
  /**
   * The number in the summary text, 1-based: one number for each workspace, in the order each is
   * first cited (the brain writes the markers of the text again with these numbers).
   */
  n: number;
  callId: string;
  workspace: AgentWsChip;
  /**
   * The Open in Databricks link of that part. A renderer must never use `deepLink` without
   * re-checking it at render time: `openInDatabricksUrl(deepLink, workspaceHost)`.
   */
  deepLink?: string;
  /** The workspace host that `deepLink` must belong to. */
  workspaceHost?: string;
}
export interface AgentCallView {
  callId: string;
  workspace: AgentWsChip;
  question: string;
  state: 'queued' | 'asking' | 'ready' | 'no_answer' | 'needs_you' | 'failed' | 'stopped';
  startedAt?: number;
  endedAt?: number;
}
export interface AgentPartView {
  callId: string;
  workspace: AgentWsChip;
  sections: CardSection[];
  fromEarlier?: boolean;
  /** The agent never saw this part: it was withheld (a quarantine hit). */
  notShownToAgent?: boolean;
  folded?: boolean;
}
export interface AgentTurnView {
  turnId: string;
  question: string;
  /** `needs_you`: the turn ended `signed_out` and the question is kept (UX slice 5). */
  state:
    | 'queued'
    | 'planning'
    | 'running'
    | 'slow'
    | 'done'
    | 'stopped'
    | 'timed_out'
    | 'offline'
    | 'failed'
    | 'needs_you';
  position?: number;
  used: number;
  budget: number;
  agentWorkspace: AgentWsChip;
  calls: AgentCallView[];
  parts: AgentPartView[];
  summary?: {
    md: string;
    kind: 'note' | 'combined';
    workspaces: AgentWsChip[];
    /** ADR 0122: the `[n]` markers of `md`. Absent when the summary cites nothing. */
    citations?: AgentCitation[];
  };
  notice?: string;
  /** The quiet "Copied to clipboard" / "Couldn't copy" note, brain-set once the
   * `clipboard_write_text` call of a finished answer resolves. The brain also clears it (to
   * `undefined`) a short time later, so it never resurfaces on a remount or a reopened thread. */
  copyNote?: 'copied' | 'failed' | undefined;
  tokens?: number;
  /** When the user saw the answer, in Unix ms (UX slice 4, Activity). */
  seenAt?: number;
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
  // Local memory (ADR 0113): the secret check refused a memory write. No row was stored.
  'secret_refused',
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
  // ADR 0117 (one MOG with workspaces).
  'workspace_not_available',
  'home_always_available',
  'workspace_storage_failed',
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
  /**
   * `recording_*` and `capture_start` (ADR 0103): the call recording codes that have no older
   * code. `recording_in_progress`: the talk key cannot take the mic while a recording runs.
   */
  'recording_off',
  'recording_busy',
  'recording_in_progress',
  'unknown_recording',
  'no_storage',
  'volume_permission',
  'transcribe_endpoint_missing',
  'notes_endpoint_missing',
  'too_long',
  'audio_unreadable',
  'upload_failed',
  'system_audio_denied',
  /** `stt_transcribe`: R4 has no installed, checked model (#413, ADR 0104). */
  'stt_model_missing',
  /** `stt_model_*` (#413, #45, #529, ADR 0104): the `SetupError`/`InstallError` codes. */
  'stt_consent_required',
  'stt_url_not_allowed',
  'stt_download_failed',
  'stt_hash_mismatch',
  'stt_extract_failed',
  'stt_io_failed',
  'stt_download_cancelled',
  'stt_install_busy',
  'stt_models_dir_unavailable',
  /** `stt_model_install`: no `local-stt` feature in this build (#534, #414). */
  'stt_setup_unavailable',
  /**
   * `agent_turn_*` and `agent_session_*` (ADR 0122). Model-facing only, never an
   * IPC code: budget_reached, ask_home_first, no_active_turn, run_stopped.
   */
  'ask_not_approved',
  'agent_queue_full',
  'agent_turn_unknown',
  'agent_session_unavailable',
  'agent_rate_limited',
  'agent_profile_unsupported',
  'agent_signed_out',
  'agent_not_eligible',
  'agent_offline',
  'agent_timeout',
  'agent_failed',
  'agent_refusals',
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
  | { id: 'footer'; sttBadge?: string; deepLink?: string; workspaceHost?: string }
  | { id: 'dataFlow'; text: string };

/**
 * `card.update` payload: what the card of one MOG shows. The brain orders and limits the blurbs
 * (`src/brain/blurb-stack.ts`); `historyCount` drives "N more in History" (UX-11a).
 */
export interface CardUpdate {
  mogId: MogId;
  blurbs: CardBlurbView[];
  /**
   * Slice 2a chip ids of an agent turn: `agent-stop:<turnId>`, `agent-more:<opId>` and
   * `agent-resend:<turnId>` (the host mints an Ask ticket for the last two) and
   * `agent-open:<turnId>` (opens the hub Agent area).
   */
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
  /** Slice 2a: the live agent turn in compact form, with "Open in Agent". */
  agentTurn?: AgentTurnView;
  /**
   * The owner's "attach Genie's answer to the clipboard" request, for the direct Genie answer
   * this card shows (an agent turn's own answer carries its note on `agentTurn.copyNote`
   * instead). The brain clears it a short time after setting it.
   */
  copyNote?: 'copied' | 'failed' | undefined;
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

// ---------- call recording (ADR 0103; Rust mirror in `mog_core::types`) ----------

/** A recording ID: 1-64 characters of `[A-Za-z0-9_-]`. */
export type RecordingId = string;

/**
 * Where the recordings go. `root` is a `/Volumes/...` path: the My Files root or a volume
 * `/Volumes/<catalog>/<schema>/<volume>`. Files go under `<root>/Bottle/Recordings/<name>/`.
 */
export interface RecordingStorage {
  kind: 'my_files' | 'volume';
  root: string;
}

/** The recording settings (`recording_settings_*`). No secret. */
export interface RecordingSettings {
  /** The knob. Default off. */
  enabled: boolean;
  /** The user acknowledged the first-use consent notice. Default false. */
  consentAck: boolean;
  storage: RecordingStorage | null;
  /** The model that transcribes the audio. Default `system.ai.gemini-3-8-flash`. */
  transcribeEndpoint: string;
  /** The model that writes the notes. Default `system.ai.claude-sonnet-5-5`. */
  notesEndpoint: string;
  /** The preset prompt. Default: the ADR 0103 prompt. */
  presetInstructions: string;
  /** The recording stops by itself after this many minutes: 5 to 120, default 60. */
  lengthMin: number;
  /**
   * ADR 0118: the one-time note "Your call can still hear you." was shown after the first
   * talk-key hold during a recording. Default false.
   */
  heldNoteShown: boolean;
}

/** A partial `RecordingSettings`. An absent field keeps its value; no field clears to null. */
export type RecordingSettingsPatch = Partial<{
  [K in keyof RecordingSettings]: NonNullable<RecordingSettings[K]>;
}>;

/** `system_audio`: the other side of the call. `silent_or_denied`: 5 s of zero buffers. */
export type SystemAudio = 'ok' | 'unavailable' | 'silent_or_denied' | 'off';

/** The `recording.state` payload. Content-free. Times are epoch milliseconds. */
export interface RecordingState {
  state: 'idle' | 'starting' | 'recording' | 'stopping';
  id?: RecordingId;
  name?: string;
  startedAtMs?: number;
  /** The recording stops by itself at this time; the REC state shows the time left. */
  endsAtMs?: number;
  systemAudio: SystemAudio;
}

/** The status of a recording job. `transcribing`: the model calls per part. */
export type RecordingStatus =
  'recording' | 'uploading' | 'transcribing' | 'writing_notes' | 'done' | 'failed';

/** Why a recording failed (`RecordingSummary.errorKind`). One message and one action each. */
export type RecordingErrorKind =
  | 'recording_off'
  | 'consent_required'
  | 'recording_busy'
  | 'mic_busy'
  | 'no_storage'
  | 'volume_permission'
  | 'transcribe_endpoint_missing'
  | 'notes_endpoint_missing'
  | 'too_long'
  | 'audio_unreadable'
  | 'upload_failed'
  | 'rate_limited'
  | 'network'
  | 'auth'
  | 'system_audio_denied'
  | 'internal';

/** One recordings list row and the `recording.job` payload. Content-free. */
export interface RecordingSummary {
  id: RecordingId;
  name: string;
  startedAtMs: number;
  durationS: number;
  status: RecordingStatus;
  errorKind?: RecordingErrorKind;
  /** The step that failed; Retry starts again there. */
  errorStage?: RecordingStatus;
  /** Parts of the recording that are in the volume. */
  partsUploaded?: number;
  /** Parts of the recording so far (all of them once it has stopped). */
  partsTotal?: number;
  /** The audio is only on this device: no job.json in the volume and no upload runs now. */
  onDeviceOnly?: boolean;
}

/** One transcript turn. Seconds from the start, across parts. `text` is user content. */
export interface RecordingSegment {
  start: number;
  end: number;
  speaker: string;
  text: string;
  part: number;
}

/** The `recording_detail` reply. User content: never log it. */
export interface RecordingDetail {
  summary: RecordingSummary;
  instructions: string;
  segments: RecordingSegment[];
  transcriptMd?: string;
  notesMd?: string;
}

/** The `recording_storage_probe` reply: the My Files root, or `null`. */
export interface StorageProbe {
  myFilesRoot: string | null;
}

/** The `hotkey.record_toggle` payload: the MOG of the talk-key hold. */
export interface RecordToggle {
  mogId: MogId;
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
  insert_text: {
    result: 'inserted' | 'no_permission' | 'no_focus' | 'secure_field' | 'target_changed';
  };
  /** The owner's "attach Genie's answer to the clipboard" request. */
  clipboard_write_text: { result: 'written' | 'too_large' | 'unsupported' };
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
  /** The local rows deleted and the Genie conversations of the wiped exchanges (ids only). */
  mem_wipe: { local: number; conversations: unknown[] };
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
  recording_settings_read: RecordingSettings;
  recording_settings_write: RecordingSettings;
  recording_storage_probe: StorageProbe;
  recording_volume_create: RecordingStorage;
  recording_start: RecordingState;
  recording_stop: RecordingState;
  recording_state: RecordingState;
  /** Newest first. */
  recording_list: RecordingSummary[];
  recording_detail: RecordingDetail;
  recording_retry: RecordingSummary;
  recording_delete: Empty;
  recording_open: Empty;
  recording_open_system_audio_settings: Empty;
  stt_model_status: SttModelStatus;
  stt_model_consent: SttModelStatus;
  stt_model_install: SttModelStatus;
  stt_model_verify: SttModelStatus;
  stt_model_cancel: { cancelled: boolean };
  link_open: Empty;
  profile_reauth: Empty;
  workspace_list: WorkspaceList;
  workspace_set_available: WorkspaceList;
  workspace_make_home: WorkspaceList;
  workspace_settings_write: WorkspaceList;
  orchestrator_models: OrchestratorModels;
  orchestrator_status: OrchestratorStatus;
  agent_turn_submit: AgentTurnAccepted;
  agent_turn_stop: Empty;
  agent_turn_call: AgentTurnCall;
  agent_turn_result: AgentTurnResult;
  agent_session_state: AgentSessionState;
  agent_session_end: Empty;
}

/** Call a command. A refused call rejects with an `IpcError` (a code only). */
export function call<K extends CommandName>(
  name: K,
  args: CommandArgs[K],
): Promise<CommandReturns[K]> {
  return invoke<CommandReturns[K]>(name, { args });
}
