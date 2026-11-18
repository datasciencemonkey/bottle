//! R16 AgentHost (#112): one shared harness runtime with one session per active MOG (U11,
//! RTR-21a, ARC-7), the Claude and Codex adapters behind one host (RTR-22), the model route
//! through the loopback proxy (RTR-23) and the restart-once rule (ARC-F14).
//!
//! Every test uses doubles: a fake runtime process, a fake token broker and a fake upstream.
//! No test starts a real harness, calls a model or reads a CLI profile. TODO(#111): the live
//! SP-19 run replaces the fakes.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mog_core::agent_host::model_proxy::{
    ModelProxy, ModelTarget, Upstream, UpstreamError, UpstreamRequest, UpstreamResponse,
};
use mog_core::agent_host::runtime::{
    LaunchSpec, RuntimeConfig, RuntimeEvent, RuntimeEvents, RuntimeLauncher, RuntimeProcess,
};
use mog_core::agent_host::{
    AgentError, AgentHost, AgentHostDeps, AgentStatus, AgentStatusSink, Harness, ModelTargets,
    RuntimeConfigs, SessionState, SharedAgentHost,
};
use mog_core::bottle_mcp::{BottleMcpDeps, UnavailableLocalTools, UnavailableRelay, MCP_PATH};
use mog_core::error::CoreResult;
use mog_core::token_broker::{TokenBroker, TokenLease};
use serde_json::{json, Value};

const FAKE_TOKEN: &[u8] = b"dapi-FAKE-TOKEN-0123456789abcdef";

// ---------- doubles ----------

struct FakeTokens {
    calls: AtomicUsize,
}

impl TokenBroker for FakeTokens {
    fn with_token(
        &self,
        _profile: &str,
        _host: &str,
        f: &mut dyn FnMut(&[u8]),
    ) -> CoreResult<TokenLease> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        f(FAKE_TOKEN);
        Ok(TokenLease::default())
    }
    fn invalidate(&self, _profile: &str, _lease: TokenLease) -> CoreResult<()> {
        Ok(())
    }
    fn workspace_id(&self, _profile: &str) -> CoreResult<String> {
        Ok("123".into())
    }
}

struct NoUpstream;

impl Upstream for NoUpstream {
    fn forward(&self, _req: &UpstreamRequest<'_>) -> Result<UpstreamResponse, UpstreamError> {
        Err(UpstreamError)
    }
}

/// One fake runtime process: it records each frame it gets.
#[derive(Default)]
struct FakeProc {
    frames: Mutex<Vec<Value>>,
    stopped: Mutex<bool>,
    /// While set, a turn for this session ID blocks until the gate opens.
    block_turns_of: Mutex<Option<String>>,
    gate: (Mutex<bool>, std::sync::Condvar),
}

struct FakeProcHandle(Arc<FakeProc>);

impl RuntimeProcess for FakeProcHandle {
    fn send_frame(&self, frame: &Value) -> Result<(), AgentError> {
        let blocked = self.0.block_turns_of.lock().unwrap().clone();
        if frame["type"] == "turn" && blocked.is_some_and(|b| frame["sessionId"] == b.as_str()) {
            let (m, cv) = &self.0.gate;
            let mut open = m.lock().unwrap();
            while !*open {
                open = cv.wait(open).unwrap();
            }
        }
        self.0.frames.lock().unwrap().push(frame.clone());
        Ok(())
    }
    fn stop(&self) {
        *self.0.stopped.lock().unwrap() = true;
    }
}

#[derive(Default)]
struct FakeLauncher {
    specs: Mutex<Vec<LaunchSpec>>,
    procs: Mutex<Vec<(Harness, Arc<FakeProc>, RuntimeEvents)>>,
}

impl RuntimeLauncher for FakeLauncher {
    fn launch(
        &self,
        spec: &LaunchSpec,
        events: RuntimeEvents,
    ) -> Result<Box<dyn RuntimeProcess>, AgentError> {
        self.specs.lock().unwrap().push(spec.clone());
        let p = Arc::new(FakeProc::default());
        self.procs
            .lock()
            .unwrap()
            .push((spec.harness, p.clone(), events));
        Ok(Box::new(FakeProcHandle(p)))
    }
}

impl FakeLauncher {
    fn launches(&self) -> usize {
        self.specs.lock().unwrap().len()
    }
    fn proc(&self, i: usize) -> Arc<FakeProc> {
        self.procs.lock().unwrap()[i].1.clone()
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

struct FakeTargets;

impl ModelTargets for FakeTargets {
    fn target(&self, mog_id: &str) -> Result<ModelTarget, AgentError> {
        if mog_id == "no-home" {
            return Err(AgentError::TargetUnavailable);
        }
        Ok(ModelTarget::Gateway {
            profile: "bottle-home".into(),
            host: "example.cloud.databricks.com".into(),
        })
    }
}

struct Rig {
    host: Arc<SharedAgentHost>,
    launcher: Arc<FakeLauncher>,
    status: Arc<FakeStatus>,
    proxy: Arc<ModelProxy>,
}

fn rig() -> Rig {
    let launcher = Arc::new(FakeLauncher::default());
    let status = Arc::new(FakeStatus::default());
    let proxy = Arc::new(
        ModelProxy::start(
            Arc::new(FakeTokens {
                calls: AtomicUsize::new(0),
            }),
            Arc::new(NoUpstream),
        )
        .expect("proxy binds loopback"),
    );
    let host = SharedAgentHost::new(AgentHostDeps {
        bottle_mcp: mcp_deps(),
        launcher: launcher.clone(),
        proxy: proxy.clone(),
        targets: Arc::new(FakeTargets),
        status: status.clone(),
        runtimes: RuntimeConfigs {
            claude: RuntimeConfig::new("fake-claude-runtime", Vec::new()),
            codex: RuntimeConfig::new("fake-codex-runtime", Vec::new()),
        },
    });
    Rig {
        host,
        launcher,
        status,
        proxy,
    }
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

fn frames_for(p: &FakeProc, session_id: &str) -> Vec<Value> {
    p.frames
        .lock()
        .unwrap()
        .iter()
        .filter(|f| f["sessionId"] == session_id)
        .cloned()
        .collect()
}

fn secret_of(frame: &Value) -> String {
    frame["modelEndpoint"]["secret"]
        .as_str()
        .expect("open frame has a secret")
        .to_owned()
}

fn turn() -> Value {
    json!({ "question": "what was revenue last week", "recall": [] })
}

// ---------- RTR-21a / AT-42: one shared runtime, one session per MOG ----------

#[test]
fn rtr_21a_at_42_three_mogs_share_one_runtime_with_three_sessions() {
    let r = rig();
    let ids: Vec<String> = ["a", "b", "c"]
        .iter()
        .map(|m| r.host.start_session(m, Harness::Claude).unwrap())
        .collect();
    assert_eq!(r.launcher.launches(), 1, "one runtime process for 3 MOGs");
    assert_eq!(r.host.runtime_count(), 1);
    assert_eq!(r.host.session_count(), 3);
    let mut distinct = ids.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 3, "each MOG has its own agent session");
    let opens = frames_of(&r.launcher.proc(0), "open_session");
    assert_eq!(opens.len(), 3);
    let mut secrets: Vec<String> = opens.iter().map(secret_of).collect();
    secrets.sort();
    secrets.dedup();
    assert_eq!(secrets.len(), 3, "each session has its own secret");
    assert_eq!(r.proxy.session_count(), 3);
}

#[test]
fn rtr_21a_start_session_is_idempotent_per_mog() {
    let r = rig();
    let a1 = r.host.start_session("a", Harness::Claude).unwrap();
    let a2 = r.host.start_session("a", Harness::Claude).unwrap();
    assert_eq!(a1, a2);
    assert_eq!(frames_of(&r.launcher.proc(0), "open_session").len(), 1);
    assert_eq!(r.host.session_count(), 1);
}

#[test]
fn rtr_21a_sessions_share_no_conversation_state() {
    let r = rig();
    let a = r.host.start_session("a", Harness::Claude).unwrap();
    let b = r.host.start_session("b", Harness::Claude).unwrap();
    r.host.send("a", &turn()).unwrap();
    let p = r.launcher.proc(0);
    let turns = frames_of(&p, "turn");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0]["sessionId"], a.as_str());
    assert_eq!(turns[0]["turn"], turn());
    assert!(
        frames_for(&p, &b)
            .iter()
            .all(|f| f["type"] == "open_session"),
        "MOG b gets nothing from MOG a's turn"
    );
}

#[test]
fn rtr_21a_send_to_a_mog_without_a_session_fails() {
    let r = rig();
    let e = r.host.send("nobody", &turn()).unwrap_err();
    assert_eq!(e, AgentError::UnknownMog);
    assert_eq!(e.code(), "agent_unknown_mog");
}

#[test]
fn rtr_21a_at_42_one_session_crash_does_not_stop_the_others() {
    let r = rig();
    let a = r.host.start_session("a", Harness::Claude).unwrap();
    let b = r.host.start_session("b", Harness::Claude).unwrap();
    let c = r.host.start_session("c", Harness::Claude).unwrap();
    let p = r.launcher.proc(0);
    let before_a = frames_for(&p, &a).len();
    let before_c = frames_for(&p, &c).len();

    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: b.clone(),
        },
    );

    assert_eq!(
        r.launcher.launches(),
        1,
        "a session crash does not relaunch the runtime"
    );
    assert!(
        !*p.stopped.lock().unwrap(),
        "the shared runtime keeps running"
    );
    assert_eq!(frames_for(&p, &a).len(), before_a, "MOG a is not touched");
    assert_eq!(frames_for(&p, &c).len(), before_c, "MOG c is not touched");
    assert_eq!(r.host.session_state("a"), Some(SessionState::Running));
    assert_eq!(r.host.session_state("c"), Some(SessionState::Running));
    r.host.send("a", &turn()).unwrap();
    r.host.send("c", &turn()).unwrap();
}

#[test]
fn rtr_21a_stop_session_closes_only_that_session() {
    let r = rig();
    let a = r.host.start_session("a", Harness::Claude).unwrap();
    r.host.start_session("b", Harness::Claude).unwrap();
    r.host.stop_session("a").unwrap();
    let p = r.launcher.proc(0);
    let closes = frames_of(&p, "close_session");
    assert_eq!(closes.len(), 1);
    assert_eq!(closes[0]["sessionId"], a.as_str());
    assert_eq!(r.host.session_count(), 1);
    assert_eq!(
        r.proxy.session_count(),
        1,
        "the closed session's secret is revoked"
    );
    assert!(!*p.stopped.lock().unwrap());
    assert_eq!(
        r.host.stop_session("a").unwrap_err(),
        AgentError::UnknownMog
    );
}

#[test]
fn rtr_21a_last_session_stop_stops_the_runtime() {
    let r = rig();
    r.host.start_session("a", Harness::Claude).unwrap();
    r.host.stop_session("a").unwrap();
    assert!(*r.launcher.proc(0).stopped.lock().unwrap());
    assert_eq!(r.host.runtime_count(), 0);
    r.host.start_session("b", Harness::Claude).unwrap();
    assert_eq!(
        r.launcher.launches(),
        2,
        "a later start launches a fresh runtime"
    );
}

#[test]
fn rtr_23_start_fails_without_a_model_target() {
    let r = rig();
    let e = r
        .host
        .start_session("no-home", Harness::Claude)
        .unwrap_err();
    assert_eq!(e, AgentError::TargetUnavailable);
    assert_eq!(r.launcher.launches(), 0, "no runtime without a model route");
}

// ---------- RTR-22: two harness kinds behind one host ----------

/// The same host contract for each harness kind: start, turn, crash restart, stop. The AT-42
/// tool-contract run with each live harness is TODO(#111).
fn harness_contract(h: Harness) {
    let r = rig();
    let id = r.host.start_session("m1", h).unwrap();
    let spec = r.launcher.specs.lock().unwrap()[0].clone();
    assert_eq!(spec.harness, h);
    let p = r.launcher.proc(0);
    let open = &frames_of(&p, "open_session")[0];
    assert_eq!(open["harness"], json!(h.as_str()));
    let base = open["modelEndpoint"]["baseUrl"].as_str().unwrap();
    assert!(base.starts_with(&r.proxy.base_url()), "{base}");
    r.host.send("m1", &turn()).unwrap();
    assert_eq!(frames_of(&p, "turn").len(), 1);
    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: id.clone(),
        },
    );
    assert_eq!(r.host.session_state("m1"), Some(SessionState::Restarted));
    r.host.stop_session("m1").unwrap();
    assert_eq!(r.host.session_count(), 0);
}

#[test]
fn rtr_22_claude_harness_passes_the_host_contract() {
    harness_contract(Harness::Claude);
}

#[test]
fn rtr_22_codex_harness_passes_the_host_contract() {
    harness_contract(Harness::Codex);
}

#[test]
fn rtr_22_claude_and_codex_sessions_run_side_by_side() {
    let r = rig();
    r.host.start_session("a", Harness::Claude).unwrap();
    r.host.start_session("b", Harness::Codex).unwrap();
    r.host.start_session("c", Harness::Claude).unwrap();
    let kinds: Vec<Harness> = r
        .launcher
        .specs
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.harness)
        .collect();
    assert_eq!(kinds, vec![Harness::Claude, Harness::Codex]);
    assert_eq!(r.host.session_count(), 3);
}

#[test]
fn rtr_22_ipc_harness_maps_to_the_host_harness() {
    assert_eq!(
        Harness::from(mog_core::ipc::Harness::Claude),
        Harness::Claude
    );
    assert_eq!(Harness::from(mog_core::ipc::Harness::Codex), Harness::Codex);
}

// ---------- RTR-23: base URL env names ----------

#[test]
fn rtr_23_claude_runtime_gets_anthropic_base_url_of_the_loopback_proxy() {
    let r = rig();
    r.host.start_session("a", Harness::Claude).unwrap();
    let spec = r.launcher.specs.lock().unwrap()[0].clone();
    let url = spec.env_value("ANTHROPIC_BASE_URL").expect("set");
    assert_eq!(url, format!("{}/anthropic", r.proxy.base_url()));
    assert!(url.starts_with("http://127.0.0.1:"));
    assert!(spec.env_value("OPENAI_BASE_URL").is_none());
}

#[test]
fn rtr_23_codex_runtime_gets_openai_base_url_of_the_loopback_proxy() {
    let r = rig();
    r.host.start_session("a", Harness::Codex).unwrap();
    let spec = r.launcher.specs.lock().unwrap()[0].clone();
    let url = spec.env_value("OPENAI_BASE_URL").expect("set");
    assert_eq!(url, format!("{}/openai/v1", r.proxy.base_url()));
    assert!(spec.env_value("ANTHROPIC_BASE_URL").is_none());
}

// ---------- ARC-7 / RTR-28: no token reaches the harness ----------

#[test]
fn arc_7_rtr_28_launch_spec_and_frames_carry_no_token() {
    let r = rig();
    for (m, h) in [("a", Harness::Claude), ("b", Harness::Codex)] {
        r.host.start_session(m, h).unwrap();
        r.host.send(m, &turn()).unwrap();
    }
    let token = std::str::from_utf8(FAKE_TOKEN).unwrap();
    for spec in r.launcher.specs.lock().unwrap().iter() {
        for (k, v) in &spec.env {
            assert!(!v.contains(token) && !k.contains(token));
        }
        for a in &spec.args {
            assert!(!a.contains(token));
        }
    }
    for i in 0..2 {
        for f in r.launcher.proc(i).frames.lock().unwrap().iter() {
            assert!(!f.to_string().contains(token), "frame carries no token");
        }
    }
}

#[test]
fn arc_7_session_secret_is_not_in_env_or_args() {
    let r = rig();
    r.host.start_session("a", Harness::Claude).unwrap();
    let secret = secret_of(&frames_of(&r.launcher.proc(0), "open_session")[0]);
    let spec = r.launcher.specs.lock().unwrap()[0].clone();
    assert!(spec.env.iter().all(|(_, v)| !v.contains(&secret)));
    assert!(spec.args.iter().all(|a| !a.contains(&secret)));
}

#[test]
fn arc_7_status_and_errors_are_content_free() {
    let r = rig();
    let a = r.host.start_session("a", Harness::Claude).unwrap();
    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: a.clone(),
        },
    );
    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: a.clone(),
        },
    );
    let e = r.host.send("a", &turn()).unwrap_err();
    let text = format!("{e} {e:?}");
    assert!(!text.contains("revenue"), "an error never echoes the turn");
    for s in r.status.0.lock().unwrap().iter() {
        let dbg = format!("{s:?}");
        assert!(!dbg.contains("revenue"));
        assert_eq!(s.mog_id, "a");
        assert_eq!(s.agent_session_id, a);
    }
}

// ---------- ARC-F14: restart a failed session once ----------

#[test]
fn arc_f14_restarts_a_failed_session_once_with_a_new_secret() {
    let r = rig();
    let a = r.host.start_session("a", Harness::Claude).unwrap();
    let p = r.launcher.proc(0);
    let first = secret_of(&frames_of(&p, "open_session")[0]);

    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: a.clone(),
        },
    );

    let opens = frames_of(&p, "open_session");
    assert_eq!(opens.len(), 2, "the session is reopened once");
    assert_eq!(opens[1]["sessionId"], a.as_str(), "same agentSessionId");
    assert_ne!(secret_of(&opens[1]), first, "the secret rotates");
    assert_eq!(r.proxy.session_count(), 1, "the old secret is revoked");
    assert_eq!(r.host.session_state("a"), Some(SessionState::Restarted));
    r.host.send("a", &turn()).unwrap();
    let states: Vec<SessionState> = r.status.0.lock().unwrap().iter().map(|s| s.state).collect();
    assert!(states.contains(&SessionState::Restarted));
}

#[test]
fn arc_f14_second_failure_marks_the_session_failed_for_the_fallback() {
    let r = rig();
    let a = r.host.start_session("a", Harness::Claude).unwrap();
    r.host.start_session("b", Harness::Claude).unwrap();
    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: a.clone(),
        },
    );
    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: a.clone(),
        },
    );

    assert_eq!(
        frames_of(&r.launcher.proc(0), "open_session").len(),
        3,
        "no 2nd restart"
    );
    assert_eq!(r.host.session_state("a"), Some(SessionState::Failed));
    let e = r.host.send("a", &turn()).unwrap_err();
    assert_eq!(e, AgentError::Unavailable);
    assert_eq!(e.code(), "agent_unavailable");
    assert_eq!(r.proxy.session_count(), 1, "only b keeps a model route");
    assert_eq!(r.host.session_state("b"), Some(SessionState::Running));
    r.host.send("b", &turn()).unwrap();
    let last = r.status.0.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.state, SessionState::Failed);
}

#[test]
fn arc_f14_a_failed_mog_can_start_again_after_a_user_action() {
    let r = rig();
    let a = r.host.start_session("a", Harness::Claude).unwrap();
    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: a.clone(),
        },
    );
    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: a.clone(),
        },
    );
    let a2 = r.host.start_session("a", Harness::Claude).unwrap();
    assert_ne!(a, a2, "a new session after the failed one");
    assert_eq!(r.host.session_state("a"), Some(SessionState::Running));
}

#[test]
fn arc_f14_runtime_exit_relaunches_once_and_reopens_the_sessions() {
    let r = rig();
    let a = r.host.start_session("a", Harness::Claude).unwrap();
    let b = r.host.start_session("b", Harness::Claude).unwrap();
    r.launcher.fire(0, RuntimeEvent::Exited);

    assert_eq!(r.launcher.launches(), 2, "the runtime restarts once");
    let p2 = r.launcher.proc(1);
    let reopened: Vec<String> = frames_of(&p2, "open_session")
        .iter()
        .map(|f| f["sessionId"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(reopened.len(), 2);
    assert!(reopened.contains(&a) && reopened.contains(&b));
    assert_eq!(r.host.session_state("a"), Some(SessionState::Restarted));
    r.host.send("a", &turn()).unwrap();
    assert_eq!(
        frames_of(&p2, "turn").len(),
        1,
        "turns go to the new process"
    );

    r.launcher.fire(1, RuntimeEvent::Exited);
    assert_eq!(r.launcher.launches(), 2, "no second runtime restart");
    assert_eq!(r.host.session_state("a"), Some(SessionState::Failed));
    assert_eq!(r.host.session_state("b"), Some(SessionState::Failed));
    assert_eq!(
        r.host.send("b", &turn()).unwrap_err(),
        AgentError::Unavailable
    );
    assert_eq!(r.proxy.session_count(), 0);
}

#[test]
fn arc_f14_runtime_exit_of_one_kind_does_not_touch_the_other_kind() {
    let r = rig();
    r.host.start_session("a", Harness::Claude).unwrap();
    r.host.start_session("b", Harness::Codex).unwrap();
    let codex_frames = r.launcher.proc(1).frames.lock().unwrap().len();
    r.launcher.fire(0, RuntimeEvent::Exited);
    assert_eq!(
        r.launcher.proc(1).frames.lock().unwrap().len(),
        codex_frames
    );
    assert_eq!(r.host.session_state("b"), Some(SessionState::Running));
}

#[test]
fn arc_f14_stale_events_from_an_old_process_are_ignored() {
    let r = rig();
    let a = r.host.start_session("a", Harness::Claude).unwrap();
    r.launcher.fire(0, RuntimeEvent::Exited);
    assert_eq!(r.launcher.launches(), 2);
    // A late event from the dead first process must not spend the new budget.
    r.launcher.fire(0, RuntimeEvent::Exited);
    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: a.clone(),
        },
    );
    assert_eq!(r.launcher.launches(), 2);
    assert_eq!(r.host.session_state("a"), Some(SessionState::Restarted));
}

#[test]
fn arc_f14_unknown_session_failure_is_ignored() {
    let r = rig();
    r.host.start_session("a", Harness::Claude).unwrap();
    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: "not-a-session".into(),
        },
    );
    assert_eq!(r.host.session_state("a"), Some(SessionState::Running));
}

// ---------- review of #309: no blocking I/O under the host lock ----------

/// Run `f` on a thread and fail the test (instead of hanging) if it does not finish in 5 s.
fn within_5s<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("the host is blocked")
}

#[test]
fn rtr_21a_arc_f14_a_blocked_turn_does_not_stop_the_other_mogs() {
    let r = rig();
    let a = r.host.start_session("a", Harness::Claude).unwrap();
    let b = r.host.start_session("b", Harness::Claude).unwrap();
    let p = r.launcher.proc(0);
    *p.block_turns_of.lock().unwrap() = Some(a.clone());

    let host = r.host.clone();
    let stuck = std::thread::spawn(move || host.send("a", &turn()));
    std::thread::sleep(std::time::Duration::from_millis(50));

    let host = r.host.clone();
    let state_b = within_5s(move || host.session_state("b"));
    assert_eq!(state_b, Some(SessionState::Running));
    let host = r.host.clone();
    within_5s(move || host.send("b", &turn())).unwrap();
    // A failure event of another session is still handled while MOG a's turn is stuck.
    let (launcher, host) = (r.launcher.clone(), r.host.clone());
    let b2 = b.clone();
    within_5s(move || {
        launcher.fire(0, RuntimeEvent::SessionFailed { session_id: b2 });
        host.session_state("b")
    });
    assert_eq!(r.host.session_state("b"), Some(SessionState::Restarted));

    let (m, cv) = &p.gate;
    *m.lock().unwrap() = true;
    cv.notify_all();
    stuck.join().unwrap().unwrap();
}

#[test]
fn rtr_22_start_with_the_other_harness_kind_is_refused() {
    let r = rig();
    let a = r.host.start_session("a", Harness::Claude).unwrap();
    let e = r.host.start_session("a", Harness::Codex).unwrap_err();
    assert_eq!(e, AgentError::HarnessMismatch);
    assert_eq!(e.code(), "agent_harness_mismatch");
    assert_eq!(r.launcher.launches(), 1, "no Codex runtime starts");
    r.host.stop_session("a").unwrap();
    let a2 = r.host.start_session("a", Harness::Codex).unwrap();
    assert_ne!(a, a2);
}

/// A sink that calls back into the host. It deadlocks if the host reports under its lock.
struct ReentrantStatus {
    host: std::sync::OnceLock<std::sync::Weak<SharedAgentHost>>,
    seen: Mutex<Vec<Option<SessionState>>>,
}

impl AgentStatusSink for ReentrantStatus {
    fn status(&self, s: &AgentStatus) {
        let host = self.host.get().and_then(|w| w.upgrade()).unwrap();
        self.seen
            .lock()
            .unwrap()
            .push(host.session_state(&s.mog_id));
    }
}

#[test]
fn arc_f14_status_reports_are_sent_outside_the_host_lock() {
    let launcher = Arc::new(FakeLauncher::default());
    let status = Arc::new(ReentrantStatus {
        host: std::sync::OnceLock::new(),
        seen: Mutex::new(Vec::new()),
    });
    let proxy = Arc::new(
        ModelProxy::start(
            Arc::new(FakeTokens {
                calls: AtomicUsize::new(0),
            }),
            Arc::new(NoUpstream),
        )
        .unwrap(),
    );
    let host = SharedAgentHost::new(AgentHostDeps {
        bottle_mcp: mcp_deps(),
        launcher: launcher.clone(),
        proxy,
        targets: Arc::new(FakeTargets),
        status: status.clone(),
        runtimes: RuntimeConfigs {
            claude: RuntimeConfig::new("fake", Vec::new()),
            codex: RuntimeConfig::new("fake", Vec::new()),
        },
    });
    status.host.set(Arc::downgrade(&host)).ok().unwrap();
    let a = host.start_session("a", Harness::Claude).unwrap();
    let l = launcher.clone();
    within_5s(move || {
        l.fire(
            0,
            RuntimeEvent::SessionFailed {
                session_id: a.clone(),
            },
        );
        l.fire(0, RuntimeEvent::Exited);
    });
    let seen = status.seen.lock().unwrap().clone();
    assert_eq!(
        seen,
        vec![Some(SessionState::Restarted), Some(SessionState::Failed)]
    );
}

struct CountingTargets(AtomicUsize);

impl ModelTargets for CountingTargets {
    fn target(&self, m: &str) -> Result<ModelTarget, AgentError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        FakeTargets.target(m)
    }
}

#[test]
fn rtr_21a_start_for_a_live_session_needs_no_target_lookup() {
    let targets = Arc::new(CountingTargets(AtomicUsize::new(0)));
    let host = SharedAgentHost::new(AgentHostDeps {
        bottle_mcp: mcp_deps(),
        launcher: Arc::new(FakeLauncher::default()),
        proxy: Arc::new(
            ModelProxy::start(
                Arc::new(FakeTokens {
                    calls: AtomicUsize::new(0),
                }),
                Arc::new(NoUpstream),
            )
            .unwrap(),
        ),
        targets: targets.clone(),
        status: Arc::new(FakeStatus::default()),
        runtimes: RuntimeConfigs {
            claude: RuntimeConfig::new("fake", Vec::new()),
            codex: RuntimeConfig::new("fake", Vec::new()),
        },
    });
    let a = host.start_session("a", Harness::Claude).unwrap();
    assert_eq!(host.start_session("a", Harness::Claude).unwrap(), a);
    assert_eq!(
        host.start_session("a", Harness::Codex).unwrap_err(),
        AgentError::HarnessMismatch
    );
    assert_eq!(
        targets.0.load(Ordering::SeqCst),
        1,
        "one lookup, at the first start"
    );
}

// ---------- RTR-25 / AT-42: only the AgentHost child gets the bottle-mcp secret ----------

fn mcp_deps() -> BottleMcpDeps {
    BottleMcpDeps::new(Arc::new(UnavailableLocalTools), Arc::new(UnavailableRelay))
}

/// The `bottleMcp` part of an open frame: the URL and the `Authorization` value.
fn mcp_of(frame: &Value) -> (String, String) {
    let m = &frame["bottleMcp"];
    (
        m["url"]
            .as_str()
            .expect("open frame has a bottle-mcp url")
            .to_owned(),
        m["authorization"]
            .as_str()
            .expect("open frame has a bottle-mcp authorization")
            .to_owned(),
    )
}

/// POST `tools/list` to `url` with `auth`; the HTTP status, or 0 if nothing answers.
fn mcp_tools_list(url: &str, auth: &str) -> (u16, Value) {
    use std::io::{Read, Write};
    let authority = url
        .strip_prefix("http://")
        .and_then(|r| r.split('/').next())
        .expect("http url");
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string();
    let req = format!(
        "POST {MCP_PATH} HTTP/1.1\r\nHost: {authority}\r\nAuthorization: {auth}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let Ok(mut s) = std::net::TcpStream::connect(authority) else {
        return (0, Value::Null);
    };
    s.set_read_timeout(Some(std::time::Duration::from_secs(120)))
        .unwrap();
    let _ = s.write_all(req.as_bytes());
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

#[test]
fn rtr_25_at_42_only_the_agent_host_child_gets_the_bottle_mcp_secret() {
    let r = rig();
    let a = r.host.start_session("a", Harness::Claude).unwrap();
    let b = r.host.start_session("b", Harness::Claude).unwrap();
    let opens = frames_of(&r.launcher.proc(0), "open_session");
    assert_eq!(opens.len(), 2);
    let (url_a, auth_a) = mcp_of(&opens[0]);
    let (url_b, auth_b) = mcp_of(&opens[1]);
    for (url, auth) in [(&url_a, &auth_a), (&url_b, &auth_b)] {
        assert!(url.starts_with("http://127.0.0.1:"), "{url}");
        assert!(url.ends_with(MCP_PATH), "{url}");
        assert!(auth.starts_with("Bearer ") && auth.len() > 40);
    }
    assert_ne!(auth_a, auth_b, "one secret per session");
    assert_ne!(url_a, url_b, "one bottle-mcp per session");
    assert_ne!(&auth_a[7..], secret_of(&opens[0]), "not the model secret");

    // The secret reaches the child on stdin only: not in the runtime environment, not in a
    // status report, not in a Debug print of the host or the proxy.
    for spec in r.launcher.specs.lock().unwrap().iter() {
        for (k, v) in &spec.env {
            assert!(
                !v.contains(&auth_a[7..]) && !v.contains(&auth_b[7..]),
                "{k}"
            );
        }
    }
    let seen = format!("{:?} {:?}", r.status.0.lock().unwrap(), r.proxy);
    assert!(!seen.contains(&auth_a[7..]) && !seen.contains(&auth_b[7..]));

    // The secret opens the session's own bottle-mcp, and only that one.
    let (status, v) = mcp_tools_list(&url_a, &auth_a);
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["result"]["tools"].as_array().unwrap().len(), 7);
    assert_eq!(mcp_tools_list(&url_a, &auth_b).0, 401);

    // Stop closes the session's bottle-mcp; the other session keeps its own.
    r.host.stop_session("a").unwrap();
    // Nothing answers (or, if another process took the free port, not with a 200).
    assert_ne!(
        mcp_tools_list(&url_a, &auth_a).0,
        200,
        "stopped: the secret opens nothing"
    );
    assert_eq!(mcp_tools_list(&url_b, &auth_b).0, 200);
    let _ = (a, b);
}

#[test]
fn rtr_25_arc_f14_a_restart_gives_bottle_mcp_a_new_secret() {
    let r = rig();
    let a = r.host.start_session("a", Harness::Claude).unwrap();
    let p = r.launcher.proc(0);
    let (old_url, old_auth) = mcp_of(&frames_of(&p, "open_session")[0]);
    r.launcher.fire(
        0,
        RuntimeEvent::SessionFailed {
            session_id: a.clone(),
        },
    );
    let opens = frames_of(&p, "open_session");
    assert_eq!(opens.len(), 2, "restarted once");
    let (new_url, new_auth) = mcp_of(&opens[1]);
    assert_ne!(old_auth, new_auth);
    assert_ne!(
        mcp_tools_list(&old_url, &old_auth).0,
        200,
        "old secret revoked"
    );
    assert_eq!(mcp_tools_list(&new_url, &new_auth).0, 200);
}

// ---------- RTR-27 / AT-42: the Genie One MCP proxy endpoint of each session ----------

struct OneTarget;

impl mog_core::mcp_transport::proxy::GenieTargets for OneTarget {
    fn target(&self, _mog_id: &str) -> CoreResult<mog_core::mcp_transport::proxy::GenieTarget> {
        Ok(mog_core::mcp_transport::proxy::GenieTarget {
            profile: "bottle-home".into(),
            warehouse_id: None,
        })
    }
}

fn rig_with_genie() -> Rig {
    let launcher = Arc::new(FakeLauncher::default());
    let status = Arc::new(FakeStatus::default());
    let proxy = Arc::new(
        ModelProxy::start(
            Arc::new(FakeTokens {
                calls: AtomicUsize::new(0),
            }),
            Arc::new(NoUpstream),
        )
        .expect("proxy binds loopback"),
    );
    let genie = mog_core::mcp_transport::proxy::GenieProxyDeps::new(
        Arc::new(mog_core::mcp_transport::StubMcpTransport),
        Arc::new(OneTarget),
    );
    let host = SharedAgentHost::new(AgentHostDeps {
        bottle_mcp: mcp_deps().with_genie_proxy(genie),
        launcher: launcher.clone(),
        proxy: proxy.clone(),
        targets: Arc::new(FakeTargets),
        status: status.clone(),
        runtimes: RuntimeConfigs {
            claude: RuntimeConfig::new("fake-claude-runtime", Vec::new()),
            codex: RuntimeConfig::new("fake-codex-runtime", Vec::new()),
        },
    });
    Rig {
        host,
        launcher,
        status,
        proxy,
    }
}

/// The `genieOneMcp` part of an open frame: the URL and the `Authorization` value.
fn genie_of(frame: &Value) -> (String, String) {
    let m = &frame["genieOneMcp"];
    (
        m["url"]
            .as_str()
            .expect("open frame has a Genie One proxy url")
            .to_owned(),
        m["authorization"]
            .as_str()
            .expect("open frame has a Genie One proxy authorization")
            .to_owned(),
    )
}

#[test]
fn rtr_27_rtr_28_at_42_each_session_gets_its_own_genie_one_proxy_endpoint_and_no_token() {
    let r = rig_with_genie();
    r.host.start_session("a", Harness::Claude).unwrap();
    r.host.start_session("b", Harness::Codex).unwrap();
    let open_a = frames_of(&r.launcher.proc(0), "open_session")[0].clone();
    let open_b = frames_of(&r.launcher.proc(1), "open_session")[0].clone();
    let (url_a, auth_a) = genie_of(&open_a);
    let (url_b, auth_b) = genie_of(&open_b);
    for url in [&url_a, &url_b] {
        assert!(url.starts_with("http://127.0.0.1:"), "{url}");
        assert!(url.ends_with(MCP_PATH), "{url}");
    }
    assert_ne!(url_a, url_b, "one proxy listener per session");
    assert_ne!(auth_a, auth_b, "one secret per session");
    // It is the session's loopback secret (shared with bottle-mcp), not the model secret.
    assert_eq!(auth_a, mcp_of(&open_a).1);
    assert_ne!(url_a, mcp_of(&open_a).0, "its own listener");
    assert_ne!(&auth_a[7..], secret_of(&open_a));

    // The endpoint goes on stdin only: not in the env or the arguments, and no token anywhere.
    let token = std::str::from_utf8(FAKE_TOKEN).unwrap();
    for spec in r.launcher.specs.lock().unwrap().iter() {
        for (k, v) in &spec.env {
            assert!(!v.contains(&auth_a[7..]) && !v.contains(&auth_b[7..]), "{k}");
            assert!(!v.contains(&url_a) && !v.contains(&url_b), "{k}");
            assert!(!v.contains(token), "{k}");
        }
    }
    for f in [&open_a, &open_b] {
        assert!(!f.to_string().contains(token));
    }

    // The proxy answers with the session secret only, and closes with the session.
    let (status, v) = mcp_tools_list(&url_a, &auth_a);
    assert_eq!(status, 200, "{v}");
    assert_eq!(mcp_tools_list(&url_a, &auth_b).0, 401);
    r.host.stop_session("a").unwrap();
    assert_ne!(mcp_tools_list(&url_a, &auth_a).0, 200);
}

#[test]
fn rtr_27_without_a_genie_proxy_the_frame_names_no_genie_endpoint() {
    let r = rig();
    r.host.start_session("a", Harness::Claude).unwrap();
    let open = frames_of(&r.launcher.proc(0), "open_session")[0].clone();
    assert!(open.get("genieOneMcp").is_none(), "{open}");
}
