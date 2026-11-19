//! R17 BottleMcpServer (section 4.3, #113): the `bottle-mcp` loopback MCP server of one agent
//! session (RTR-25, RTR-26, RTR-29e, ARC-6, ARC-7, 4.6 rule 9).
//!
//! Responsibilities: The `bottle-mcp` server on loopback: `transcript.search`, `memory.recall`,
//! `memory.write` (through R10), and the UI tools `show_blurb`, `set_mood`, `propose_page`,
//! `ask_user` (relayed to the brain as `agent.tool_call`). No tool starts capture, approves
//! context, publishes or switches the workspace.
//!
//! The server is a [`LoopbackHandler`] on the `bottle-mcp` endpoint of a [`LoopbackSession`]
//! (#224). The listener binds `127.0.0.1:0`, checks the `Host` header and the per-session
//! `Authorization: Bearer <secret>` (constant time) before any handler code runs, caps the open
//! connections ([`crate::loopback::MAX_CONNECTIONS`]) and the body (1 MiB), and scrubs every
//! reply with the rule-7 scrubber. Only [`crate::agent_host::SharedAgentHost`] gets the URL and
//! the secret, and it sends them to the harness child in the `open_session` frame on stdin.
//!
//! It speaks a minimal MCP JSON-RPC 2.0 over `POST /mcp`: `initialize`,
//! `notifications/initialized`, `ping`, `tools/list` and `tools/call`. A call of any tool that is
//! not one of the 7 is a JSON-RPC error, and nothing runs. Nothing goes to Genie from the
//! `bottle-mcp` endpoint, and the server writes no log line: no request or reply content is kept.
//!
//! RTR-27 (#139): with [`BottleMcpDeps::with_genie_proxy`], the `genie-one-mcp` listener of the
//! same session serves the R8 Genie One MCP proxy ([`crate::mcp_transport::proxy`]), with the same
//! per-session secret. Without it that listener answers 501.
//!
//! RTR-30 (#141, ADR 0085): each call of one of the 7 tools writes one content-free
//! `agent_action:<tool>` entry ([`crate::agent_guard::AgentActionLog`]) before anything runs:
//! the MOG, the tool, the time and the argument size. A call with bad arguments is logged too
//! (it is a decision of the agent), and runs nothing. If the entry cannot be written, nothing
//! runs and the agent gets [`crate::agent_guard::AUDIT_UNAVAILABLE`] (fail closed).
//!
//! TODO(#111): SP-19 fixes the MCP wire details (streamable HTTP, SSE, session header).

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use crate::agent_guard::{AgentActionLog, AuditActionLog, OutboundApprovals, AUDIT_UNAVAILABLE};
use crate::audit::{AuditLog, AuditRetention, MemoryAuditStore};
use crate::clock::SystemWallClock;
use crate::error::{CoreError, CoreResult};
use crate::ipc::{events, AgentToolResultArgs};
use crate::local_store::LocalStore;
use crate::loopback::{
    HarnessConfig, LoopbackEndpoint, LoopbackHandler, LoopbackRequest, LoopbackResponse,
    LoopbackSession, NotImplementedHandler,
};
use crate::mcp_transport::proxy::{GenieProxy, GenieProxyDeps, PROXY_PATH};
use crate::token_guard::scrub_json;

const ID: &str = "R17";

/// The tools of `bottle-mcp`, as the 4.3 row lists them (RTR-26), in `tools/list` order.
pub const TOOLS: [&str; 7] = [
    "transcript.search",
    "memory.recall",
    "memory.write",
    "show_blurb",
    "set_mood",
    "propose_page",
    "ask_user",
];

/// The UI tools that go to the brain as `agent.tool_call`.
pub const UI_TOOLS: [&str; 4] = ["show_blurb", "set_mood", "propose_page", "ask_user"];

/// The HTTP path of the MCP endpoint.
pub const MCP_PATH: &str = "/mcp";

/// The MCP protocol version that `initialize` answers.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// How long a UI tool call waits for `agent_tool_result`. `ask_user` waits for a person, so
/// the bound is generous.
pub const RELAY_TIMEOUT: Duration = Duration::from_secs(300);

/// A waiting UI tool call wakes at least this often to see whether the server stopped.
const STOP_POLL: Duration = Duration::from_millis(50);

/// The moods that the agent may set. `listening` is not one: it tells the user that the mic is
/// open, and the agent cannot open the mic. `sleeping` is a resident state of the shell.
pub const AGENT_MOODS: [&str; 5] = ["idle", "processing", "answering", "success", "error"];

/// The memory item types that the agent may write. `genie_answer` rows come from Genie only.
pub const AGENT_ITEM_TYPES: [&str; 4] = ["decision", "fact", "entity", "open_question"];

const DEFAULT_LIMIT: u32 = 10;
const MAX_LIMIT: u32 = 50;

// ---------- the seams ----------

/// The R10 side of the local tools. Every call is for the MOG of the session, never for a MOG
/// that the harness names.
pub trait LocalTools: Send + Sync {
    /// `transcript.search`: transcript text of `mog_id` that matches `query`.
    fn transcript_search(&self, mog_id: &str, query: &str, limit: u32) -> CoreResult<Value>;
    /// `memory.recall`: memory items of `mog_id` that match `query`.
    fn memory_recall(&self, mog_id: &str, query: &str, limit: u32) -> CoreResult<Value>;
    /// `memory.write`: add one memory item of `item_type` to `mog_id`.
    fn memory_write(&self, mog_id: &str, item_type: &str, text: &str) -> CoreResult<Value>;
}

/// No local store is wired: every call fails with [`CoreError::NotImplemented`].
/// TODO(#563): the app host wires [`StoreLocalTools`] over the opened R10 store.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableLocalTools;

impl LocalTools for UnavailableLocalTools {
    fn transcript_search(&self, _: &str, _: &str, _: u32) -> CoreResult<Value> {
        Err(CoreError::NotImplemented { component: ID })
    }
    fn memory_recall(&self, _: &str, _: &str, _: u32) -> CoreResult<Value> {
        Err(CoreError::NotImplemented { component: ID })
    }
    fn memory_write(&self, _: &str, _: &str, _: &str) -> CoreResult<Value> {
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// The local tools over the fixed R10 statement catalog.
pub struct StoreLocalTools {
    store: Arc<dyn LocalStore>,
    device_id: String,
}

/// The confidence of a memory item that the agent writes.
pub const AGENT_CONFIDENCE: f64 = 0.8;

impl StoreLocalTools {
    /// Tools over `store`. `device_id` goes into the rows that the tools write.
    pub fn new(store: Arc<dyn LocalStore>, device_id: impl Into<String>) -> Self {
        Self {
            store,
            device_id: device_id.into(),
        }
    }
}

impl fmt::Debug for StoreLocalTools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreLocalTools").finish_non_exhaustive()
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// A random UUID v4 from the OS random source.
fn new_uuid() -> CoreResult<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|_| CoreError::Unavailable {
        component: ID,
        reason: "random_unavailable",
    })?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}

impl LocalTools for StoreLocalTools {
    fn transcript_search(&self, _: &str, _: &str, _: u32) -> CoreResult<Value> {
        // TODO(#61): the local catalog has no transcript statement yet; #61 stores transcripts.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn memory_recall(&self, mog_id: &str, query: &str, limit: u32) -> CoreResult<Value> {
        self.store.exec(
            "mem.recall.fts",
            json!({ "mogId": mog_id, "query": query, "limit": limit }),
        )
    }

    /// The agent's item goes through `mem.item.write` (#114, MEM-10a): the row and its embed job
    /// commit in one transaction, so this write never skips the outbox. R10 checks the text and
    /// stores an injection item as `quarantined` (SEC-24a), so `memory.recall` never returns it
    /// (SEC-24b). The agent never pins (MEM-28b: a pin needs a user action).
    /// TODO(#538): the agent item has no workspace host or id yet (MEM-31). It is source
    /// `agent`, so it never goes into a Genie preamble (Table G-CTX).
    fn memory_write(&self, mog_id: &str, item_type: &str, text: &str) -> CoreResult<Value> {
        let now = now_ms();
        // TODO(#195): a hybrid logical clock for `rowHlc` when the v1 sync lands; until then
        // the wall clock in milliseconds (the catalog takes either).
        self.store.exec(
            "mem.item.write",
            json!({
                "itemId": new_uuid()?,
                "mogId": mog_id,
                "itemType": item_type,
                "text": text,
                "source": "agent",
                "confidence": AGENT_CONFIDENCE,
                "seenAt": now,
                "rowHlc": now,
                "deviceId": self.device_id,
                "now": now,
            }),
        )
    }
}

/// One UI tool call for the brain: the `agent.tool_call` payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// The MOG of the session.
    pub mog_id: String,
    /// A new random ID for this call.
    pub call_id: String,
    /// One of [`UI_TOOLS`].
    pub tool: String,
    /// The checked arguments.
    pub args: Value,
}

/// Sends UI tool calls to the brain. The receiver gets the result once.
pub trait ToolRelay: Send + Sync {
    /// Send `call`. The server waits on the receiver with a bound.
    fn send(&self, call: &ToolCall) -> CoreResult<mpsc::Receiver<Value>>;
    /// The server stopped waiting for `call` (time-out or stop).
    fn cancel(&self, _call: &ToolCall) {}
}

/// No brain is wired: every UI tool call fails.
/// TODO(#563): the app host wires [`BrainRelay`] to the brain channel and `agent_tool_result`.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableRelay;

impl ToolRelay for UnavailableRelay {
    fn send(&self, _call: &ToolCall) -> CoreResult<mpsc::Receiver<Value>> {
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// Emits one core-to-brain event: the event name and its payload.
pub type EventEmit = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// The relay to the TS brain: emits `agent.tool_call {mogId, callId, tool, args}` and hands the
/// `agent_tool_result {mogId, callId, result}` of the brain to the waiting call.
pub struct BrainRelay {
    emit: EventEmit,
    pending: Mutex<HashMap<(String, String), mpsc::Sender<Value>>>,
}

impl fmt::Debug for BrainRelay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrainRelay").finish_non_exhaustive()
    }
}

impl BrainRelay {
    /// A relay that emits through `emit`.
    pub fn new(emit: EventEmit) -> Self {
        Self {
            emit,
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), mpsc::Sender<Value>>> {
        self.pending.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Hand `result` to the waiting call `call_id` of `mog_id`. False if no such call waits (an
    /// unknown, finished or timed-out call, or the call of another MOG).
    pub fn deliver(&self, mog_id: &str, call_id: &str, result: Value) -> bool {
        let tx = self
            .pending()
            .remove(&(mog_id.to_owned(), call_id.to_owned()));
        tx.is_some_and(|tx| tx.send(result).is_ok())
    }

    /// [`Self::deliver`] for the `agent_tool_result` IPC arguments.
    pub fn deliver_result(&self, a: &AgentToolResultArgs) -> bool {
        self.deliver(&a.mog_id, &a.call_id, a.result.clone())
    }
}

impl ToolRelay for BrainRelay {
    fn send(&self, call: &ToolCall) -> CoreResult<mpsc::Receiver<Value>> {
        let (tx, rx) = mpsc::channel();
        self.pending()
            .insert((call.mog_id.clone(), call.call_id.clone()), tx);
        (self.emit)(
            events::AGENT_TOOL_CALL,
            json!({
                "mogId": call.mog_id,
                "callId": call.call_id,
                "tool": call.tool,
                "args": call.args,
            }),
        );
        Ok(rx)
    }

    fn cancel(&self, call: &ToolCall) {
        self.pending()
            .remove(&(call.mog_id.clone(), call.call_id.clone()));
    }
}

/// What a `bottle-mcp` server uses.
#[derive(Clone)]
pub struct BottleMcpDeps {
    /// The local tools (R10).
    pub local: Arc<dyn LocalTools>,
    /// The relay of the UI tools to the brain.
    pub relay: Arc<dyn ToolRelay>,
    /// How long a UI tool call waits for its result.
    pub relay_timeout: Duration,
    /// The Genie One MCP proxy of the session (RTR-27), on its `genie-one-mcp` listener.
    pub genie: Option<GenieProxyDeps>,
    /// The content-free agent action log (RTR-30).
    pub audit: Arc<dyn AgentActionLog>,
    /// The approved outbound question of each MOG (RTR-29b). The AgentHost sets it on each
    /// turn; the Genie One proxy uses it.
    pub approvals: Arc<OutboundApprovals>,
}

impl fmt::Debug for BottleMcpDeps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BottleMcpDeps")
            .field("relay_timeout", &self.relay_timeout)
            .field("genie", &self.genie.is_some())
            .finish_non_exhaustive()
    }
}

impl BottleMcpDeps {
    /// Deps with the default [`RELAY_TIMEOUT`], an in-memory action log on the system clock
    /// (the #140 store until #442) and no approval.
    /// TODO(#563): the app host passes the one audit log of the process with [`Self::with_audit`].
    pub fn new(local: Arc<dyn LocalTools>, relay: Arc<dyn ToolRelay>) -> Self {
        Self {
            local,
            relay,
            relay_timeout: RELAY_TIMEOUT,
            genie: None,
            audit: Arc::new(AuditActionLog::new(
                AuditLog::new(MemoryAuditStore::default(), AuditRetention::default()),
                Arc::new(SystemWallClock),
            )),
            approvals: Arc::new(OutboundApprovals::default()),
        }
    }

    /// The same deps with the Genie One MCP proxy of each session (RTR-27).
    pub fn with_genie_proxy(mut self, genie: GenieProxyDeps) -> Self {
        self.genie = Some(genie);
        self
    }

    /// The same deps with another agent action log.
    pub fn with_audit(mut self, audit: Arc<dyn AgentActionLog>) -> Self {
        self.audit = audit;
        self
    }

    /// The same deps with a shared approval store.
    pub fn with_approvals(mut self, approvals: Arc<OutboundApprovals>) -> Self {
        self.approvals = approvals;
        self
    }

    /// The same deps with another relay timeout.
    pub fn with_relay_timeout(mut self, t: Duration) -> Self {
        self.relay_timeout = t;
        self
    }
}

// ---------- the tool registry ----------

fn str_prop(desc: &str, max: usize) -> Value {
    json!({ "type": "string", "description": desc, "minLength": 1, "maxLength": max })
}

fn limit_prop() -> Value {
    json!({ "type": "integer", "minimum": 1, "maximum": MAX_LIMIT, "default": DEFAULT_LIMIT,
            "description": "The most results to return." })
}

/// The `tools/list` entries, in [`TOOLS`] order. No name or description offers a forbidden
/// action (RTR-29e).
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "transcript.search",
            "description": "Search the local transcripts of this MOG for text. Read only.",
            "inputSchema": { "type": "object", "required": ["query"], "properties": {
                "query": str_prop("The words to look for.", 500), "limit": limit_prop() } },
        }),
        json!({
            "name": "memory.recall",
            "description": "Recall items from the local memory of this MOG that match a query. Read only.",
            "inputSchema": { "type": "object", "required": ["query"], "properties": {
                "query": str_prop("The words to look for.", 500), "limit": limit_prop() } },
        }),
        json!({
            "name": "memory.write",
            "description": "Add one item to the local memory of this MOG.",
            "inputSchema": { "type": "object", "required": ["type", "text"], "properties": {
                "type": { "type": "string", "enum": AGENT_ITEM_TYPES },
                "text": str_prop("The item text.", 600) } },
        }),
        json!({
            "name": "show_blurb",
            "description": "Show a short answer blurb on the card of this MOG.",
            "inputSchema": { "type": "object", "required": ["text"], "properties": {
                "text": str_prop("The blurb text (Markdown).", 4000),
                "title": str_prop("An optional title.", 200) } },
        }),
        json!({
            "name": "set_mood",
            "description": "Set the mood of the MOG mascot.",
            "inputSchema": { "type": "object", "required": ["mood"], "properties": {
                "mood": { "type": "string", "enum": AGENT_MOODS } } },
        }),
        json!({
            "name": "propose_page",
            "description": "Propose a draft Page definition to the user. It only proposes: the user reviews it and decides what happens next.",
            "inputSchema": { "type": "object", "required": ["title", "body"], "properties": {
                "title": str_prop("The term or title.", 200),
                "body": str_prop("The definition text (Markdown).", 20000) } },
        }),
        json!({
            "name": "ask_user",
            "description": "Ask the user one question and wait for the answer.",
            "inputSchema": { "type": "object", "required": ["question"], "properties": {
                "question": str_prop("The question.", 1000),
                "choices": { "type": "array", "maxItems": 10, "items": str_prop("A choice.", 200) } } },
        }),
    ]
}

// ---------- the handler ----------

/// Why a tool call did not give a result.
enum Fail {
    /// The arguments do not match the schema: a JSON-RPC error, nothing ran.
    BadArgs,
    /// The tool ran and failed: an MCP tool result with `isError`, and a fixed code.
    Tool(&'static str),
}

type Args = Map<String, Value>;

fn text_arg(a: &Args, key: &str, max: usize, required: bool) -> Result<Option<String>, Fail> {
    match a.get(key) {
        None | Some(Value::Null) if !required => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() && s.chars().count() <= max => {
            Ok(Some(s.clone()))
        }
        _ => Err(Fail::BadArgs),
    }
}

fn limit_arg(a: &Args) -> Result<u32, Fail> {
    match a.get("limit") {
        None | Some(Value::Null) => Ok(DEFAULT_LIMIT),
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=u64::from(MAX_LIMIT)).contains(n))
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(Fail::BadArgs),
    }
}

fn enum_arg(a: &Args, key: &str, allowed: &[&str]) -> Result<String, Fail> {
    match a.get(key).and_then(Value::as_str) {
        Some(s) if allowed.contains(&s) => Ok(s.to_owned()),
        _ => Err(Fail::BadArgs),
    }
}

/// Check the arguments of a UI tool and keep only the schema's properties.
fn ui_args(tool: &str, a: &Args) -> Result<Value, Fail> {
    let mut out = Map::new();
    let mut put = |k: &str, v: Option<String>| {
        if let Some(v) = v {
            out.insert(k.to_owned(), Value::String(v));
        }
    };
    match tool {
        "show_blurb" => {
            put("text", text_arg(a, "text", 4000, true)?);
            put("title", text_arg(a, "title", 200, false)?);
        }
        "set_mood" => put("mood", Some(enum_arg(a, "mood", &AGENT_MOODS)?)),
        "propose_page" => {
            put("title", text_arg(a, "title", 200, true)?);
            put("body", text_arg(a, "body", 20000, true)?);
        }
        "ask_user" => {
            put("question", text_arg(a, "question", 1000, true)?);
            match a.get("choices") {
                None | Some(Value::Null) => {}
                Some(Value::Array(c))
                    if c.len() <= 10
                        && c.iter().all(|x| {
                            x.as_str()
                                .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 200)
                        }) =>
                {
                    out.insert("choices".into(), Value::Array(c.clone()));
                }
                _ => return Err(Fail::BadArgs),
            }
        }
        _ => return Err(Fail::BadArgs),
    }
    Ok(Value::Object(out))
}

fn new_call_id() -> CoreResult<String> {
    Ok(format!("tc-{}", &new_uuid()?.replace('-', "")[..16]))
}

struct Handler {
    mog_id: String,
    deps: BottleMcpDeps,
    closed: Arc<AtomicBool>,
    /// The session's Genie One MCP proxy, if the deps have one.
    genie: Option<GenieProxy>,
}

fn rpc_result(id: Value, result: Value) -> LoopbackResponse {
    LoopbackResponse {
        status: 200,
        body: json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    }
}

fn rpc_error(id: Value, code: i64, message: &str) -> LoopbackResponse {
    LoopbackResponse {
        status: 200,
        body: json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    }
}

fn http_error(status: u16, code: &str) -> LoopbackResponse {
    LoopbackResponse {
        status,
        body: json!({ "error": code }),
    }
}

fn tool_ok(v: Value) -> Value {
    // Rule 7 here too, so the text copy and the structured copy are both clean.
    let v = scrub_json(&v);
    let structured = if v.is_object() {
        v.clone()
    } else {
        json!({ "result": v })
    };
    json!({
        "content": [{ "type": "text", "text": v.to_string() }],
        "structuredContent": structured,
        "isError": false,
    })
}

fn tool_error(code: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": code }], "isError": true })
}

impl Handler {
    fn local(&self, tool: &str, a: &Args) -> Result<Value, Fail> {
        let l = &self.deps.local;
        let m = &self.mog_id;
        let r = match tool {
            "transcript.search" | "memory.recall" => {
                let q = text_arg(a, "query", 500, true)?.unwrap_or_default();
                let limit = limit_arg(a)?;
                if tool == "memory.recall" {
                    l.memory_recall(m, &q, limit)
                } else {
                    l.transcript_search(m, &q, limit)
                }
            }
            "memory.write" => {
                let t = enum_arg(a, "type", &AGENT_ITEM_TYPES)?;
                let text = text_arg(a, "text", 600, true)?.unwrap_or_default();
                l.memory_write(m, &t, &text)
            }
            _ => return Err(Fail::BadArgs),
        };
        r.map_err(|e| {
            Fail::Tool(match e {
                CoreError::NotImplemented { .. } => "not_available",
                CoreError::InvalidStatementParams { .. } => "invalid_arguments",
                _ => "local_tool_failed",
            })
        })
    }

    fn relay(&self, tool: &str, a: &Args) -> Result<Value, Fail> {
        let call = ToolCall {
            mog_id: self.mog_id.clone(),
            call_id: new_call_id().map_err(|_| Fail::Tool("relay_unavailable"))?,
            tool: tool.to_owned(),
            args: ui_args(tool, a)?,
        };
        let relay = &self.deps.relay;
        let rx = relay
            .send(&call)
            .map_err(|_| Fail::Tool("relay_unavailable"))?;
        let deadline = Instant::now() + self.deps.relay_timeout;
        loop {
            if self.closed.load(Ordering::SeqCst) {
                relay.cancel(&call);
                return Err(Fail::Tool("session_closed"));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                relay.cancel(&call);
                return Err(Fail::Tool("timeout"));
            }
            match rx.recv_timeout(left.min(STOP_POLL)) {
                Ok(v) => return Ok(v),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(Fail::Tool("relay_unavailable"))
                }
            }
        }
    }

    fn tools_call(&self, id: Value, params: Option<&Value>) -> LoopbackResponse {
        let params = params.and_then(Value::as_object);
        let name = params.and_then(|p| p.get("name")).and_then(Value::as_str);
        // Only the 7 tools exist. Any other name runs nothing (RTR-29e).
        let Some(tool) = name.and_then(|n| TOOLS.iter().find(|t| **t == n)) else {
            return rpc_error(id, -32602, "unknown tool");
        };
        let empty = Map::new();
        let raw_args = params.and_then(|p| p.get("arguments"));
        // RTR-30: the decision is logged before anything runs. No entry, no action.
        let args_bytes = raw_args.map_or(0, |a| a.to_string().len() as u64);
        if self
            .deps
            .audit
            .record(&self.mog_id, tool, args_bytes)
            .is_err()
        {
            return rpc_result(id, tool_error(AUDIT_UNAVAILABLE));
        }
        let args = match raw_args {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(a)) => a,
            Some(_) => return rpc_error(id, -32602, "invalid arguments"),
        };
        let out = if UI_TOOLS.contains(tool) {
            self.relay(tool, args)
        } else {
            self.local(tool, args)
        };
        match out {
            Ok(v) => rpc_result(id, tool_ok(v)),
            Err(Fail::BadArgs) => rpc_error(id, -32602, "invalid arguments"),
            Err(Fail::Tool(code)) => rpc_result(id, tool_error(code)),
        }
    }

    fn mcp(&self, body: &[u8]) -> LoopbackResponse {
        let Ok(msg) = serde_json::from_slice::<Value>(body) else {
            return rpc_error(Value::Null, -32700, "parse error");
        };
        let Some(obj) = msg.as_object() else {
            return rpc_error(Value::Null, -32600, "invalid request");
        };
        let id = obj.get("id").cloned();
        let method = obj.get("method").and_then(Value::as_str);
        let (Some("2.0"), Some(method)) = (obj.get("jsonrpc").and_then(Value::as_str), method)
        else {
            return rpc_error(id.unwrap_or(Value::Null), -32600, "invalid request");
        };
        let Some(id) = id else {
            // A notification (`notifications/initialized` and others): accepted, no answer.
            return LoopbackResponse {
                status: 202,
                body: Value::Null,
            };
        };
        match method {
            "initialize" => rpc_result(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "bottle-mcp", "version": env!("CARGO_PKG_VERSION") },
                }),
            ),
            "ping" => rpc_result(id, json!({})),
            "tools/list" => rpc_result(id, json!({ "tools": tool_definitions() })),
            "tools/call" => self.tools_call(id, obj.get("params")),
            _ => rpc_error(id, -32601, "method not found"),
        }
    }
}

impl LoopbackHandler for Handler {
    fn handle(&self, req: &LoopbackRequest) -> LoopbackResponse {
        match (req.endpoint, &self.genie) {
            (LoopbackEndpoint::BottleMcp, _) => {}
            // RTR-27: the R8 Genie One MCP proxy of this session.
            (LoopbackEndpoint::GenieOneMcp, Some(g)) => return g.handle(req),
            // The model proxy is `agent_host::model_proxy`, on its own listener.
            _ => return NotImplementedHandler.handle(req),
        }
        if req.path != MCP_PATH {
            return http_error(404, "not_found");
        }
        if req.method != "POST" {
            return http_error(405, "method_not_allowed");
        }
        self.mcp(&req.body)
    }
}

// ---------- the server ----------

/// The `bottle-mcp` server of one agent session. It listens on `127.0.0.1` until it stops or
/// drops; then the secret opens nothing.
pub struct BottleMcpServer {
    closed: Arc<AtomicBool>,
    genie: bool,
    config: HarnessConfig,
    session: Option<LoopbackSession>,
}

impl fmt::Debug for BottleMcpServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BottleMcpServer")
            .field("url", &self.url())
            .field("genie_one_url", &self.genie_one_url())
            .field("authorization", &"<redacted>")
            .finish()
    }
}

impl BottleMcpServer {
    /// Start the server of the session of `mog_id` with a new per-session secret.
    pub fn start(mog_id: &str, deps: &BottleMcpDeps) -> CoreResult<Self> {
        let closed = Arc::new(AtomicBool::new(false));
        let genie = deps.genie.clone().map(|g| {
            GenieProxy::new(mog_id, g, closed.clone())
                .with_guard(deps.audit.clone(), deps.approvals.clone())
        });
        let has_genie = genie.is_some();
        let handler = Arc::new(Handler {
            mog_id: mog_id.to_owned(),
            deps: deps.clone(),
            closed: closed.clone(),
            genie,
        });
        let session = LoopbackSession::open(handler)?;
        Ok(Self {
            closed,
            genie: has_genie,
            config: session.harness_config(),
            session: Some(session),
        })
    }

    /// The MCP URL for the harness: `http://127.0.0.1:<port>/mcp`.
    pub fn url(&self) -> String {
        let base = self
            .config
            .endpoints()
            .iter()
            .find(|(e, _)| *e == LoopbackEndpoint::BottleMcp)
            .map(|(_, u)| u.as_str())
            .unwrap_or("");
        format!("{base}{MCP_PATH}")
    }

    /// The Genie One MCP proxy URL for the harness, `http://127.0.0.1:<port>/mcp` on the
    /// session's `genie-one-mcp` listener, if the session has the proxy (RTR-27). The same
    /// [`Self::authorization`] opens it.
    pub fn genie_one_url(&self) -> Option<String> {
        if !self.genie {
            return None;
        }
        self.config
            .endpoints()
            .iter()
            .find(|(e, _)| *e == LoopbackEndpoint::GenieOneMcp)
            .map(|(_, u)| format!("{u}{PROXY_PATH}"))
    }

    /// The `Authorization` header value for the harness: `Bearer <session secret>`.
    pub fn authorization(&self) -> &str {
        self.config.authorization()
    }

    /// The number of open connections.
    pub fn open_connections(&self) -> usize {
        self.session
            .as_ref()
            .map_or(0, LoopbackSession::open_connections)
    }

    /// Stop the listeners. A waiting UI tool call ends within [`STOP_POLL`].
    pub fn stop(self) {}
}

impl Drop for BottleMcpServer {
    fn drop(&mut self) {
        // First tell the waiting calls, so the listener's join does not wait for a relay.
        self.closed.store(true, Ordering::SeqCst);
        if let Some(s) = self.session.take() {
            s.close();
        }
    }
}
