//! The core half of the #142 preview gate (ADR 0086), on the #141 approval path (ADR 0085).
//!
//! The brain sends `agent_send` only when the 1.5 s preview ends with no cancel, or on a chip
//! click (`src/brain/ask-gate-wiring.test.ts` proves no `agent_send` before 1.5 s or after a
//! cancel). Its arguments are `fixtures/preview_gate_agent_send.json`; the TS test checks that
//! the gate sends exactly that file. Here the same file goes through the IPC argument check and
//! `AgentHost::send`, and the test checks that its question is the one approval that the Genie
//! proxy takes for `genie_ask` (RTR-29b): once, and no other text.
//!
//! #307: the fixture goes through the hardened gateway and `AgentComponent` to `AgentHost::send`.
//! Every part is a double: no harness starts, no model runs, no profile is read.

use std::sync::{mpsc, Arc, Mutex};

use mog_core::agent_guard::OutboundApprovals;
use mog_core::agent_host::ipc_route::AgentComponent;
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
use mog_core::bottle_mcp::{BottleMcpDeps, ToolCall, ToolRelay, UnavailableLocalTools};
use mog_core::error::CoreResult;
use mog_core::ipc_gateway::{hardened_gateway_with, IpcGateway, NoLog};
use mog_core::token_broker::{TokenBroker, TokenLease};
use serde_json::Value;

struct FakeTokens;

impl TokenBroker for FakeTokens {
    fn with_token(&self, _: &str, _: &str, f: &mut dyn FnMut(&[u8])) -> CoreResult<TokenLease> {
        f(b"test-token-not-real-142");
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

struct NoRelay;

impl ToolRelay for NoRelay {
    fn send(&self, _: &ToolCall) -> CoreResult<mpsc::Receiver<Value>> {
        let (_tx, rx) = mpsc::channel();
        Ok(rx)
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

struct Rig {
    host: Arc<SharedAgentHost>,
    launcher: Arc<FakeLauncher>,
    approvals: Arc<OutboundApprovals>,
}

fn rig() -> Rig {
    let launcher = Arc::new(FakeLauncher::default());
    let approvals = Arc::new(OutboundApprovals::default());
    let proxy = Arc::new(ModelProxy::start(Arc::new(FakeTokens), Arc::new(NoUpstream)).unwrap());
    let mcp = BottleMcpDeps::new(Arc::new(UnavailableLocalTools), Arc::new(NoRelay))
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
    Rig {
        host,
        launcher,
        approvals,
    }
}

/// The `agent_send` arguments that the brain's ask gate sends after the preview.
fn fixture() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/preview_gate_agent_send.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("JSON")
}

fn mog_and_question(args: &Value) -> (String, String) {
    (
        args["mogId"].as_str().expect("mogId").to_owned(),
        args["turn"]["question"]
            .as_str()
            .expect("question")
            .to_owned(),
    )
}

/// The #307 route: the hardened gateway, then `AgentComponent` and `AgentHost::send`.
fn agent_send(r: &Rig, args: &Value) {
    let reply = hardened_gateway_with(NoLog, AgentComponent::new(r.host.clone())).dispatch(
        "brain",
        "agent_send",
        args.clone(),
    );
    assert_eq!(
        reply.ok(),
        Some(serde_json::json!({})),
        "the session takes the turn"
    );
}

#[test]
fn rtr_04b_rtr_29b_the_previewed_question_is_the_one_genie_ask_may_send_once() {
    let r = rig();
    let args = fixture();
    let (mog, question) = mog_and_question(&args);
    r.host.start_session(&mog, Harness::Claude).unwrap();
    agent_send(&r, &args);
    // The turn reached the session with the question as a data field.
    let p = r.launcher.procs.lock().unwrap()[0].clone();
    let turns: Vec<Value> = p
        .frames
        .lock()
        .unwrap()
        .iter()
        .filter(|f| f["type"] == "turn")
        .cloned()
        .collect();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0]["turn"]["question"], question.as_str());
    // The proxy takes the approval for `genie_ask` (proxy.rs): only this text, only once.
    assert!(!r.approvals.take(&mog, "What were APAC bookings last week?"));
    assert!(!r.approvals.take(&mog, &format!("{question} ")));
    assert!(
        r.approvals.take(&mog, &question),
        "the previewed question is approved"
    );
    assert!(
        !r.approvals.take(&mog, &question),
        "an approval is used once"
    );
}

#[test]
fn rtr_04b_rtr_29b_with_no_agent_send_nothing_is_approved() {
    // Before the preview ends, after a cancel, and without a chip click the brain sends no
    // `agent_send`, so the session is open but holds no approval: `genie_ask` sends nothing.
    let r = rig();
    let (mog, question) = mog_and_question(&fixture());
    r.host.start_session(&mog, Harness::Claude).unwrap();
    assert!(!r.approvals.take(&mog, &question));
}

#[test]
fn rtr_04b_rtr_29b_a_stop_after_the_hand_off_clears_the_approval() {
    let r = rig();
    let args = fixture();
    let (mog, question) = mog_and_question(&args);
    r.host.start_session(&mog, Harness::Claude).unwrap();
    agent_send(&r, &args);
    r.host.stop_session(&mog).unwrap();
    assert!(!r.approvals.take(&mog, &question));
}
