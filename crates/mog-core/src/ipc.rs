//! The IPC command and event surface of requirements 4.5 (contract, #223).
//!
//! One table (`COMMANDS`) lists every command with the window classes that may call it.
//! `handle` is the shared entry for the Tauri host and the C ABI: it checks the window
//! label (ARC-4), parses the arguments with `deny_unknown_fields`, and then reaches the
//! component stub. Arguments never carry a token, a header or a request URL (ARC-6, 4.6).
//!
//! `COMMANDS` is 4.5 plus `brain_subscribe` (ADR 0003). Only the foundation lead changes this
//! file (brief, "Contracts").

use schemars::{JsonSchema, Schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub mod events;

// ---------- window classes ----------

/// The window classes of ARC-3, as the IPC table names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowClass {
    Brain,
    Hub,
    Mog,
    Card,
}

impl WindowClass {
    /// The class of a window label: `brain`, `hub`, `mog-<id>` or `card-<id>`.
    pub fn of_label(label: &str) -> Option<(WindowClass, Option<&str>)> {
        let id_ok = |id: &str| {
            !id.is_empty()
                && id.len() <= 64
                && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        };
        match label {
            "brain" => Some((WindowClass::Brain, None)),
            "hub" => Some((WindowClass::Hub, None)),
            _ => {
                if let Some(id) = label.strip_prefix("mog-") {
                    id_ok(id).then_some((WindowClass::Mog, Some(id)))
                } else if let Some(id) = label.strip_prefix("card-") {
                    id_ok(id).then_some((WindowClass::Card, Some(id)))
                } else {
                    None
                }
            }
        }
    }
}

/// The phase of a command (requirements phases).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Phase {
    #[serde(rename = "MVP-D")]
    MvpD,
    #[serde(rename = "v1")]
    V1,
}

/// One row of the 4.5 table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CommandSpec {
    pub name: &'static str,
    pub windows: &'static [WindowClass],
    pub phase: Phase,
}

const B: &[WindowClass] = &[WindowClass::Brain];
const BH: &[WindowClass] = &[WindowClass::Brain, WindowClass::Hub];
const VIEWS: &[WindowClass] = &[WindowClass::Hub, WindowClass::Mog, WindowClass::Card];

const fn row(name: &'static str, windows: &'static [WindowClass]) -> CommandSpec {
    CommandSpec {
        name,
        windows,
        phase: Phase::MvpD,
    }
}

const fn v1(name: &'static str, windows: &'static [WindowClass]) -> CommandSpec {
    CommandSpec {
        name,
        windows,
        phase: Phase::V1,
    }
}

/// Every command of 4.5, in table order. Slash pairs are separate rows.
/// `capture_stop_all` is also reachable from the tray inside Rust; that path is not IPC.
pub const COMMANDS: &[CommandSpec] = &[
    row("cli_status", BH),
    row("cli_install", BH),
    row("profiles_list", BH),
    row("profile_status", BH),
    row("profile_login", BH),
    row("profile_select", BH),
    row("capture_start", B),
    row("capture_stop", B),
    row("capture_pause", B),
    row("capture_status", B),
    row("capture_stop_all", B),
    row("capture_subscribe", B),
    row("session_attest", B),
    row("stt_transcribe", B),
    row("insert_text", B),
    row("agent_start", B),
    row("agent_stop", B),
    row("agent_send", B),
    row("agent_tool_result", B),
    row("model_invoke", B),
    row("mcp_call", B),
    row("dbx_http", B),
    row("mem_local_exec", B),
    v1("mem_remote_exec", B),
    v1("mem_remote_status", BH),
    row("config_list", BH),
    row("config_read", BH),
    row("config_write", BH),
    row("config_delete", BH),
    row("policy_get", BH),
    row("hotkeys_set", BH),
    row("mog_set_active", B),
    row("window_mog_open", B),
    row("window_mog_close", B),
    row("window_card_show", B),
    row("window_card_hide", B),
    row("view_emit", VIEWS),
    row("audit_append", B),
    row("telemetry_span", B),
    // Not in 4.5 (ADR 0003): the brain's private Channel for core-to-brain traffic.
    row("brain_subscribe", B),
];

/// The row of a command, if it exists.
pub fn command(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.name == name)
}

// ---------- errors and results ----------

/// Why the gateway refused a call. No variant carries argument values (secret scrubbing, R12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum IpcError {
    UnknownCommand {
        command: String,
    },
    WindowNotAllowed {
        command: String,
        window: String,
    },
    InvalidArgs {
        command: String,
        reason: String,
    },
    NotImplemented {
        command: String,
    },
    /// The host could not deliver the result (for example no brain channel yet).
    HostFailed {
        command: String,
    },
    /// CAP-02a: no live lease with this ID belongs to `mogId`. R2 `CaptureError::UnknownLease`
    /// maps here; the host that wires R2 (#4) also returns it when the lease owner is another
    /// MOG, so a caller cannot probe the leases of other MOGs (ADR 0005).
    UnknownLease {
        command: String,
    },
    /// 4.5 `capture_start`: another capture holds the mic (R2 `MicBusy`, CAP-01a).
    MicBusy {
        command: String,
    },
    /// 4.5 `capture_start`: a session needs a valid consent token first.
    ConsentRequired {
        command: String,
    },
    /// The command does not apply to the mode of this lease (R2 `InvalidMode`).
    InvalidMode {
        command: String,
    },
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::UnknownCommand { command } => write!(f, "unknown command {command}"),
            IpcError::WindowNotAllowed { command, window } => {
                write!(f, "window {window} may not call {command}")
            }
            IpcError::InvalidArgs { command, reason } => {
                write!(f, "invalid arguments for {command}: {reason}")
            }
            IpcError::NotImplemented { command } => write!(f, "{command} is not implemented"),
            IpcError::HostFailed { command } => write!(f, "the host could not complete {command}"),
            IpcError::UnknownLease { command } => {
                write!(f, "{command}: no live capture lease with this ID")
            }
            IpcError::MicBusy { command } => write!(f, "{command}: the mic is busy"),
            IpcError::ConsentRequired { command } => write!(f, "{command}: consent is required"),
            IpcError::InvalidMode { command } => {
                write!(f, "{command} does not apply to this capture mode")
            }
        }
    }
}

impl std::error::Error for IpcError {}

/// What the host must do after the gateway accepted a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handled {
    /// `view_emit`: relay the arguments to the brain as `view:input` (never to a view).
    RelayToBrain { from: String },
    /// `brain_subscribe`: keep this Channel as the brain's private delivery path.
    SubscribeBrain { channel: ChannelRef },
}

// ---------- shared argument types ----------

pub use crate::types::{CaptureMode, MogId, ProfileName, SessionPolicy, Tags, TranscriptProvider};

/// A lease ID argument (CAP-02a): 1-64 characters of `[A-Za-z0-9-]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LeaseRef(String);

/// The wire rule of a lease ID, also stated in the JSON Schema.
pub const LEASE_PATTERN: &str = "^[A-Za-z0-9-]{1,64}$";

impl JsonSchema for LeaseRef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LeaseRef".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> Schema {
        schemars::json_schema!({ "type": "string", "pattern": LEASE_PATTERN })
    }
}

impl LeaseRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for LeaseRef {
    type Error = &'static str;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        let ok = !s.is_empty()
            && s.len() <= 64
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if ok {
            Ok(LeaseRef(s))
        } else {
            Err("not a lease ID")
        }
    }
}

impl From<LeaseRef> for String {
    fn from(l: LeaseRef) -> String {
        l.0
    }
}

/// The wire prefix of a Tauri `Channel` argument (`__CHANNEL__:<id>`).
pub const CHANNEL_PREFIX: &str = "__CHANNEL__:";

/// A Tauri `Channel` argument. On the wire it is `__CHANNEL__:<numeric id>`; other strings fail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ChannelRef(String);

impl JsonSchema for ChannelRef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ChannelRef".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> Schema {
        schemars::json_schema!({ "type": "string", "pattern": "^__CHANNEL__:[0-9]+$" })
    }
}

impl ChannelRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ChannelRef {
    type Error = &'static str;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.strip_prefix(CHANNEL_PREFIX) {
            Some(id) if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) => {
                Ok(ChannelRef(s))
            }
            _ => Err("not a channel reference"),
        }
    }
}

impl From<ChannelRef> for String {
    fn from(c: ChannelRef) -> String {
        c.0
    }
}

/// A JSON object whose shape another contract owns (for example `AgentTurn` in 4.7).
pub type JsonObject = BTreeMap<String, Value>;

macro_rules! args {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        pub struct $name { $($(#[$fm])* pub $field: $ty),* }
    };
}

args!(
    /// No arguments.
    NoArgs {}
);
args!(CliInstallArgs {
    /// Must be `true`: the user gave consent (U16).
    consent: bool,
});
args!(ProfilesListArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh: Option<bool>,
});
args!(ProfileArgs {
    profile: ProfileName
});
args!(ProfileLoginArgs {
    /// The workspace that `databricks auth login --host` opens. Not a request target.
    workspace_url: String,
    profile: ProfileName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SelectScope {
    Question,
    Session,
}
args!(ProfileSelectArgs {
    mog_id: MogId,
    profile: ProfileName,
    scope: SelectScope,
});

args!(CaptureStartArgs {
    mog_id: MogId,
    mode: CaptureMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    policy: Option<SessionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    consent_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    others_track: Option<bool>,
});
args!(MogArgs { mog_id: MogId });
args!(
    /// CAP-02a: stop and pause name the lease they act on. R2 rejects a stale one.
    CaptureLeaseArgs {
        mog_id: MogId,
        lease_id: LeaseRef,
    }
);
args!(CaptureSubscribeArgs {
    on_level: ChannelRef,
    on_partial: ChannelRef,
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttestKind {
    OnlyMe,
    OthersAgreed,
}
args!(SessionAttestArgs {
    mog_id: MogId,
    kind: AttestKind,
    notice_version: u32,
});

args!(SttTranscribeArgs {
    segment_id: String,
    provider: TranscriptProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<ProfileName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    tags: Tags,
});
args!(InsertTextArgs {
    mog_id: MogId,
    text: String,
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    Claude,
    Codex,
}
args!(AgentStartArgs {
    mog_id: MogId,
    harness: Harness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
});
args!(AgentSendArgs {
    mog_id: MogId,
    /// `AgentTurn` of 4.7 (owned by #222 and the agent issues).
    turn: JsonObject,
});
args!(AgentToolResultArgs {
    mog_id: MogId,
    call_id: String,
    result: Value,
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    Cleanup,
    Router,
    Writer,
    Judge,
    Embed,
    Merge,
}
args!(ModelInvokeArgs {
    role: ModelRole,
    profile: ProfileName,
    request: JsonObject,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deadline_ms: Option<u32>,
    tags: Tags,
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpServer {
    GenieOne,
}
args!(McpCallArgs {
    profile: ProfileName,
    server: McpServer,
    tool: String,
    /// Tool arguments. R8 strips any `profile` key before it forwards them (4.6 rule 5).
    args: JsonObject,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    meta: Option<JsonObject>,
    op_id: String,
    tags: Tags,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u32>,
});

/// The fixed REST route IDs of 4.6. `dbx_http` takes only these, never a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RouteId {
    #[serde(rename = "scim.me")]
    ScimMe,
    #[serde(rename = "genie.start_conversation")]
    GenieStartConversation,
    #[serde(rename = "genie.message")]
    GenieMessage,
    #[serde(rename = "genie.get_message")]
    GenieGetMessage,
    #[serde(rename = "genie.query_result")]
    GenieQueryResult,
    #[serde(rename = "genie.feedback")]
    GenieFeedback,
    #[serde(rename = "genie.download_viz")]
    GenieDownloadViz,
    #[serde(rename = "genie.delete_conversation")]
    GenieDeleteConversation,
    #[serde(rename = "genie_one.list_conversations")]
    GenieOneListConversations,
    #[serde(rename = "genie_one.delete_conversation")]
    GenieOneDeleteConversation,
    #[serde(rename = "apps.invocations")]
    AppsInvocations,
    #[serde(rename = "apps.events")]
    AppsEvents,
    #[serde(rename = "apps.responses")]
    AppsResponses,
    #[serde(rename = "knowledge.create_page")]
    KnowledgeCreatePage,
    #[serde(rename = "knowledge.get_page")]
    KnowledgeGetPage,
    #[serde(rename = "knowledge.list_domains")]
    KnowledgeListDomains,
    #[serde(rename = "knowledge.update_page")]
    KnowledgeUpdatePage,
    #[serde(rename = "knowledge.request_publish")]
    KnowledgeRequestPublish,
    #[serde(rename = "postgres.credentials")]
    PostgresCredentials,
}
args!(DbxHttpArgs {
    profile: ProfileName,
    route_id: RouteId,
    params: JsonObject,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stream: Option<ChannelRef>,
    tags: Tags,
});
args!(MemExecArgs {
    /// A statement ID of the fixed catalog (R9, R10). Never SQL text.
    stmt_id: String,
    params: JsonObject,
});
args!(ConfigIdArgs { id: String });
args!(ConfigWriteArgs {
    id: String,
    json: JsonObject,
});
args!(HotkeysSetArgs {
    bindings: JsonObject
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
args!(WindowMogOpenArgs {
    mog_id: MogId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    geometry: Option<Geometry>,
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewEmitKind {
    Poked,
    Hover,
    DragEnd,
    ChipClick,
    Cancel,
    Expand,
    Click,
}
args!(ViewEmitArgs {
    mog_id: MogId,
    kind: ViewEmitKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payload: Option<Value>,
});
args!(AuditAppendArgs {
    kind: String,
    mog_id: MogId,
    op_id: String,
    /// Content-free sizes, for example `{"chars": 120}`.
    sizes: BTreeMap<String, u64>,
});
args!(BrainSubscribeArgs {
    on_event: ChannelRef
});
args!(TelemetrySpanArgs {
    name: String,
    start_ms: u64,
    end_ms: u64,
    /// Content-free attributes only.
    #[serde(default)]
    attrs: BTreeMap<String, serde_json::Number>,
});

// ---------- dispatch ----------

/// Calls `$f::<ArgType>(name)` for the argument type of each command.
macro_rules! for_command {
    ($name:expr, $f:ident) => {
        match $name {
            "cli_status" | "capture_status" | "capture_stop_all" | "mem_remote_status"
            | "config_list" | "policy_get" => Some($f::<NoArgs>()),
            "cli_install" => Some($f::<CliInstallArgs>()),
            "profiles_list" => Some($f::<ProfilesListArgs>()),
            "profile_status" => Some($f::<ProfileArgs>()),
            "profile_login" => Some($f::<ProfileLoginArgs>()),
            "profile_select" => Some($f::<ProfileSelectArgs>()),
            "capture_start" => Some($f::<CaptureStartArgs>()),
            "capture_stop" | "capture_pause" => Some($f::<CaptureLeaseArgs>()),
            "agent_stop" | "mog_set_active" | "window_mog_close" | "window_card_show"
            | "window_card_hide" => Some($f::<MogArgs>()),
            "capture_subscribe" => Some($f::<CaptureSubscribeArgs>()),
            "session_attest" => Some($f::<SessionAttestArgs>()),
            "stt_transcribe" => Some($f::<SttTranscribeArgs>()),
            "insert_text" => Some($f::<InsertTextArgs>()),
            "agent_start" => Some($f::<AgentStartArgs>()),
            "agent_send" => Some($f::<AgentSendArgs>()),
            "agent_tool_result" => Some($f::<AgentToolResultArgs>()),
            "model_invoke" => Some($f::<ModelInvokeArgs>()),
            "mcp_call" => Some($f::<McpCallArgs>()),
            "dbx_http" => Some($f::<DbxHttpArgs>()),
            "mem_local_exec" | "mem_remote_exec" => Some($f::<MemExecArgs>()),
            "config_read" | "config_delete" => Some($f::<ConfigIdArgs>()),
            "config_write" => Some($f::<ConfigWriteArgs>()),
            "hotkeys_set" => Some($f::<HotkeysSetArgs>()),
            "window_mog_open" => Some($f::<WindowMogOpenArgs>()),
            "view_emit" => Some($f::<ViewEmitArgs>()),
            "audit_append" => Some($f::<AuditAppendArgs>()),
            "telemetry_span" => Some($f::<TelemetrySpanArgs>()),
            "brain_subscribe" => Some($f::<BrainSubscribeArgs>()),
            _ => None,
        }
    };
}

fn schema_of<T: JsonSchema>() -> Schema {
    schemars::generate::SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<T>()
}

/// The JSON Schema of the arguments of every command.
pub fn schemas() -> BTreeMap<&'static str, Value> {
    COMMANDS
        .iter()
        .map(|c| {
            let schema = for_command!(c.name, schema_of).expect("every row has an argument type");
            (c.name, schema.to_value())
        })
        .collect()
}

/// The committed schema document: the table, the events and the argument schemas.
pub fn schema_document() -> Value {
    serde_json::json!({
        "$comment": "Generated from mog_core::ipc (#223). Do not edit; rerun the tests with BOTTLE_UPDATE_SCHEMAS=1.",
        "commands": COMMANDS,
        "events": events::ALL,
        "arguments": schemas(),
    })
}

/// The fixed reason for arguments that do not parse. serde messages quote the bad value or
/// key (for example "unknown variant `dapi...`"), so they never leave the gateway (4.6 rule 7).
pub const INVALID_ARGS_REASON: &str = "the arguments do not match the command schema";

fn parse<T: for<'de> Deserialize<'de>>(command: &str, args: Value) -> Result<T, IpcError> {
    serde_json::from_value(args).map_err(|_| IpcError::InvalidArgs {
        command: command.to_owned(),
        reason: INVALID_ARGS_REASON.to_owned(),
    })
}

fn check<T: for<'de> Deserialize<'de>>() -> fn(&str, Value) -> Result<(), IpcError> {
    |command, args| parse::<T>(command, args).map(|_| ())
}

/// The single entry for every command (R12). Checks the window, then the arguments.
pub fn handle(window_label: &str, command_name: &str, args: Value) -> Result<Handled, IpcError> {
    let spec = command(command_name).ok_or_else(|| IpcError::UnknownCommand {
        command: command_name.to_owned(),
    })?;
    let not_allowed = || IpcError::WindowNotAllowed {
        command: spec.name.to_owned(),
        window: window_label.to_owned(),
    };
    let (class, own_id) = WindowClass::of_label(window_label).ok_or_else(not_allowed)?;
    if !spec.windows.contains(&class) {
        return Err(not_allowed());
    }
    let validate = for_command!(spec.name, check).expect("every row has an argument type");
    validate(spec.name, args.clone())?;
    match spec.name {
        "cli_install" => {
            let a: CliInstallArgs = parse(spec.name, args)?;
            if !a.consent {
                return Err(IpcError::InvalidArgs {
                    command: spec.name.to_owned(),
                    reason: "consent must be true".to_owned(),
                });
            }
        }
        "view_emit" => {
            let a: ViewEmitArgs = parse(spec.name, args)?;
            // A MOG or card view may only speak for its own MOG.
            if let Some(id) = own_id {
                if a.mog_id != id {
                    return Err(IpcError::InvalidArgs {
                        command: spec.name.to_owned(),
                        reason: "mogId does not match the sending window".to_owned(),
                    });
                }
            }
            return Ok(Handled::RelayToBrain {
                from: window_label.to_owned(),
            });
        }
        "brain_subscribe" => {
            let a: BrainSubscribeArgs = parse(spec.name, args)?;
            return Ok(Handled::SubscribeBrain {
                channel: a.on_event,
            });
        }
        "config_write" => {
            // TODO(#25): reject token-like values and secret-named keys (MB-02, 4.5).
        }
        _ => {}
    }
    // Each feature issue replaces this with a call into its R1-R17 component (stubs from #225).
    Err(IpcError::NotImplemented {
        command: spec.name.to_owned(),
    })
}
