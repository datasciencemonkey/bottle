//! Main-agent guardrails, tool decisions and the agent action log (#141, ADR 0085): RTR-03c,
//! RTR-21b, RTR-29a..d, RTR-30, AT-42, 12.0 B4 and the 9.2 action log.
//!
//! Every test uses doubles: a local-tools double, a relay double, a fake runtime process and the
//! in-memory audit store. No test starts a real harness, calls a model, reads a CLI profile or
//! leaves loopback. The adversarial set is `fixtures/at_42_adversarial.json`; its texts are data.
//! TODO(#111): the live AT-42 run with each harness (SP-19) replaces the doubles.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use mog_core::agent_guard::{
    tool_policy, AgentActionLog, AuditActionLog, OutboundApprovals, AUDIT_UNAVAILABLE,
    MAIN_AGENT_SYSTEM_PROMPT,
};
use mog_core::agent_host::model_proxy::{
    ModelProxy, ModelTarget, Upstream, UpstreamError, UpstreamRequest, UpstreamResponse,
};
use mog_core::agent_host::runtime::{
    LaunchSpec, RuntimeConfig, RuntimeEvents, RuntimeLauncher, RuntimeProcess,
};
use mog_core::agent_host::{
    AgentError, AgentHost, AgentHostDeps, AgentStatus, AgentStatusSink, Harness, ModelTargets,
    RuntimeConfigs, SharedAgentHost,
};
use mog_core::audit::{AuditEntry, AuditError, AuditLog, AuditRetention, MemoryAuditStore};
use mog_core::bottle_mcp::{
    BottleMcpDeps, BottleMcpServer, LocalTools, ToolCall, ToolRelay, MCP_PATH, TOOLS,
};
use mog_core::clock::SimWallClock;
use mog_core::error::CoreResult;
use mog_core::token_broker::{TokenBroker, TokenLease};
use serde_json::{json, Value};

/// A loaded machine can be slow; no test asserts an upper time bound.
const BOUND: Duration = Duration::from_secs(120);
const MOG: &str = "00000000-0000-4000-8000-000000000141";
const MOG_2: &str = "00000000-0000-4000-8000-000000000142";
const NOW_MS: i64 = 1_800_000_000_000;
const QUESTION: &str = "What was revenue in Q3?";

// ---------- the adversarial set ----------

struct Adversarial {
    id: String,
    asks: String,
    text: String,
}

fn adversarial() -> Vec<Adversarial> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/at_42_adversarial.json");
    let raw = std::fs::read_to_string(path).expect("the AT-42 adversarial set exists");
    let v: Value = serde_json::from_str(&raw).expect("the set is JSON");
    v["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| Adversarial {
            id: i["id"].as_str().unwrap().to_owned(),
            asks: i["asks"].as_str().unwrap().to_owned(),
            text: i["text"].as_str().unwrap().to_owned(),
        })
        .collect()
}

// ---------- doubles ----------

/// Local tools that answer with adversarial text, as a poisoned transcript or memory would.
#[derive(Default)]
struct PoisonedLocal {
    calls: AtomicUsize,
    answer: Mutex<String>,
}

impl PoisonedLocal {
    fn reply(&self) -> CoreResult<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({ "rows": [{ "text": self.answer.lock().unwrap().clone() }] }))
    }
}

impl LocalTools for PoisonedLocal {
    fn transcript_search(&self, _: &str, _: &str, _: u32) -> CoreResult<Value> {
        self.reply()
    }
    fn memory_recall(&self, _: &str, _: &str, _: u32) -> CoreResult<Value> {
        self.reply()
    }
    fn memory_write(&self, _: &str, _: &str, _: &str) -> CoreResult<Value> {
        self.reply()
    }
}

/// A relay that answers every UI tool call at once and keeps the calls.
#[derive(Default)]
struct AnsweringRelay {
    calls: Mutex<Vec<ToolCall>>,
}

impl ToolRelay for AnsweringRelay {
    fn send(&self, call: &ToolCall) -> CoreResult<mpsc::Receiver<Value>> {
        self.calls.lock().unwrap().push(call.clone());
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(json!({ "ok": true }));
        Ok(rx)
    }
}

/// An action log that always fails, as a broken store would.
struct BrokenLog;

impl AgentActionLog for BrokenLog {
    fn record(&self, _: &str, _: &str, _: u64) -> Result<(), AuditError> {
        Err(AuditError::BadMogId)
    }
}

type Log = AuditActionLog<MemoryAuditStore>;

fn log() -> Arc<Log> {
    Arc::new(AuditActionLog::new(
        AuditLog::new(MemoryAuditStore::default(), AuditRetention::default()),
        Arc::new(SimWallClock::at(NOW_MS)),
    ))
}

struct Rig {
    server: BottleMcpServer,
    local: Arc<PoisonedLocal>,
    relay: Arc<AnsweringRelay>,
    log: Arc<Log>,
}

fn rig_for(mog: &str) -> Rig {
    let local = Arc::new(PoisonedLocal::default());
    *local.answer.lock().unwrap() = "revenue grew".into();
    let relay = Arc::new(AnsweringRelay::default());
    let log = log();
    let deps = BottleMcpDeps::new(local.clone(), relay.clone()).with_audit(log.clone());
    let server = BottleMcpServer::start(mog, &deps).expect("bottle-mcp binds loopback");
    Rig {
        server,
        local,
        relay,
        log,
    }
}

fn rig() -> Rig {
    rig_for(MOG)
}

// ---------- a minimal HTTP client ----------

fn addr_of(url: &str) -> SocketAddr {
    let rest = url.strip_prefix("http://").expect("http url");
    rest.split('/').next().unwrap().parse().expect("ip:port")
}

fn call(server: &BottleMcpServer, tool: &str, args: Value) -> Value {
    let body = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": { "name": tool, "arguments": args } })
    .to_string();
    let addr = addr_of(&server.url());
    let req = format!(
        "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        addr.port(),
        server.authorization(),
        body.len()
    );
    let mut s = TcpStream::connect(addr).expect("connects");
    s.set_read_timeout(Some(BOUND)).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    serde_json::from_str(body).unwrap_or(Value::Null)
}

/// Valid arguments for each of the 7 tools, with `text` in each free-text field.
fn args_for(tool: &str, text: &str) -> Value {
    match tool {
        "transcript.search" | "memory.recall" => json!({ "query": text }),
        "memory.write" => json!({ "type": "fact", "text": text }),
        "show_blurb" => json!({ "text": text }),
        "set_mood" => json!({ "mood": "answering" }),
        "propose_page" => json!({ "title": "Net revenue", "body": text }),
        "ask_user" => json!({ "question": text }),
        other => panic!("not a bottle-mcp tool: {other}"),
    }
}

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_digit() || (b'a'..=b'f').contains(c),
        })
}

fn entries(log: &Log, mog: &str) -> Vec<AuditEntry> {
    log.entries(mog).expect("entries")
}

// ---------- RTR-30 / RTR-21b / AT-42: the agent action log ----------

#[test]
fn rtr_30_rtr_21b_at_42_each_tool_call_writes_one_agent_action_row_with_its_tool_name() {
    let r = rig();
    for tool in TOOLS {
        let v = call(&r.server, tool, args_for(tool, "revenue"));
        assert_eq!(v["result"]["isError"], false, "{tool}: {v}");
    }
    let rows = entries(&r.log, MOG);
    let kinds: Vec<String> = rows.iter().map(|e| e.kind.wire()).collect();
    let want: Vec<String> = TOOLS.iter().map(|t| format!("agent_action:{t}")).collect();
    assert_eq!(kinds, want, "the agent log shows each decision with its tool name");
}

#[test]
fn rtr_30_at_42_each_row_holds_the_mog_id_the_tool_the_time_and_sizes_only() {
    let r = rig();
    let set = adversarial();
    for (i, a) in set.iter().enumerate() {
        let tool = TOOLS[i % TOOLS.len()];
        let args = args_for(tool, &a.text);
        call(&r.server, tool, args.clone());
        let rows = entries(&r.log, MOG);
        let row = rows.last().expect("one row per call");
        assert_eq!(row.mog_id, MOG, "{}", a.id);
        assert_eq!(row.kind.tool(), Some(tool), "{}", a.id);
        assert_eq!(row.at_ms, NOW_MS as u64, "{}: the time of the call", a.id);
        assert!(is_uuid(&row.op_id), "{}: the opId is a new UUID", a.id);
        let keys: Vec<&str> = row.sizes.keys().copied().collect();
        assert_eq!(keys, ["args_bytes"], "{}: sizes only", a.id);
        assert_eq!(
            row.sizes["args_bytes"],
            args.to_string().len() as u64,
            "{}",
            a.id
        );
        assert!(row.hashes.is_empty(), "{}: no hash on an agent action", a.id);
        // No content: no word of the text, the question or the arguments is in the row.
        let stored = serde_json::to_string(row).unwrap();
        for word in a.text.split_whitespace().filter(|w| w.len() > 6) {
            assert!(!stored.contains(word), "{}: `{word}` in {stored}", a.id);
        }
    }
    let ops: BTreeSet<String> = entries(&r.log, MOG).into_iter().map(|e| e.op_id).collect();
    assert_eq!(ops.len(), set.len(), "each action has its own opId");
}

#[test]
fn rtr_30_rows_belong_to_the_session_mog_only() {
    let a = rig_for(MOG);
    let b = rig_for(MOG_2);
    call(&a.server, "memory.recall", args_for("memory.recall", "x"));
    call(&b.server, "set_mood", args_for("set_mood", ""));
    call(&b.server, "show_blurb", args_for("show_blurb", "y"));
    assert_eq!(entries(&a.log, MOG).len(), 1);
    assert!(entries(&a.log, MOG_2).is_empty());
    assert_eq!(entries(&b.log, MOG_2).len(), 2);
}

#[test]
fn rtr_30_a_call_with_bad_arguments_is_logged_and_runs_nothing() {
    let r = rig();
    let v = call(&r.server, "set_mood", json!({ "mood": "listening" }));
    assert_eq!(v["error"]["code"], -32602, "{v}");
    let rows = entries(&r.log, MOG);
    assert_eq!(rows.len(), 1, "the refused decision is in the log");
    assert_eq!(rows[0].kind.tool(), Some("set_mood"));
    assert!(r.relay.calls.lock().unwrap().is_empty());
}

#[test]
fn rtr_30_no_action_runs_when_its_row_cannot_be_written() {
    let local = Arc::new(PoisonedLocal::default());
    let relay = Arc::new(AnsweringRelay::default());
    let deps = BottleMcpDeps::new(local.clone(), relay.clone()).with_audit(Arc::new(BrokenLog));
    let server = BottleMcpServer::start(MOG, &deps).unwrap();
    for tool in TOOLS {
        let v = call(&server, tool, args_for(tool, "revenue"));
        assert_eq!(v["result"]["isError"], true, "{tool}: {v}");
        assert_eq!(v["result"]["content"][0]["text"], AUDIT_UNAVAILABLE, "{v}");
    }
    assert_eq!(local.calls.load(Ordering::SeqCst), 0, "no local tool ran");
    assert!(relay.calls.lock().unwrap().is_empty(), "nothing was relayed");
}

// ---------- RTR-29a, RTR-29c, RTR-29d: no forbidden action ----------

#[test]
fn rtr_29a_29c_29d_forbidden_tool_names_run_nothing_and_write_no_row() {
    let r = rig();
    for name in [
        "capture_start",
        "capture.start",
        "start_capture",
        "context_approve",
        "approve_context",
        "page_publish",
        "publish_page",
        "profile_select",
        "switch_workspace",
    ] {
        let v = call(&r.server, name, json!({ "mode": "session", "profile": "prod" }));
        assert_eq!(v["error"]["code"], -32602, "{name}: {v}");
    }
    assert_eq!(r.local.calls.load(Ordering::SeqCst), 0);
    assert!(r.relay.calls.lock().unwrap().is_empty());
    assert!(
        entries(&r.log, MOG).is_empty(),
        "an unknown name is not an agent tool; it runs nothing"
    );
}

// ---------- RTR-03c / AT-42: transcript and memory text is data ----------

#[test]
fn rtr_03c_at_42_adversarial_tool_results_cause_no_further_call() {
    let r = rig();
    for a in adversarial() {
        *r.local.answer.lock().unwrap() = a.text.clone();
        let before = r.local.calls.load(Ordering::SeqCst);
        let rows = entries(&r.log, MOG).len();
        for tool in ["transcript.search", "memory.recall"] {
            let v = call(&r.server, tool, args_for(tool, "revenue"));
            assert_eq!(v["result"]["isError"], false, "{}: {v}", a.id);
        }
        assert_eq!(
            r.local.calls.load(Ordering::SeqCst),
            before + 2,
            "{} ({}): only the 2 calls ran",
            a.id,
            a.asks
        );
        assert_eq!(entries(&r.log, MOG).len(), rows + 2, "{}", a.id);
        assert!(
            r.relay.calls.lock().unwrap().is_empty(),
            "{}: the text started no UI tool",
            a.id
        );
    }
}

#[test]
fn rtr_03c_rtr_21b_the_system_prompt_treats_text_as_data_and_names_the_tool_policy() {
    let p = MAIN_AGENT_SYSTEM_PROMPT.to_ascii_lowercase();
    for must in [
        "data",
        "never instructions",
        "transcript",
        "memory",
        "start capture",
        "approve",
        "publish",
        "switch",
        "genie one",
    ] {
        assert!(p.contains(must), "the prompt names `{must}`");
    }
    for tool in TOOLS {
        assert!(MAIN_AGENT_SYSTEM_PROMPT.contains(tool), "the prompt names {tool}");
    }
    let policy = tool_policy();
    assert_eq!(policy["mcpServers"], json!(["bottle-mcp", "genie-one-mcp"]));
    assert_eq!(policy["builtinTools"], "none", "no shell, file or web tool");
    assert_eq!(policy["tools"], json!(TOOLS));
}

// ---------- the AgentHost: open frame and approvals ----------

struct FakeTokens;

impl TokenBroker for FakeTokens {
    fn with_token(&self, _: &str, _: &str, f: &mut dyn FnMut(&[u8])) -> CoreResult<TokenLease> {
        f(b"test-token-not-real-141");
        Ok(TokenLease::default())
    }
    fn invalidate(&self, _: &str, _: TokenLease) -> CoreResult<()> {
        Ok(())
    }
    fn workspace_id(&self, _: &str) -> CoreResult<String> {
        Ok("123".into())
    }
}

struct NoUpstream;

impl Upstream for NoUpstream {
    fn forward(&self, _: &UpstreamRequest<'_>) -> Result<UpstreamResponse, UpstreamError> {
        Err(UpstreamError)
    }
}

#[derive(Default)]
struct FakeProc {
    frames: Mutex<Vec<Value>>,
}

struct FakeProcHandle(Arc<FakeProc>);

impl RuntimeProcess for FakeProcHandle {
    fn send_frame(&self, frame: &Value) -> Result<(), AgentError> {
        self.0.frames.lock().unwrap().push(frame.clone());
        Ok(())
    }
    fn stop(&self) {}
}

#[derive(Default)]
struct FakeLauncher {
    procs: Mutex<Vec<Arc<FakeProc>>>,
}

impl RuntimeLauncher for FakeLauncher {
    fn launch(
        &self,
        _spec: &LaunchSpec,
        _events: RuntimeEvents,
    ) -> Result<Box<dyn RuntimeProcess>, AgentError> {
        let p = Arc::new(FakeProc::default());
        self.procs.lock().unwrap().push(p.clone());
        Ok(Box::new(FakeProcHandle(p)))
    }
}

struct NoStatus;

impl AgentStatusSink for NoStatus {
    fn status(&self, _: &AgentStatus) {}
}

struct FakeTargets;

impl ModelTargets for FakeTargets {
    fn target(&self, _: &str) -> Result<ModelTarget, AgentError> {
        Ok(ModelTarget::Gateway {
            profile: "bottle-home".into(),
            host: "example.cloud.databricks.com".into(),
        })
    }
}

struct HostRig {
    host: Arc<SharedAgentHost>,
    launcher: Arc<FakeLauncher>,
    approvals: Arc<OutboundApprovals>,
}

fn host_rig() -> HostRig {
    let launcher = Arc::new(FakeLauncher::default());
    let approvals = Arc::new(OutboundApprovals::default());
    let proxy = Arc::new(ModelProxy::start(Arc::new(FakeTokens), Arc::new(NoUpstream)).unwrap());
    let mcp = BottleMcpDeps::new(
        Arc::new(PoisonedLocal::default()),
        Arc::new(AnsweringRelay::default()),
    )
    .with_audit(log())
    .with_approvals(approvals.clone());
    let host = SharedAgentHost::new(AgentHostDeps {
        bottle_mcp: mcp,
        launcher: launcher.clone(),
        proxy,
        targets: Arc::new(FakeTargets),
        status: Arc::new(NoStatus),
        runtimes: RuntimeConfigs {
            claude: RuntimeConfig::new("fake-claude-runtime", Vec::new()),
            codex: RuntimeConfig::new("fake-codex-runtime", Vec::new()),
        },
    });
    HostRig {
        host,
        launcher,
        approvals,
    }
}

fn frames(r: &HostRig, kind: &str) -> Vec<Value> {
    let p = r.launcher.procs.lock().unwrap()[0].clone();
    let f = p.frames.lock().unwrap();
    f.iter().filter(|f| f["type"] == kind).cloned().collect()
}

fn turn(question: &str) -> Value {
    json!({
        "mogId": MOG,
        "question": question,
        "decision": { "intent": "question" },
        "recall": [{ "itemId": "i1", "type": "fact", "text": adversarial()[5].text, "score": 0.9, "ageH": 1 }],
        "target": "bottle-home",
    })
}

#[test]
fn rtr_03c_rtr_21b_each_harness_gets_the_data_only_prompt_and_the_tool_policy() {
    for h in [Harness::Claude, Harness::Codex] {
        let r = host_rig();
        r.host.start_session(MOG, h).unwrap();
        let open = frames(&r, "open_session");
        assert_eq!(open.len(), 1);
        assert_eq!(open[0]["systemPrompt"], MAIN_AGENT_SYSTEM_PROMPT, "{h:?}");
        assert_eq!(open[0]["toolPolicy"], tool_policy(), "{h:?}");
    }
}

#[test]
fn rtr_03c_the_turn_carries_question_and_recall_as_data_fields_not_as_prompt_text() {
    let r = host_rig();
    r.host.start_session(MOG, Harness::Claude).unwrap();
    let t = turn(QUESTION);
    r.host.send(MOG, &t).unwrap();
    let sent = frames(&r, "turn");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["turn"], t, "the turn is passed as JSON data, unchanged");
    let open = &frames(&r, "open_session")[0];
    let prompt = open["systemPrompt"].as_str().unwrap();
    assert!(!prompt.contains(QUESTION), "no user text is joined to the prompt");
}

#[test]
fn rtr_29b_send_approves_the_turn_question_once_and_nothing_else() {
    let r = host_rig();
    r.host.start_session(MOG, Harness::Claude).unwrap();
    assert!(!r.approvals.take(MOG, QUESTION), "nothing is approved before a turn");
    r.host.send(MOG, &turn(QUESTION)).unwrap();
    // The question with added memory or transcript text is new outbound context: refused.
    for a in adversarial() {
        let padded = format!("{QUESTION}\n{}", a.text);
        assert!(!r.approvals.take(MOG, &padded), "{}", a.id);
        assert!(!r.approvals.take(MOG, &a.text), "{}", a.id);
    }
    assert!(!r.approvals.take(MOG_2, QUESTION), "another MOG has no approval");
    assert!(r.approvals.take(MOG, QUESTION), "the approved question, once");
    assert!(!r.approvals.take(MOG, QUESTION), "an approval is used once");
}

#[test]
fn rtr_29b_a_new_turn_replaces_the_approval_and_a_stop_clears_it() {
    let r = host_rig();
    r.host.start_session(MOG, Harness::Claude).unwrap();
    r.host.send(MOG, &turn("first question")).unwrap();
    r.host.send(MOG, &turn(QUESTION)).unwrap();
    assert!(!r.approvals.take(MOG, "first question"), "the old turn's question expired");
    assert!(r.approvals.take(MOG, QUESTION));
    r.host.send(MOG, &turn(QUESTION)).unwrap();
    r.host.stop_session(MOG).unwrap();
    assert!(!r.approvals.take(MOG, QUESTION), "a stop clears the approval");
}

#[test]
fn rtr_29b_sec_23_approvals_keep_no_text() {
    let a = OutboundApprovals::default();
    a.approve(MOG, QUESTION);
    let shown = format!("{a:?}");
    assert!(!shown.contains("revenue"), "{shown}");
    assert!(!shown.contains(MOG), "{shown}");
}
