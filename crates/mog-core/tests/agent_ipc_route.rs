//! #307: the IPC route of `agent_start`, `agent_stop` and `agent_send` to the one R16
//! `SharedAgentHost` (RTR-21a, RTR-22, ARC-7), the `AgentError` codes that cross IPC (ARC-F14,
//! RTR-15b) and the content-free `agent.status` event (SEC-03, SEC-16c).
//!
//! Every part is a double: no harness starts, no model runs, no profile is read, and no token
//! exists. The question text is a plainly fake marker.

use std::sync::{Arc, Mutex};

use mog_core::agent_host::ipc_route::{agent_ipc_error, AgentComponent, BrainStatusSink};
use mog_core::agent_host::model_proxy::{
    ModelProxy, ModelTarget, Upstream, UpstreamError, UpstreamRequest, UpstreamResponse,
};
use mog_core::agent_host::runtime::{
    LaunchSpec, RuntimeConfig, RuntimeEvent, RuntimeEvents, RuntimeLauncher, RuntimeProcess,
};
use mog_core::agent_host::{
    AgentError, AgentHostDeps, AgentStatus, AgentStatusSink, Harness, ModelTargets,
    RuntimeConfigs, SessionState, SharedAgentHost,
};
use mog_core::bottle_mcp::{BottleMcpDeps, UnavailableLocalTools, UnavailableRelay};
use mog_core::error::CoreError;
use mog_core::ipc::{self, AgentSendArgs, Handled, IpcError};
use mog_core::ipc_gateway::{hardened_gateway_with, ipc_error_code, IpcGateway, NoLog, ERROR_CODES};
use mog_core::token_broker::StubTokenBroker;
use serde_json::{json, Value};

/// A plainly fake question. It must never show in an error, a status or a `Debug` dump.
const QUESTION: &str = "test-question-not-real";
const MOG: &str = "00000000-0000-4000-8000-0000000003a1";
const MOG_B: &str = "00000000-0000-4000-8000-0000000003b2";

// ---------- doubles ----------

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
    fail: bool,
    procs: Mutex<Vec<(Harness, Arc<FakeProc>, RuntimeEvents)>>,
}

impl RuntimeLauncher for FakeLauncher {
    fn launch(
        &self,
        spec: &LaunchSpec,
        events: RuntimeEvents,
    ) -> Result<Box<dyn RuntimeProcess>, AgentError> {
        if self.fail {
            return Err(AgentError::LaunchFailed);
        }
        let p = Arc::new(FakeProc::default());
        self.procs
            .lock()
            .unwrap()
            .push((spec.harness, p.clone(), events));
        Ok(Box::new(FakeProcHandle(p)))
    }
}

impl FakeLauncher {
    fn proc(&self, i: usize) -> Arc<FakeProc> {
        self.procs.lock().unwrap()[i].1.clone()
    }
    fn harness(&self, i: usize) -> Harness {
        self.procs.lock().unwrap()[i].0
    }
    fn fire(&self, i: usize, ev: RuntimeEvent) {
        let events = self.procs.lock().unwrap()[i].2.clone();
        events(ev);
    }
}

#[derive(Default)]
struct FakeStatus(Mutex<Vec<AgentStatus>>);

impl AgentStatusSink for FakeStatus {
    fn status(&self, s: &AgentStatus) {
        self.0.lock().unwrap().push(s.clone());
    }
}

/// `no-home` has no target; `bad-target` names a host with a path, which the proxy refuses.
struct FakeTargets;

impl ModelTargets for FakeTargets {
    fn target(&self, mog_id: &str) -> Result<ModelTarget, AgentError> {
        match mog_id {
            "no-home" => Err(AgentError::TargetUnavailable),
            "bad-target" => Ok(ModelTarget::Gateway {
                profile: "bottle-home".into(),
                host: "example.cloud.databricks.com/not-a-bare-host".into(),
            }),
            _ => Ok(ModelTarget::Gateway {
                profile: "bottle-home".into(),
                host: "example.cloud.databricks.com".into(),
            }),
        }
    }
}

struct Rig {
    host: Arc<SharedAgentHost>,
    launcher: Arc<FakeLauncher>,
    component: AgentComponent,
}

fn rig_with(launcher: FakeLauncher, status: Arc<dyn AgentStatusSink>) -> Rig {
    let launcher = Arc::new(launcher);
    let proxy = Arc::new(
        ModelProxy::start(Arc::new(StubTokenBroker), Arc::new(NoUpstream))
            .expect("proxy binds loopback"),
    );
    let host = SharedAgentHost::new(AgentHostDeps {
        bottle_mcp: BottleMcpDeps::new(Arc::new(UnavailableLocalTools), Arc::new(UnavailableRelay)),
        launcher: launcher.clone(),
        proxy,
        targets: Arc::new(FakeTargets),
        status,
        runtimes: RuntimeConfigs {
            claude: RuntimeConfig::new("fake-claude-runtime", Vec::new()),
            codex: RuntimeConfig::new("fake-codex-runtime", Vec::new()),
        },
    });
    Rig {
        component: AgentComponent::new(host.clone()),
        host,
        launcher,
    }
}

fn rig() -> Rig {
    rig_with(FakeLauncher::default(), Arc::new(FakeStatus::default()))
}

/// One call through the hardened gateway over the agent component, from window `from`.
fn call(r: &Rig, from: &str, command: &str, args: Value) -> Result<Value, String> {
    hardened_gateway_with(NoLog, r.component.clone())
        .dispatch(from, command, args)
        .map_err(|e| match e {
            CoreError::IpcRefused { code } => code.to_owned(),
            other => panic!("not a content-free refusal: {other:?}"),
        })
}

fn start(r: &Rig, mog: &str, harness: &str) -> Result<Value, String> {
    call(
        r,
        "brain",
        "agent_start",
        json!({"mogId": mog, "harness": harness}),
    )
}

fn turn_args(mog: &str) -> Value {
    json!({"mogId": mog, "turn": {"mogId": mog, "question": QUESTION}})
}

fn frames_of(p: &FakeProc, kind: &str) -> Vec<Value> {
    p.frames
        .lock()
        .unwrap()
        .iter()
        .filter(|f| f["type"] == kind)
        .cloned()
        .collect()
}

// ---------- the contract: `handle` routes the three commands ----------

#[test]
fn rtr_21a_arc_7_handle_routes_the_agent_commands_to_a_component() {
    for (command, args) in [
        ("agent_start", json!({"mogId": MOG, "harness": "claude"})),
        ("agent_stop", json!({"mogId": MOG})),
        ("agent_send", turn_args(MOG)),
    ] {
        assert_eq!(
            ipc::handle("brain", command, args),
            Ok(Handled::Component),
            "{command} must reach the AgentHost, not not_implemented"
        );
    }
}

#[test]
fn arc_4_only_the_brain_may_call_the_agent_commands() {
    let r = rig();
    for from in ["hub", "mog-a1", "card-a1"] {
        for (command, args) in [
            ("agent_start", json!({"mogId": MOG, "harness": "claude"})),
            ("agent_stop", json!({"mogId": MOG})),
            ("agent_send", turn_args(MOG)),
        ] {
            assert_eq!(
                call(&r, from, command, args),
                Err("window_not_allowed".to_owned()),
                "{from} {command}"
            );
        }
    }
    assert_eq!(r.host.session_count(), 0, "no view window started a session");
}

// ---------- the route ----------

#[test]
fn rtr_21a_agent_start_returns_the_agent_session_id_and_one_runtime_serves_three_mogs() {
    let r = rig();
    let a = start(&r, MOG, "claude").unwrap();
    let id = a["agentSessionId"].as_str().expect("agentSessionId").to_owned();
    assert!(id.starts_with("as-"), "{id}");
    assert_eq!(a, json!({"agentSessionId": id}), "only the session ID");
    // A second start of the same MOG gives the same session.
    assert_eq!(start(&r, MOG, "claude").unwrap(), a);
    start(&r, MOG_B, "claude").unwrap();
    start(&r, "00000000-0000-4000-8000-0000000003c3", "claude").unwrap();
    assert_eq!(r.host.runtime_count(), 1, "one shared runtime (U11)");
    assert_eq!(r.host.session_count(), 3, "one session per MOG");
}

#[test]
fn rtr_22_agent_start_takes_both_harness_kinds_and_the_model() {
    let r = rig();
    start(&r, MOG, "claude").unwrap();
    call(
        &r,
        "brain",
        "agent_start",
        json!({"mogId": MOG_B, "harness": "codex", "model": "fake-model-not-real"}),
    )
    .unwrap();
    assert_eq!(r.host.runtime_count(), 2);
    assert_eq!(r.launcher.harness(0), Harness::Claude);
    assert_eq!(r.launcher.harness(1), Harness::Codex);
    let open = frames_of(&r.launcher.proc(1), "open_session");
    assert_eq!(open.len(), 1);
    assert!(
        open[0].to_string().contains("fake-model-not-real"),
        "the model reaches the session"
    );
    // The other harness kind for a MOG with a live session is refused with its own code.
    assert_eq!(
        start(&r, MOG, "codex"),
        Err("agent_harness_mismatch".to_owned())
    );
}

#[test]
fn rtr_21a_agent_send_delivers_the_turn_and_agent_stop_ends_the_session() {
    let r = rig();
    start(&r, MOG, "claude").unwrap();
    assert_eq!(call(&r, "brain", "agent_send", turn_args(MOG)), Ok(json!({})));
    let turns = frames_of(&r.launcher.proc(0), "turn");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0]["turn"]["question"], QUESTION);
    assert_eq!(
        call(&r, "brain", "agent_stop", json!({"mogId": MOG})),
        Ok(json!({}))
    );
    assert_eq!(r.host.session_count(), 0);
    assert_eq!(frames_of(&r.launcher.proc(0), "close_session").len(), 1);
    // After the stop the MOG has no session.
    assert_eq!(
        call(&r, "brain", "agent_send", turn_args(MOG)),
        Err("agent_unknown_mog".to_owned())
    );
    assert_eq!(
        call(&r, "brain", "agent_stop", json!({"mogId": MOG})),
        Err("agent_unknown_mog".to_owned())
    );
}

// ---------- the error codes ----------

#[test]
fn arc_f14_rtr_15b_every_agent_error_crosses_ipc_as_its_own_code() {
    for e in [
        AgentError::UnknownMog,
        AgentError::Unavailable,
        AgentError::LaunchFailed,
        AgentError::TargetUnavailable,
        AgentError::InvalidTarget,
        AgentError::HarnessMismatch,
    ] {
        let code = ipc_error_code(&agent_ipc_error("agent_send", e));
        assert_eq!(code, e.code(), "{e:?} keeps its code");
        assert!(ERROR_CODES.contains(&code), "{code} is in ERROR_CODES");
    }
    for code in [
        "agent_unknown_mog",
        "agent_unavailable",
        "agent_launch_failed",
        "agent_target_unavailable",
        "agent_invalid_target",
    ] {
        assert!(ERROR_CODES.contains(&code), "{code}");
    }
}

#[test]
fn arc_f14_rtr_15b_a_failed_session_answers_agent_unavailable_to_the_brain() {
    let status = Arc::new(FakeStatus::default());
    let r = rig_with(FakeLauncher::default(), status.clone());
    let id = start(&r, MOG, "claude").unwrap()["agentSessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    // ARC-F14: the first failure restarts the session once; the turn still goes.
    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: id.clone(),
        },
    );
    assert_eq!(r.host.session_state(MOG), Some(SessionState::Restarted));
    assert_eq!(call(&r, "brain", "agent_send", turn_args(MOG)), Ok(json!({})));
    // The second failure: the brain gets `agent_unavailable` and routes with the fallback
    // classifier (RTR-15b).
    r.launcher
        .fire(0, RuntimeEvent::SessionFailed { session_id: id });
    assert_eq!(r.host.session_state(MOG), Some(SessionState::Failed));
    assert_eq!(
        call(&r, "brain", "agent_send", turn_args(MOG)),
        Err("agent_unavailable".to_owned())
    );
    let states: Vec<SessionState> = status.0.lock().unwrap().iter().map(|s| s.state).collect();
    assert_eq!(states, vec![SessionState::Restarted, SessionState::Failed]);
}

#[test]
fn arc_7_start_failures_cross_as_content_free_codes() {
    let r = rig();
    assert_eq!(
        start(&r, "no-home", "claude"),
        Err("agent_target_unavailable".to_owned())
    );
    assert_eq!(
        start(&r, "bad-target", "claude"),
        Err("agent_invalid_target".to_owned())
    );
    let failing = rig_with(
        FakeLauncher {
            fail: true,
            ..FakeLauncher::default()
        },
        Arc::new(FakeStatus::default()),
    );
    assert_eq!(
        start(&failing, MOG, "claude"),
        Err("agent_launch_failed".to_owned())
    );
}

#[test]
fn sec_03_sec_16c_an_agent_error_carries_no_turn_text() {
    let r = rig();
    // No session: the refusal names a code only, never the question.
    let err = hardened_gateway_with(NoLog, r.component.clone())
        .dispatch("brain", "agent_send", turn_args(MOG))
        .unwrap_err();
    for text in [format!("{err:?}"), format!("{err:#?}"), err.to_string()] {
        assert!(!text.contains(QUESTION), "{text}");
    }
    let e = agent_ipc_error("agent_send", AgentError::Unavailable);
    let wire = serde_json::to_string(&e).unwrap();
    assert!(!wire.contains(QUESTION), "{wire}");
    assert!(matches!(e, IpcError::AgentUnavailable { .. }), "{e:?}");
}

// ---------- the agent.status event ----------

#[test]
fn sec_03_agent_status_holds_the_mog_the_session_and_the_state_only() {
    let got: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let sink_got = got.clone();
    let sink = BrainStatusSink::new(Arc::new(move |event: &str, payload: Value| {
        sink_got.lock().unwrap().push((event.to_owned(), payload));
    }));
    for (state, wire) in [
        (SessionState::Running, "running"),
        (SessionState::Restarted, "restarted"),
        (SessionState::Failed, "failed"),
    ] {
        sink.status(&AgentStatus {
            mog_id: MOG.to_owned(),
            agent_session_id: "as-fake-session".to_owned(),
            state,
        });
        let (event, payload) = got.lock().unwrap().pop().expect("one event");
        assert_eq!(event, ipc::events::AGENT_STATUS);
        assert_eq!(
            payload,
            json!({"mogId": MOG, "agentSessionId": "as-fake-session", "state": wire})
        );
    }
}

#[test]
fn arc_f14_the_host_status_reaches_the_brain_sink() {
    let got: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let sink_got = got.clone();
    let sink = Arc::new(BrainStatusSink::new(Arc::new(move |e: &str, p: Value| {
        sink_got.lock().unwrap().push((e.to_owned(), p));
    })));
    let r = rig_with(FakeLauncher::default(), sink);
    let id = start(&r, MOG, "claude").unwrap()["agentSessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    call(&r, "brain", "agent_send", turn_args(MOG)).unwrap();
    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: id.clone(),
        },
    );
    let sent = got.lock().unwrap().clone();
    assert_eq!(
        sent,
        vec![(
            ipc::events::AGENT_STATUS.to_owned(),
            json!({"mogId": MOG, "agentSessionId": id, "state": "restarted"})
        )]
    );
    assert!(!serde_json::to_string(&sent).unwrap().contains(QUESTION));
}

// ---------- the redacting Debug of the turn ----------

#[test]
fn sec_16c_agent_send_args_debug_never_shows_the_question() {
    let a: AgentSendArgs = serde_json::from_value(turn_args(MOG)).unwrap();
    for text in [format!("{a:?}"), format!("{a:#?}")] {
        assert!(!text.contains(QUESTION), "{text}");
        assert!(text.contains(MOG), "the MOG ID still shows: {text}");
    }
    // The wire form still carries the turn: only `Debug` redacts.
    assert_eq!(serde_json::to_value(&a).unwrap(), turn_args(MOG));
}
