//! Tests of the session host. The server is an in-process fake behind the `Spawner` seam, the
//! guard and the network watch are fakes of the frozen seam, and the clock is simulated.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::approval::Approvals;
use super::appserver::{ServerIo, ServerProc, SpawnSpec, Spawner};
use super::component::*;
use super::genie_bin::BinInfo;
use super::install::InstallError;
use crate::agent_run::seam::{
    AgentEvent, CallRecord, CatalogEntry, NetWatch, RunEvents, RunSecret, SessionGuard,
    SessionSpec, StopWhy, TurnSpec,
};
use crate::clock::{SimClock, SimWallClock};
use crate::error::CoreResult;
use crate::ipc::AgentTurnSubmitArgs;
use crate::types::{
    AgentAnswerKind, AgentCallState, AgentProfileState, AgentSessionEndReason, AgentSessionEvent,
    AgentSessionPhase, AgentTurnCall, AgentTurnEvent, AgentTurnOutcome, GenieCodeBinary,
};
use crate::wipe::AgentWipeHook;

const WAIT: Duration = Duration::from_secs(5);
const MOG: &str = "0123456789abcdef0123456789abcdef";

// ---------- pipes ----------

struct PipeReader {
    rx: Receiver<Vec<u8>>,
    left: VecDeque<u8>,
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.left.is_empty() {
            match self.rx.recv() {
                Ok(b) => self.left.extend(b),
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.left.len());
        for o in out.iter_mut().take(n) {
            *o = self.left.pop_front().unwrap_or(0);
        }
        Ok(n)
    }
}

struct PipeWriter(Sender<Vec<u8>>);

impl Write for PipeWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0
            .send(b.to_vec())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed"))?;
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

type SharedTx = Arc<Mutex<Option<Sender<Vec<u8>>>>>;

/// The test side of one fake server.
struct Server {
    spec: SpawnSpec,
    from_client: Mutex<Receiver<Vec<u8>>>,
    buf: Mutex<Vec<u8>>,
    out: SharedTx,
    err: SharedTx,
    killed: Arc<AtomicBool>,
}

impl Server {
    fn send(&self, v: Value) {
        let mut b = v.to_string().into_bytes();
        b.push(b'\n');
        if let Some(tx) = self.out.lock().unwrap().as_ref() {
            let _ = tx.send(b);
        }
    }

    fn stderr(&self, text: &str) {
        if let Some(tx) = self.err.lock().unwrap().as_ref() {
            let _ = tx.send(text.as_bytes().to_vec());
        }
    }

    /// The server dies: stdout reaches its end.
    fn die(&self) {
        *self.out.lock().unwrap() = None;
    }

    /// The next message of the client.
    fn recv(&self) -> Value {
        loop {
            {
                let mut buf = self.buf.lock().unwrap();
                if let Some(i) = buf.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=i).collect();
                    return serde_json::from_slice(&line[..line.len() - 1]).unwrap();
                }
            }
            let chunk = self
                .from_client
                .lock()
                .unwrap()
                .recv_timeout(WAIT)
                .expect("the client sent no message");
            self.buf.lock().unwrap().extend(chunk);
        }
    }

    /// True if no message is waiting (after a short pause).
    fn quiet(&self) -> bool {
        std::thread::sleep(Duration::from_millis(150));
        self.buf.lock().unwrap().is_empty() && self.from_client.lock().unwrap().try_recv().is_err()
    }

    fn recv_req(&self, method: &str) -> (u64, Value) {
        let m = self.recv();
        assert_eq!(m["method"], method, "unexpected message");
        (m["id"].as_u64().expect("a request id"), m["params"].clone())
    }

    fn reply(&self, id: u64, result: Value) {
        self.send(json!({ "id": id, "result": result }));
    }

    fn notify(&self, method: &str, params: Value) {
        self.send(json!({ "method": method, "params": params }));
    }

    fn thread_ok() -> Value {
        json!({
            "thread": { "id": "th-1", "ephemeral": true, "cliVersion": "0.5.0",
                        "modelProvider": "databricks" },
            "modelProvider": "databricks",
            "approvalPolicy": "never",
            "sandbox": { "type": "readOnly", "networkAccess": false }
        })
    }

    /// Answer `initialize` and `thread/start` the way genie 0.5.0 does.
    fn handshake(&self) -> Value {
        let (id, p) = self.recv_req("initialize");
        assert_eq!(p["clientInfo"]["name"], "bottle");
        self.reply(id, json!({ "userAgent": "x" }));
        self.notify(
            "remoteControl/status/changed",
            json!({ "status": "disabled" }),
        );
        let n = self.recv();
        assert_eq!(n["method"], "initialized");
        assert!(n.get("id").is_none());
        let (id, p) = self.recv_req("thread/start");
        self.reply(id, Self::thread_ok());
        p
    }

    /// Take a `turn/start`, ack it and send `turn/started`. Gives the request id and params.
    fn start_turn(&self, server_turn: &str) -> (u64, Value) {
        let (id, p) = self.recv_req("turn/start");
        self.reply(
            id,
            json!({ "turn": { "id": server_turn, "status": "inProgress" } }),
        );
        self.notify(
            "turn/started",
            json!({ "threadId": "th-1", "turn": { "id": server_turn } }),
        );
        (id, p)
    }

    fn usage(&self, server_turn: &str, input: u64, output: u64) {
        self.notify(
            "thread/tokenUsage/updated",
            json!({ "threadId": "th-1", "turnId": server_turn, "tokenUsage": {
                "total": { "inputTokens": 999999, "outputTokens": 999999 },
                "last": { "inputTokens": input, "cachedInputTokens": 1,
                          "outputTokens": output, "reasoningOutputTokens": 2 } } }),
        );
    }

    fn complete(&self, server_turn: &str, status: &str, final_text: Option<&str>) {
        if let Some(t) = final_text {
            self.notify(
                "item/completed",
                json!({ "item": { "type": "agentMessage", "text": t, "phase": "final_answer" } }),
            );
        }
        self.notify(
            "turn/completed",
            json!({ "threadId": "th-1", "turn": { "id": server_turn, "status": status, "items": [] } }),
        );
    }

    fn killed(&self) -> bool {
        self.killed.load(Ordering::SeqCst)
    }
}

struct FakeProc {
    pid: u32,
    io: Option<ServerIo>,
    out: SharedTx,
    err: SharedTx,
    killed: Arc<AtomicBool>,
}

impl ServerProc for FakeProc {
    fn pid(&self) -> u32 {
        self.pid
    }
    fn take_io(&mut self) -> Option<ServerIo> {
        self.io.take()
    }
    fn terminate(&mut self) {
        self.killed.store(true, Ordering::SeqCst);
        *self.out.lock().unwrap() = None;
        *self.err.lock().unwrap() = None;
    }
}

#[derive(Default)]
struct FakeSpawner {
    servers: Mutex<VecDeque<Arc<Server>>>,
    all: Mutex<Vec<Arc<Server>>>,
    fail: AtomicBool,
    next_pid: AtomicUsize,
}

impl Spawner for FakeSpawner {
    fn spawn(&self, spec: &SpawnSpec) -> std::io::Result<Box<dyn ServerProc>> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("no"));
        }
        let (c2s_tx, c2s_rx) = mpsc::channel();
        let (out_tx, out_rx) = mpsc::channel();
        let (err_tx, err_rx) = mpsc::channel();
        let out: SharedTx = Arc::new(Mutex::new(Some(out_tx)));
        let err: SharedTx = Arc::new(Mutex::new(Some(err_tx)));
        let killed = Arc::new(AtomicBool::new(false));
        let server = Arc::new(Server {
            spec: spec.clone(),
            from_client: Mutex::new(c2s_rx),
            buf: Mutex::new(Vec::new()),
            out: Arc::clone(&out),
            err: Arc::clone(&err),
            killed: Arc::clone(&killed),
        });
        self.servers.lock().unwrap().push_back(Arc::clone(&server));
        self.all.lock().unwrap().push(server);
        let pid = 4000 + u32::try_from(self.next_pid.fetch_add(1, Ordering::SeqCst)).unwrap();
        Ok(Box::new(FakeProc {
            pid,
            io: Some(ServerIo {
                stdin: Box::new(PipeWriter(c2s_tx)),
                stdout: Box::new(PipeReader {
                    rx: out_rx,
                    left: VecDeque::new(),
                }),
                stderr: Box::new(PipeReader {
                    rx: err_rx,
                    left: VecDeque::new(),
                }),
            }),
            out,
            err,
            killed,
        }))
    }
}

impl FakeSpawner {
    fn next(&self) -> Arc<Server> {
        let start = std::time::Instant::now();
        loop {
            if let Some(s) = self.servers.lock().unwrap().pop_front() {
                return s;
            }
            assert!(start.elapsed() < WAIT, "no server was started");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn count(&self) -> usize {
        self.all.lock().unwrap().len()
    }
}

// ---------- seam fakes ----------

#[derive(Default)]
struct FakeGuard {
    log: Mutex<Vec<String>>,
    tainted: AtomicBool,
    calls: Mutex<Vec<CallRecord>>,
}

impl FakeGuard {
    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
    fn has(&self, entry: &str) -> bool {
        self.log().iter().any(|l| l == entry)
    }
}

impl SessionGuard for FakeGuard {
    fn open_session(&self, spec: SessionSpec) -> CoreResult<RunSecret> {
        self.log
            .lock()
            .unwrap()
            .push(format!("open:{}", spec.session_id));
        Ok(RunSecret([9; 32]))
    }
    fn bind_pid(&self, _: &str, pid: u32) {
        self.log.lock().unwrap().push(format!("bind:{pid}"));
    }
    fn begin_turn(&self, _: &str, t: TurnSpec) -> CoreResult<()> {
        self.log
            .lock()
            .unwrap()
            .push(format!("begin:{}:{}", t.turn_id, t.budget));
        Ok(())
    }
    fn stop_turn(&self, turn_id: &str, why: StopWhy) {
        self.log
            .lock()
            .unwrap()
            .push(format!("stop:{turn_id}:{why:?}"));
    }
    fn end_turn(&self, turn_id: &str) {
        self.log.lock().unwrap().push(format!("end:{turn_id}"));
    }
    fn tainted(&self, _: &str) -> bool {
        self.tainted.load(Ordering::SeqCst)
    }
    fn calls(&self, _: &str) -> Vec<CallRecord> {
        self.calls.lock().unwrap().clone()
    }
    fn call_detail(&self, _: &str, _: &str) -> Option<AgentTurnCall> {
        None
    }
    fn wipe_all(&self) {
        self.log.lock().unwrap().push("wipe_all".to_owned());
    }
    fn close_session(&self, session_id: &str, wipe: bool) {
        self.log
            .lock()
            .unwrap()
            .push(format!("close:{session_id}:{wipe}"));
    }
}

type NetCallback = Box<dyn Fn(bool) + Send + Sync>;

#[derive(Default)]
struct FakeNet {
    offline: AtomicBool,
    cb: Mutex<Option<NetCallback>>,
}

impl NetWatch for FakeNet {
    fn online(&self) -> bool {
        !self.offline.load(Ordering::SeqCst)
    }
    fn subscribe(&self, f: Box<dyn Fn(bool) + Send + Sync>) {
        *self.cb.lock().unwrap() = Some(f);
    }
}

impl FakeNet {
    fn set(&self, online: bool) {
        self.offline.store(!online, Ordering::SeqCst);
        if let Some(f) = self.cb.lock().unwrap().as_ref() {
            f(online);
        }
    }
}

#[derive(Default)]
struct Rec {
    v: Mutex<Vec<AgentEvent>>,
    cv: Condvar,
}

impl RunEvents for Rec {
    fn emit(&self, e: AgentEvent) {
        self.v.lock().unwrap().push(e);
        self.cv.notify_all();
    }
}

impl Rec {
    fn all(&self) -> Vec<AgentEvent> {
        self.v.lock().unwrap().clone()
    }

    fn wait(&self, what: &str, f: impl Fn(&AgentEvent) -> bool) -> AgentEvent {
        let mut g = self.v.lock().unwrap();
        let end = std::time::Instant::now() + WAIT;
        loop {
            if let Some(e) = g.iter().find(|e| f(e)) {
                return e.clone();
            }
            let left = end.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "timed out waiting for {what}: {:?}", *g);
            g = self.cv.wait_timeout(g, left).unwrap().0;
        }
    }

    fn ended(&self, turn: &str) -> (AgentTurnOutcome, u32, Option<u64>) {
        let e = self.wait("turn ended", |e| {
            matches!(e, AgentEvent::Turn(AgentTurnEvent::Ended { turn_id, .. }) if turn_id == turn)
        });
        match e {
            AgentEvent::Turn(AgentTurnEvent::Ended {
                outcome,
                calls,
                tokens,
                ..
            }) => (outcome, calls, tokens),
            _ => unreachable!(),
        }
    }

    fn session_ended(&self, nth: usize) -> AgentSessionEndReason {
        let end = std::time::Instant::now() + WAIT;
        loop {
            let found: Vec<AgentSessionEndReason> = self
                .all()
                .iter()
                .filter_map(|e| match e {
                    AgentEvent::Session(AgentSessionEvent::Ended { reason, .. }) => Some(*reason),
                    _ => None,
                })
                .collect();
            if let Some(r) = found.get(nth) {
                return *r;
            }
            assert!(std::time::Instant::now() < end, "no session end #{nth}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn has_turn(&self, f: impl Fn(&AgentTurnEvent) -> bool) -> bool {
        self.all()
            .iter()
            .any(|e| matches!(e, AgentEvent::Turn(t) if f(t)))
    }
}

struct FakeWs(Mutex<Snapshot>);

impl WorkspaceView for FakeWs {
    fn snapshot(&self) -> CoreResult<Snapshot> {
        Ok(self.0.lock().unwrap().clone())
    }
}

fn entry(alias: &str, profile: &str, home: bool, agent: bool) -> CatalogEntry {
    CatalogEntry {
        alias: alias.into(),
        profile: profile.into(),
        name: alias.to_uppercase(),
        is_home: home,
        is_agent_workspace: agent,
    }
}

fn snapshot() -> Snapshot {
    Snapshot {
        catalog: vec![
            entry("sales", "p-sales", true, true),
            entry("finance", "p-finance", false, false),
        ],
        cross_workspace: false,
        budget: 10,
        agent_profile: Some("p-sales".into()),
        agent_host: Some("sales.cloud.databricks.com".into()),
    }
}

struct Approve(AtomicBool, AtomicUsize);

impl Approvals for Approve {
    fn redeem(&self, _: &str, _: Option<&str>) -> bool {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.load(Ordering::SeqCst)
    }
}

struct Env {
    preflight: Mutex<Result<(), PreflightFail>>,
}

impl AgentEnv for Env {
    fn cli_path(&self) -> Option<PathBuf> {
        Some(PathBuf::from("/opt/bottle/bin/databricks"))
    }
    fn var(&self, n: &str) -> Option<String> {
        match n {
            "HOME" => Some("/Users/t".into()),
            "AWS_SECRET_ACCESS_KEY" => Some("must-not-pass".into()),
            _ => None,
        }
    }
    fn preflight(&self, _: &str) -> Result<(), PreflightFail> {
        *self.preflight.lock().unwrap()
    }
    fn profile_state(&self, _: &str) -> AgentProfileState {
        AgentProfileState::Ready
    }
}

struct Shim;

impl ShimProvider for Shim {
    fn launch(&self, session_id: &str, _: &RunSecret) -> ShimLaunch {
        ShimLaunch {
            command: PathBuf::from("/Applications/Bottle.app/bottle"),
            args: vec!["--agent-mcp-shim".into()],
            env: vec![("BOTTLE_AGENT_SESSION".into(), session_id.into())],
        }
    }
}

struct Bin(Mutex<GenieCodeBinary>);

impl BinaryCheck for Bin {
    fn check(&self, _: bool) -> BinInfo {
        let s = *self.0.lock().unwrap();
        BinInfo {
            state: s,
            version: Some("0.5.0".into()),
            path: (s == GenieCodeBinary::Ready).then(|| PathBuf::from("/fake/genie")),
        }
    }
}

struct NoInstall;

impl Installer for NoInstall {
    fn install(&self) -> Result<(), InstallError> {
        Err(InstallError::Failed)
    }
}

// ---------- the rig ----------

struct Rig {
    host: Arc<SessionHost>,
    guard: Arc<FakeGuard>,
    net: Arc<FakeNet>,
    events: Arc<Rec>,
    ws: Arc<FakeWs>,
    spawner: Arc<FakeSpawner>,
    clock: SimClock,
    approve: Arc<Approve>,
    env: Arc<Env>,
    bin: Arc<Bin>,
    dir: PathBuf,
    cfg: SessionConfig,
}

fn tmpdir() -> PathBuf {
    let mut n = [0u8; 6];
    getrandom::fill(&mut n).unwrap();
    let p = std::env::temp_dir().join(format!(
        "r1-sess-{}",
        n.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

impl Rig {
    fn new() -> Self {
        let dir = tmpdir();
        let cfg = SessionConfig {
            home: dir.join("genie-home"),
            tmp_root: dir.join("tmp"),
            pid_file: dir.join("agent.pid"),
            tick: Duration::from_secs(3600),
        };
        std::fs::create_dir_all(&cfg.tmp_root).unwrap();
        let guard = Arc::new(FakeGuard::default());
        let net = Arc::new(FakeNet::default());
        let events = Arc::new(Rec::default());
        let ws = Arc::new(FakeWs(Mutex::new(snapshot())));
        let spawner = Arc::new(FakeSpawner::default());
        let clock = SimClock::new();
        let approve = Arc::new(Approve(AtomicBool::new(true), AtomicUsize::new(0)));
        let env = Arc::new(Env {
            preflight: Mutex::new(Ok(())),
        });
        let bin = Arc::new(Bin(Mutex::new(GenieCodeBinary::Ready)));
        let host = SessionHost::new(
            SessionDeps {
                guard: guard.clone(),
                net: net.clone(),
                events: events.clone(),
                clock: Arc::new(clock.clone()),
                wall: Arc::new(SimWallClock::at(1_700_000_000_000)),
                spawner: spawner.clone(),
                workspaces: ws.clone(),
                approvals: approve.clone(),
                env: env.clone(),
                shim: Arc::new(Shim),
                binary: bin.clone(),
                installer: Arc::new(NoInstall),
            },
            cfg.clone(),
        );
        Self {
            host,
            guard,
            net,
            events,
            ws,
            spawner,
            clock,
            approve,
            env,
            bin,
            dir,
            cfg,
        }
    }

    fn args(q: &str, mode: Option<&str>) -> AgentTurnSubmitArgs {
        let mut v = json!({ "mogId": MOG, "opId": "op-1", "question": q });
        if let Some(m) = mode {
            v["mode"] = json!(m);
        }
        serde_json::from_value(v).unwrap()
    }

    fn submit(&self, q: &str) -> Result<crate::types::AgentTurnAccepted, &'static str> {
        self.host.submit(&Self::args(q, None))
    }

    /// Submit, take the server, finish the handshake and the turn start.
    fn first_turn(&self, q: &str) -> (Arc<Server>, String) {
        let acc = self.submit(q).unwrap();
        let s = self.spawner.next();
        s.handshake();
        s.start_turn("st-1");
        (s, acc.turn_id)
    }

    fn wait_until(&self, what: &str, f: impl Fn() -> bool) {
        let start = std::time::Instant::now();
        while !f() {
            assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn answer_json(call: &str, ws: &str) -> String {
    json!({ "answer": "Churn is up", "sources": [{ "workspace": ws, "callId": call }] }).to_string()
}

fn record(turn: &str, call: &str, alias: &str, state: AgentCallState) -> CallRecord {
    CallRecord {
        call_id: call.into(),
        turn_id: turn.into(),
        alias: alias.into(),
        state,
    }
}

// ---------- start, handshake, GENIE_HOME ----------

#[test]
fn the_server_starts_on_the_first_turn_and_not_before() {
    let r = Rig::new();
    assert_eq!(r.spawner.count(), 0);
    assert!(!r.cfg.home.exists());
    assert_eq!(r.host.session_state().state, AgentSessionPhase::None);
}

#[test]
fn ac_2a_22_thread_start_is_ephemeral_and_never_asks_approval() {
    let r = Rig::new();
    let acc = r.submit("Compare churn in sales and finance").unwrap();
    assert!(acc.new_session);
    assert_eq!(acc.position, 0);
    assert_eq!(acc.session_id.len(), 32);
    assert_eq!(acc.catalog.len(), 2);
    assert!(!acc.cross_workspace);
    let s = r.spawner.next();
    let p = s.handshake();
    assert_eq!(p["ephemeral"], true);
    assert_eq!(p["approvalPolicy"], "never");
    // The thread cwd is a private temp folder that exists during the session.
    let cwd = PathBuf::from(p["cwd"].as_str().unwrap());
    assert!(cwd.starts_with(&r.cfg.tmp_root) && cwd.is_dir());
    r.events.wait("session started", |e| {
        matches!(e, AgentEvent::Session(AgentSessionEvent::Started { .. }))
    });
    let (_, tp) = s.recv_req("turn/start");
    assert_eq!(tp["threadId"], "th-1");
    assert_eq!(tp["input"][0]["type"], "text");
    assert!(tp["input"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Compare churn in sales and finance"));
    assert_eq!(tp["outputSchema"]["required"], json!(["answer", "sources"]));
}

fn bad_thread_start(mutate: impl Fn(&mut Value)) -> Rig {
    let r = Rig::new();
    let acc = r.submit("q").unwrap();
    let s = r.spawner.next();
    let (id, _) = s.recv_req("initialize");
    s.reply(id, json!({}));
    let _ = s.recv();
    let (id, _) = s.recv_req("thread/start");
    let mut ok = Server::thread_ok();
    mutate(&mut ok);
    s.reply(id, ok);
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::Lockdown);
    assert_eq!(r.events.ended(&acc.turn_id).0, AgentTurnOutcome::Lockdown);
    assert!(s.killed());
    assert!(!r.cfg.home.exists());
    r
}

#[test]
fn ac_2a_22_a_thread_that_is_not_ephemeral_ends_the_session_with_lockdown() {
    bad_thread_start(|v| v["thread"]["ephemeral"] = json!(false));
}

#[test]
fn ac_2a_22_another_cli_version_or_provider_or_policy_ends_the_session() {
    bad_thread_start(|v| v["thread"]["cliVersion"] = json!("0.6.1"));
    bad_thread_start(|v| {
        v["thread"]["modelProvider"] = json!("openai");
        v["modelProvider"] = json!("openai");
    });
    bad_thread_start(|v| v["approvalPolicy"] = json!("on-request"));
    bad_thread_start(|v| v["sandbox"]["type"] = json!("workspaceWrite"));
}

#[test]
fn the_start_rewrites_genie_home_and_spawns_with_a_clean_argv_and_env() {
    let r = Rig::new();
    std::fs::create_dir_all(&r.cfg.home).unwrap();
    std::fs::write(r.cfg.home.join("state_5.sqlite"), b"OLD TRANSCRIPT").unwrap();
    r.submit("q").unwrap();
    let s = r.spawner.next();
    assert!(!r.cfg.home.join("state_5.sqlite").exists());
    let cfg = std::fs::read_to_string(r.cfg.home.join("config.toml")).unwrap();
    assert!(cfg.contains("approval_policy = \"never\""));
    assert!(cfg.contains("tool_timeout_sec = 600"));
    assert!(cfg.contains("BOTTLE_AGENT_SESSION"));
    assert!(r.cfg.home.join("answer.schema.json").exists());
    assert_eq!(s.spec.args, vec!["app-server".to_owned()]);
    assert_eq!(s.spec.program, PathBuf::from("/fake/genie"));
    let names: Vec<&str> = s.spec.env.iter().map(|(k, _)| k.as_str()).collect();
    assert!(names.contains(&"GENIE_HOME") && names.contains(&"HOME"));
    assert!(!names.contains(&"AWS_SECRET_ACCESS_KEY"));
    assert!(s
        .spec
        .env
        .iter()
        .any(|(k, v)| k == "PATH" && v == "/usr/bin:/bin:/opt/bottle/bin"));
    assert!(r.guard.log().iter().any(|l| l.starts_with("bind:4000")));
    assert!(std::fs::read_to_string(&r.cfg.pid_file).unwrap() == "4000");
}

// ---------- queue (AC-2a.19 to 2a.21) ----------

#[test]
fn ac_2a_19_a_second_turn_waits_and_no_turn_start_goes_out_before_the_first_completes() {
    let r = Rig::new();
    let (s, t1) = r.first_turn("one");
    let a2 = r.submit("two").unwrap();
    assert_eq!(a2.position, 1);
    assert!(!a2.new_session);
    assert_eq!(r.submit("three").unwrap().position, 2);
    assert!(s.quiet(), "a second turn/start went out too early");
    s.complete(
        "st-1",
        "completed",
        Some(&json!({"answer":"a","sources":[]}).to_string()),
    );
    assert_eq!(r.events.ended(&t1).0, AgentTurnOutcome::Answered);
    let (_, p) = s.start_turn("st-2");
    assert!(p["input"][0]["text"].as_str().unwrap().contains("two"));
    assert!(r
        .events
        .has_turn(|e| matches!(e, AgentTurnEvent::Queued { position: 1, .. })));
}

#[test]
fn ac_2a_20_the_fourth_waiting_turn_is_refused_with_agent_queue_full() {
    let r = Rig::new();
    let _ = r.first_turn("live");
    for q in ["a", "b", "c"] {
        r.submit(q).unwrap();
    }
    assert_eq!(r.submit("d").unwrap_err(), "agent_queue_full");
    assert_eq!(r.host.session_state().queued, 3);
}

#[test]
fn ac_2a_21_stop_on_a_queued_turn_removes_it_and_sends_nothing() {
    let r = Rig::new();
    let (s, _) = r.first_turn("live");
    let q = r.submit("waiting").unwrap();
    r.host.stop(&q.turn_id).unwrap();
    assert_eq!(
        r.events.ended(&q.turn_id),
        (AgentTurnOutcome::Removed, 0, None)
    );
    assert!(s.quiet());
    assert_eq!(r.host.session_state().queued, 0);
    assert_eq!(r.host.stop(&q.turn_id).unwrap_err(), "agent_turn_unknown");
    assert!(!r.guard.log().iter().any(|l| l.contains(&q.turn_id)));
}

#[test]
fn six_turns_a_minute_is_the_rate_cap() {
    let r = Rig::new();
    let (s, _) = r.first_turn("1");
    s.complete("st-1", "completed", None);
    for i in 2..=6 {
        r.wait_until("idle", || {
            r.host.session_state().state == AgentSessionPhase::Idle
        });
        r.submit(&format!("{i}")).unwrap();
        s.start_turn(&format!("st-{i}"));
        s.complete(&format!("st-{i}"), "completed", None);
    }
    r.wait_until("idle", || {
        r.host.session_state().state == AgentSessionPhase::Idle
    });
    assert_eq!(r.submit("7").unwrap_err(), "agent_rate_limited");
    r.clock.advance(Duration::from_secs(61));
    assert!(r.submit("8").is_ok());
}

// ---------- stop, deadlines (AC-2a.9, 2a.10, 2a.10a to 2a.10c, 2a.11) ----------

#[test]
fn ac_2a_9_stop_on_the_live_turn_interrupts_and_cancels_the_calls() {
    let r = Rig::new();
    let (s, t) = r.first_turn("live");
    let queued = r.submit("next").unwrap();
    r.host.stop(&t).unwrap();
    let (_, p) = s.recv_req("turn/interrupt");
    assert_eq!(p, json!({ "threadId": "th-1", "turnId": "st-1" }));
    assert!(r.guard.has(&format!("stop:{t}:User")));
    s.complete("st-1", "interrupted", None);
    assert_eq!(r.events.ended(&t), (AgentTurnOutcome::Stopped, 0, None));
    // The queue goes on in the same session.
    let (_, p) = s.start_turn("st-2");
    assert!(p["input"][0]["text"].as_str().unwrap().contains("next"));
    assert_eq!(r.spawner.count(), 1);
    let _ = queued;
}

#[test]
fn ac_2a_10_still_working_shows_at_four_minutes() {
    let r = Rig::new();
    let (_s, t) = r.first_turn("slow");
    r.clock.advance(Duration::from_secs(239));
    r.host.tick();
    assert!(!r
        .events
        .has_turn(|e| matches!(e, AgentTurnEvent::Slow { .. })));
    r.clock.advance(Duration::from_secs(1));
    r.host.tick();
    r.events.wait(
        "slow",
        |e| matches!(e, AgentEvent::Turn(AgentTurnEvent::Slow { turn_id }) if *turn_id == t),
    );
    // Only once.
    r.clock.advance(Duration::from_secs(30));
    r.host.tick();
    let slows = r
        .events
        .all()
        .iter()
        .filter(|e| matches!(e, AgentEvent::Turn(AgentTurnEvent::Slow { .. })))
        .count();
    assert_eq!(slows, 1);
}

#[test]
fn ac_2a_10a_at_ten_minutes_interrupt_then_kill_the_process_after_five_seconds() {
    let r = Rig::new();
    let (s, t) = r.first_turn("long");
    r.clock.advance(Duration::from_secs(600));
    r.host.tick();
    s.recv_req("turn/interrupt");
    // A timeout keeps the open Genie calls polling.
    assert!(r.guard.has(&format!("stop:{t}:Timeout")));
    assert!(!s.killed());
    r.clock.advance(Duration::from_secs(4));
    r.host.tick();
    assert!(!s.killed());
    r.clock.advance(Duration::from_secs(1));
    r.host.tick();
    assert_eq!(
        r.events.session_ended(0),
        AgentSessionEndReason::HardTimeout
    );
    assert_eq!(r.events.ended(&t), (AgentTurnOutcome::Timeout, 0, None));
    assert!(s.killed());
    assert!(!r.cfg.home.exists());
}

#[test]
fn ac_2a_10a_an_answer_to_the_interrupt_in_time_keeps_the_session() {
    let r = Rig::new();
    let (s, t) = r.first_turn("long");
    r.clock.advance(Duration::from_secs(600));
    r.host.tick();
    s.recv_req("turn/interrupt");
    s.complete("st-1", "interrupted", None);
    assert_eq!(r.events.ended(&t).0, AgentTurnOutcome::Timeout);
    r.clock.advance(Duration::from_secs(10));
    r.host.tick();
    assert!(!s.killed());
    assert_ne!(r.host.session_state().state, AgentSessionPhase::None);
}

#[test]
fn ac_2a_10b_offline_interrupts_at_once_and_the_calls_keep_polling() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    r.net.set(false);
    s.recv_req("turn/interrupt");
    assert!(r.guard.has(&format!("stop:{t}:Offline")));
    s.complete("st-1", "interrupted", None);
    // AC-2a.11: no Genie call, so the brain falls back to the direct path.
    assert_eq!(r.events.ended(&t), (AgentTurnOutcome::Offline, 0, None));
}

#[test]
fn ac_2a_10b_a_turn_that_starts_while_offline_ends_at_once_and_sends_nothing() {
    let r = Rig::new();
    r.net.set(false);
    let acc = r.submit("q").unwrap();
    let s = r.spawner.next();
    s.handshake();
    assert_eq!(
        r.events.ended(&acc.turn_id),
        (AgentTurnOutcome::Offline, 0, None)
    );
    assert!(s.quiet());
    assert!(!r.guard.log().iter().any(|l| l.starts_with("begin:")));
}

#[test]
fn ac_2a_10c_a_waiting_for_network_error_interrupts_at_once() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    s.notify(
        "error",
        json!({ "error": { "message": "stream error: waiting for network" }, "willRetry": true }),
    );
    s.recv_req("turn/interrupt");
    assert!(r.guard.has(&format!("stop:{t}:Offline")));
    s.complete("st-1", "interrupted", None);
    assert_eq!(r.events.ended(&t).0, AgentTurnOutcome::Offline);
}

#[test]
fn ac_2a_10c_the_same_line_on_stderr_does_the_same() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    s.stderr("WARN retrying: waiting for network\n");
    s.recv_req("turn/interrupt");
    s.complete("st-1", "interrupted", None);
    assert_eq!(r.events.ended(&t).0, AgentTurnOutcome::Offline);
}

#[test]
fn ac_2a_11_a_failed_or_timed_out_turn_with_no_call_reports_zero_calls() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    s.complete("st-1", "failed", None);
    assert_eq!(r.events.ended(&t), (AgentTurnOutcome::Failed, 0, None));
    // The turn ended, the session stays: the brain asks the home workspace directly.
    assert!(!s.killed());
}

#[test]
fn a_401_on_stderr_gives_signed_out_and_a_403_on_responses_gives_not_eligible() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    s.stderr("HTTP 401 from model, run auth login\n");
    s.recv_req("turn/interrupt");
    s.complete("st-1", "interrupted", None);
    assert_eq!(r.events.ended(&t).0, AgentTurnOutcome::SignedOut);
    r.wait_until("idle", || {
        r.host.session_state().state == AgentSessionPhase::Idle
    });
    r.submit("again").unwrap();
    s.start_turn("st-2");
    s.stderr("POST /responses: 403 forbidden\n");
    s.recv_req("turn/interrupt");
    s.complete("st-2", "interrupted", None);
    r.wait_until("not eligible", || {
        r.host.status(false).agent_profile_state == AgentProfileState::NotEligible
    });
    // The cache is cleared by a recheck.
    assert_eq!(
        r.host.status(true).agent_profile_state,
        AgentProfileState::Ready
    );
}

// ---------- tripwires (AC-2a.13, 2a.23, 2a.24) ----------

#[test]
fn ac_2a_23_a_server_request_gets_a_refusal_then_the_session_ends_with_lockdown() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    s.send(
        json!({ "method": "item/fileChange/requestApproval", "id": 0,
                   "params": { "itemId": "x" } }),
    );
    let refusal = s.recv();
    assert_eq!(refusal["id"], 0);
    assert!(refusal["error"].is_object(), "the request needs a refusal");
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::Lockdown);
    assert_eq!(r.events.ended(&t).0, AgentTurnOutcome::Lockdown);
    assert!(r.guard.has(&format!("stop:{t}:Lockdown")));
    assert!(s.killed());
    assert!(!r.cfg.home.exists());
}

#[test]
fn ac_2a_24_remote_control_other_than_disabled_ends_the_session() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    s.notify(
        "remoteControl/status/changed",
        json!({ "status": "disabled" }),
    );
    assert_ne!(r.host.session_state().state, AgentSessionPhase::None);
    s.notify(
        "remoteControl/status/changed",
        json!({ "status": "connected" }),
    );
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::Lockdown);
    assert_eq!(r.events.ended(&t).0, AgentTurnOutcome::Lockdown);
}

#[test]
fn ac_2a_13_a_command_a_file_change_a_web_search_or_a_foreign_tool_ends_the_session() {
    for item in [
        json!({ "type": "commandExecution", "command": "curl x" }),
        json!({ "type": "fileChange" }),
        json!({ "type": "webSearch" }),
        json!({ "type": "mcpToolCall", "server": "evil", "tool": "ask_genie" }),
        json!({ "type": "mcpToolCall", "server": "codex", "tool": "exec" }),
    ] {
        let r = Rig::new();
        let (s, t) = r.first_turn("q");
        s.notify("item/started", json!({ "item": item }));
        assert_eq!(r.events.ended(&t).0, AgentTurnOutcome::Lockdown, "{item}");
        assert_eq!(r.events.session_ended(0), AgentSessionEndReason::Lockdown);
    }
}

#[test]
fn ac_2a_13_the_bottle_tool_the_codex_resource_tools_and_plain_items_are_allowed() {
    let r = Rig::new();
    let (s, _) = r.first_turn("q");
    for item in [
        json!({ "type": "mcpToolCall", "server": "bottle", "tool": "ask_genie" }),
        json!({ "type": "mcpToolCall", "server": "codex", "tool": "read_mcp_resource" }),
        json!({ "type": "reasoning" }),
        json!({ "type": "userMessage" }),
        json!({ "type": "agentMessage", "text": "" }),
    ] {
        s.notify("item/completed", json!({ "item": item }));
    }
    s.notify("thread/status/changed", json!({}));
    r.host.tick();
    assert_eq!(r.host.session_state().state, AgentSessionPhase::Busy);
    assert!(!s.killed());
}

#[test]
fn a_line_over_the_cap_ends_the_session_as_a_crash() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    let big = "x".repeat(super::appserver::MAX_LINE_BYTES + 10);
    s.send(json!({ "method": "x", "params": { "t": big } }));
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::Crashed);
    assert_eq!(r.events.ended(&t).0, AgentTurnOutcome::Failed);
}

// ---------- session end (AC-2a.25, 2a.26) ----------

#[test]
fn ac_2a_25_new_session_interrupts_kills_and_deletes_genie_home() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    let host = Arc::clone(&r.host);
    let h = std::thread::spawn(move || {
        crate::ipc_gateway::GatewayComponents::call(
            &*host,
            "agent_session_end",
            json!({ "reason": "new_session" }),
        )
    });
    s.recv_req("turn/interrupt");
    s.complete("st-1", "interrupted", None);
    h.join().unwrap().unwrap();
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::NewSession);
    assert_eq!(r.events.ended(&t).0, AgentTurnOutcome::SessionEnded);
    assert!(s.killed());
    assert!(!r.cfg.home.exists());
    assert!(!r.cfg.pid_file.exists());
    assert_eq!(std::fs::read_dir(&r.cfg.tmp_root).unwrap().count(), 0);
    assert!(r
        .guard
        .log()
        .iter()
        .any(|l| l.ends_with(":false") && l.starts_with("close:")));
    assert_eq!(r.host.session_state().state, AgentSessionPhase::None);
}

#[test]
fn ac_2a_25_an_unanswered_interrupt_at_session_end_kills_after_five_seconds() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    let host = Arc::clone(&r.host);
    let h = std::thread::spawn(move || host.end_blocking(AgentSessionEndReason::Quit));
    s.recv_req("turn/interrupt");
    assert!(!s.killed());
    r.clock.advance(Duration::from_secs(5));
    r.host.tick();
    h.join().unwrap();
    assert!(s.killed());
    assert_eq!(r.events.ended(&t).0, AgentTurnOutcome::SessionEnded);
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::Quit);
}

#[test]
fn ac_2a_25_wipe_ends_the_session_drops_the_ledger_at_once_and_deletes_genie_home() {
    let r = Rig::new();
    let (s, _t) = r.first_turn("q");
    s.complete("st-1", "completed", Some("{}"));
    r.wait_until("idle", || {
        r.host.session_state().state == AgentSessionPhase::Idle
    });
    AgentWipeHook::end_for_wipe(&*r.host);
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::Wipe);
    assert!(r
        .guard
        .log()
        .iter()
        .any(|l| l.ends_with(":true") && l.starts_with("close:")));
    assert!(!r.cfg.home.exists());
    assert!(s.killed());
}

#[test]
fn ac_2a_25_wipe_with_no_session_still_deletes_a_leftover_genie_home() {
    let r = Rig::new();
    std::fs::create_dir_all(&r.cfg.home).unwrap();
    std::fs::write(r.cfg.home.join("thread_history_1.sqlite"), b"x").unwrap();
    AgentWipeHook::end_for_wipe(&*r.host);
    assert!(!r.cfg.home.exists());
}

#[test]
fn ac_2a_25_a_profile_change_waits_for_the_live_turn_then_ends_the_session() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    r.ws.0.lock().unwrap().agent_profile = Some("p-finance".into());
    r.clock.advance(Duration::from_secs(2));
    r.host.tick();
    assert!(!s.killed(), "the live turn must not be cut");
    assert!(s.quiet());
    s.complete("st-1", "completed", Some("{}"));
    assert_eq!(r.events.ended(&t).0, AgentTurnOutcome::Answered);
    assert_eq!(
        r.events.session_ended(0),
        AgentSessionEndReason::ProfileChanged
    );
    assert!(s.killed());
    assert!(!r.cfg.home.exists());
}

#[test]
fn ac_2a_25_a_d1_change_and_a_catalog_change_end_the_session_after_the_turn() {
    let r = Rig::new();
    let (s, _) = r.first_turn("q");
    r.ws.0.lock().unwrap().cross_workspace = true;
    r.clock.advance(Duration::from_secs(2));
    r.host.tick();
    s.complete("st-1", "completed", Some("{}"));
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::D1Changed);

    let r = Rig::new();
    let (s, _) = r.first_turn("q");
    // A workspace switched on or off changes the catalog.
    r.ws.0.lock().unwrap().catalog.pop();
    r.clock.advance(Duration::from_secs(2));
    r.host.tick();
    s.complete("st-1", "completed", Some("{}"));
    assert_eq!(
        r.events.session_ended(0),
        AgentSessionEndReason::CatalogChanged
    );
}

#[test]
fn ac_2a_25_an_idle_session_with_a_changed_setting_ends_at_the_next_tick() {
    let r = Rig::new();
    let (s, _) = r.first_turn("q");
    s.complete("st-1", "completed", Some("{}"));
    r.wait_until("idle", || {
        r.host.session_state().state == AgentSessionPhase::Idle
    });
    r.ws.0.lock().unwrap().catalog.pop();
    r.clock.advance(Duration::from_secs(2));
    r.host.tick();
    assert_eq!(
        r.events.session_ended(0),
        AgentSessionEndReason::CatalogChanged
    );
    assert!(s.killed());
}

#[test]
fn a_new_turn_after_a_setting_change_starts_a_new_session() {
    let r = Rig::new();
    let (s, _) = r.first_turn("q");
    s.complete("st-1", "completed", Some("{}"));
    r.wait_until("idle", || {
        r.host.session_state().state == AgentSessionPhase::Idle
    });
    r.ws.0.lock().unwrap().cross_workspace = true;
    let acc = r.submit("after").unwrap();
    assert!(acc.new_session);
    assert!(s.killed());
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::D1Changed);
    let s2 = r.spawner.next();
    s2.handshake();
    s2.recv_req("turn/start");
    assert_eq!(r.spawner.count(), 2);
}

#[test]
fn ac_2a_26_no_file_under_genie_home_or_the_cwd_holds_the_prompt_canary() {
    const CANARY: &str = "CANARY-7f3a91c2-do-not-store";
    fn scan(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                scan(&p, found);
            } else if std::fs::read(&p)
                .map(|b| b.windows(CANARY.len()).any(|w| w == CANARY.as_bytes()))
                .unwrap_or(false)
            {
                found.push(p);
            }
        }
    }
    let r = Rig::new();
    let acc = r.submit(&format!("what is {CANARY}?")).unwrap();
    let s = r.spawner.next();
    s.handshake();
    let (id, p) = s.recv_req("turn/start");
    s.reply(id, json!({ "turn": { "id": "st-1" } }));
    // The question travels only in the JSON-RPC prompt, which the fake server holds in memory.
    assert!(p["input"][0]["text"].as_str().unwrap().contains(CANARY));
    let mut found = Vec::new();
    scan(&r.cfg.home, &mut found);
    scan(&r.cfg.tmp_root, &mut found);
    assert!(
        found.is_empty(),
        "canary on disk during the session: {found:?}"
    );
    let host = Arc::clone(&r.host);
    let h = std::thread::spawn(move || host.end_blocking(AgentSessionEndReason::NewSession));
    s.recv_req("turn/interrupt");
    s.complete("st-1", "interrupted", None);
    h.join().unwrap();
    scan(&r.dir, &mut found);
    assert!(
        found.is_empty(),
        "canary on disk after the session: {found:?}"
    );
    assert!(!r.cfg.home.exists());
    let _ = acc;
}

// ---------- tokens (AC-2a.29) ----------

#[test]
fn ac_2a_29_tokens_are_the_sum_of_last_input_plus_output_between_started_and_completed() {
    let r = Rig::new();
    let acc = r.submit("q").unwrap();
    let s = r.spawner.next();
    s.handshake();
    let (id, _) = s.recv_req("turn/start");
    s.reply(id, json!({ "turn": { "id": "st-1" } }));
    // Before turn/started: not counted.
    s.usage("st-1", 1000, 1000);
    s.notify("turn/started", json!({ "turn": { "id": "st-1" } }));
    s.usage("st-1", 15_658, 5);
    s.usage("st-1", 16_000, 300);
    s.usage("st-1", 17_000, 40);
    // The total field must never be used.
    s.complete(
        "st-1",
        "completed",
        Some(&json!({"answer":"a","sources":[]}).to_string()),
    );
    let (o, _, tokens) = r.events.ended(&acc.turn_id);
    assert_eq!(o, AgentTurnOutcome::Answered);
    assert_eq!(tokens, Some(15_658 + 5 + 16_000 + 300 + 17_000 + 40));
    let res = r.host.turn_result(&acc.turn_id).unwrap();
    assert_eq!(res.usage.unwrap().input_tokens, 15_658 + 16_000 + 17_000);
    r.wait_until("session total", || r.host.session_state().tokens == 49_003);
    assert_eq!(r.host.session_state().turns, 1);
}

// ---------- crash (AC-2a.30) ----------

#[test]
fn ac_2a_30_a_crash_fails_the_live_turn_and_the_next_one_starts_a_new_session_once() {
    let r = Rig::new();
    let (s1, t1) = r.first_turn("one");
    let q2 = r.submit("two").unwrap();
    let q3 = r.submit("three").unwrap();
    s1.die();
    assert_eq!(r.events.ended(&t1).0, AgentTurnOutcome::Failed);
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::Crashed);
    // A new session for the next waiting turn.
    let s2 = r.spawner.next();
    s2.handshake();
    let (_, p) = s2.start_turn("st-1");
    assert!(p["input"][0]["text"].as_str().unwrap().contains("two"));
    assert_eq!(r.spawner.count(), 2);
    // A second crash: the waiting turn goes to the direct path (ended with 0 calls).
    s2.die();
    assert_eq!(r.events.ended(&q2.turn_id).0, AgentTurnOutcome::Failed);
    assert_eq!(
        r.events.ended(&q3.turn_id),
        (AgentTurnOutcome::Failed, 0, None)
    );
    assert_eq!(r.events.session_ended(1), AgentSessionEndReason::Crashed);
    assert_eq!(r.spawner.count(), 2, "no third start");
    assert!(!r.cfg.home.exists());
}

#[test]
fn a_crash_with_nothing_waiting_just_ends_the_session() {
    let r = Rig::new();
    let (s, t) = r.first_turn("one");
    s.die();
    assert_eq!(r.events.ended(&t).0, AgentTurnOutcome::Failed);
    assert_eq!(r.host.session_state().state, AgentSessionPhase::None);
    assert_eq!(r.spawner.count(), 1);
}

#[test]
fn an_answered_turn_resets_the_crash_count() {
    let r = Rig::new();
    let (s1, _) = r.first_turn("one");
    let q2 = r.submit("two").unwrap();
    s1.die();
    let s2 = r.spawner.next();
    s2.handshake();
    s2.start_turn("st-1");
    s2.complete("st-1", "completed", Some("{}"));
    assert_eq!(r.events.ended(&q2.turn_id).0, AgentTurnOutcome::Answered);
    r.wait_until("idle", || {
        r.host.session_state().state == AgentSessionPhase::Idle
    });
    let q3 = r.submit("three").unwrap();
    let q4 = r.submit("four").unwrap();
    s2.start_turn("st-2");
    s2.die();
    assert_eq!(r.events.ended(&q3.turn_id).0, AgentTurnOutcome::Failed);
    // One restart again, because the count was reset.
    let s3 = r.spawner.next();
    s3.handshake();
    let (_, p) = s3.recv_req("turn/start");
    assert!(p["input"][0]["text"].as_str().unwrap().contains("four"));
    let _ = q4;
}

// ---------- idle, taint ----------

#[test]
fn the_session_ends_after_thirty_minutes_with_no_turn() {
    let r = Rig::new();
    let (s, _) = r.first_turn("q");
    s.complete("st-1", "completed", Some("{}"));
    r.wait_until("idle", || {
        r.host.session_state().state == AgentSessionPhase::Idle
    });
    r.clock.advance(Duration::from_secs(30 * 60 - 1));
    r.host.tick();
    assert_ne!(r.host.session_state().state, AgentSessionPhase::None);
    r.clock.advance(Duration::from_secs(1));
    r.host.tick();
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::Idle);
    assert!(s.killed());
    assert!(!r.cfg.home.exists());
}

#[test]
fn a_running_turn_is_never_cut_by_the_idle_rule() {
    let r = Rig::new();
    let (s, _) = r.first_turn("q");
    r.clock.advance(Duration::from_secs(9 * 60));
    r.host.tick();
    // 9 min is past the slow mark, below the hard stop and the idle limit does not apply.
    assert!(!s.killed());
}

#[test]
fn r_15_a_tainted_session_restarts_for_a_new_question_but_not_for_a_follow_up() {
    let r = Rig::new();
    let (s1, _) = r.first_turn("one");
    s1.complete("st-1", "completed", Some("{}"));
    r.wait_until("idle", || {
        r.host.session_state().state == AgentSessionPhase::Idle
    });
    r.guard.tainted.store(true, Ordering::SeqCst);
    // A follow-up uses the open session.
    let f = r
        .host
        .submit(&Rig::args("more", Some("follow_up")))
        .unwrap();
    assert!(!f.new_session);
    s1.start_turn("st-2");
    s1.complete("st-2", "completed", Some("{}"));
    r.wait_until("idle", || {
        r.host.session_state().state == AgentSessionPhase::Idle
    });
    assert!(!s1.killed());
    // A new cross-workspace question gets a new session.
    let n = r
        .host
        .submit(&Rig::args("new", Some("ask_others")))
        .unwrap();
    assert!(n.new_session);
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::Tainted);
    assert!(s1.killed());
    let s2 = r.spawner.next();
    s2.handshake();
    let (_, p) = s2.recv_req("turn/start");
    // ask_others leaves home out of the prompt.
    assert!(!p["input"][0]["text"]
        .as_str()
        .unwrap()
        .contains("- sales ="));
}

#[test]
fn a_queued_new_question_restarts_a_tainted_session_when_its_turn_comes() {
    let r = Rig::new();
    let (s1, _) = r.first_turn("one");
    let q = r.submit("two").unwrap();
    r.guard.tainted.store(true, Ordering::SeqCst);
    s1.complete("st-1", "completed", Some("{}"));
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::Tainted);
    let s2 = r.spawner.next();
    s2.handshake();
    s2.recv_req("turn/start");
    let _ = q;
}

// ---------- results (AC-2a.12, 2a.28 at the host side) ----------

#[test]
fn a_cited_answered_call_of_this_or_an_earlier_turn_gives_a_note() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    *r.guard.calls.lock().unwrap() = vec![
        record(
            "older-turn",
            "c-00000001",
            "finance",
            AgentCallState::Answered,
        ),
        record(&t, "c-00000002", "sales", AgentCallState::Answered),
    ];
    let text = json!({ "answer": "x".repeat(2000), "sources": [
        { "workspace": "finance", "callId": "c-00000001" },
        { "workspace": "sales", "callId": "c-00000002" } ] })
    .to_string();
    s.complete("st-1", "completed", Some(&text));
    let (o, calls, _) = r.events.ended(&t);
    assert_eq!((o, calls), (AgentTurnOutcome::Answered, 1));
    let res = r.host.turn_result(&t).unwrap();
    assert_eq!(res.answer_kind, AgentAnswerKind::Note);
    assert_eq!(res.answer.unwrap().chars().count(), 1200);
    assert_eq!(res.sources.len(), 2);
    // The source of an earlier turn keeps its own turn id.
    assert_eq!(res.sources[0].turn_id, "older-turn");
}

#[test]
fn an_unknown_call_id_a_wrong_workspace_or_a_failed_call_hides_the_summary() {
    for (call, ws, state) in [
        ("c-deadbeef", "sales", AgentCallState::Answered),
        ("c-00000002", "finance", AgentCallState::Answered),
        ("c-00000002", "sales", AgentCallState::Failed),
    ] {
        let r = Rig::new();
        let (s, t) = r.first_turn("q");
        *r.guard.calls.lock().unwrap() = vec![record(&t, "c-00000002", "sales", state)];
        s.complete("st-1", "completed", Some(&answer_json(call, ws)));
        r.events.ended(&t);
        let res = r.host.turn_result(&t).unwrap();
        assert_eq!(res.answer_kind, AgentAnswerKind::Hidden, "{call} {ws}");
        assert!(res.answer.is_none() && res.sources.is_empty());
    }
}

#[test]
fn a_final_message_that_is_not_the_schema_json_or_is_over_32_kb_is_hidden() {
    for text in [
        "just words".to_owned(),
        json!({"answer": 5, "sources": []}).to_string(),
        json!({"answer": "x".repeat(40_000), "sources": []}).to_string(),
    ] {
        let r = Rig::new();
        let (s, t) = r.first_turn("q");
        s.complete("st-1", "completed", Some(&text));
        r.events.ended(&t);
        assert_eq!(
            r.host.turn_result(&t).unwrap().answer_kind,
            AgentAnswerKind::Hidden
        );
    }
}

#[test]
fn the_answers_leave_memory_ten_minutes_after_the_session_ends() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    s.complete("st-1", "completed", Some("{}"));
    r.events.ended(&t);
    r.host.end_blocking(AgentSessionEndReason::NewSession);
    assert!(r.host.turn_result(&t).is_some());
    r.clock.advance(Duration::from_secs(10 * 60));
    r.host.tick();
    assert!(r.host.turn_result(&t).is_none());
}

// ---------- submit checks ----------

#[test]
fn a_submit_with_no_latch_and_no_ticket_is_refused_and_starts_nothing() {
    let r = Rig::new();
    r.approve.0.store(false, Ordering::SeqCst);
    assert_eq!(r.submit("q").unwrap_err(), "ask_not_approved");
    assert_eq!(r.spawner.count(), 0);
    assert!(!r.cfg.home.exists());
}

#[test]
fn a_binary_that_is_missing_or_not_pinned_uses_up_no_approval() {
    let r = Rig::new();
    *r.bin.0.lock().unwrap() = GenieCodeBinary::NotInstalled;
    assert_eq!(r.submit("q").unwrap_err(), "genie_code_missing");
    *r.bin.0.lock().unwrap() = GenieCodeBinary::WrongVersion;
    assert_eq!(r.submit("q").unwrap_err(), "genie_code_version");
    *r.bin.0.lock().unwrap() = GenieCodeBinary::UnsupportedOs;
    assert_eq!(r.submit("q").unwrap_err(), "genie_code_missing");
    assert_eq!(r.approve.1.load(Ordering::SeqCst), 0);
}

#[test]
fn the_profile_preflight_gives_fixed_codes_and_uses_up_no_approval() {
    let r = Rig::new();
    for (fail, code) in [
        (PreflightFail::NotU2m, "agent_profile_unsupported"),
        (PreflightFail::SignedOut, "agent_signed_out"),
        (PreflightFail::Offline, "agent_offline"),
        (PreflightFail::NotEligible, "agent_not_eligible"),
    ] {
        *r.env.preflight.lock().unwrap() = Err(fail);
        assert_eq!(r.submit("q").unwrap_err(), code);
    }
    assert_eq!(r.approve.1.load(Ordering::SeqCst), 0);
    assert_eq!(r.spawner.count(), 0);
}

#[test]
fn a_follow_up_with_no_open_session_is_unavailable() {
    let r = Rig::new();
    let e = r
        .host
        .submit(&Rig::args("q", Some("follow_up")))
        .unwrap_err();
    assert_eq!(e, "agent_session_unavailable");
}

#[test]
fn a_spawn_failure_leaves_no_session_and_no_files() {
    let r = Rig::new();
    r.spawner.fail.store(true, Ordering::SeqCst);
    assert_eq!(r.submit("q").unwrap_err(), "agent_failed");
    assert!(!r.cfg.home.exists());
    assert!(r
        .guard
        .log()
        .iter()
        .any(|l| l.starts_with("close:") && l.ends_with(":true")));
    assert_eq!(r.host.session_state().state, AgentSessionPhase::None);
}

#[test]
fn the_status_reports_the_binary_the_profile_and_the_pin() {
    let r = Rig::new();
    let st = r.host.status(false);
    assert_eq!(st.binary, GenieCodeBinary::Ready);
    assert_eq!(st.pinned, "0.5.0");
    assert_eq!(st.agent_profile.as_deref(), Some("p-sales"));
    assert_eq!(st.agent_profile_state, AgentProfileState::Ready);
    r.ws.0.lock().unwrap().agent_profile = None;
    assert_eq!(
        r.host.status(false).agent_profile_state,
        AgentProfileState::NoHome
    );
    let e = crate::ipc_gateway::GatewayComponents::call(
        &*r.host,
        "genie_code_install",
        json!({ "consent": true }),
    )
    .unwrap_err();
    assert!(matches!(
        e,
        crate::error::CoreError::IpcRefused {
            code: "install_failed"
        }
    ));
}

#[test]
fn the_gateway_commands_round_trip_through_json() {
    let r = Rig::new();
    let call =
        |c: &'static str, a: Value| crate::ipc_gateway::GatewayComponents::call(&*r.host, c, a);
    let acc = call(
        "agent_turn_submit",
        json!({ "mogId": MOG, "opId": "o", "question": "Compare churn" }),
    )
    .unwrap();
    assert_eq!(acc["position"], 0);
    let tid = acc["turnId"].as_str().unwrap().to_owned();
    let s = r.spawner.next();
    s.handshake();
    s.start_turn("st-1");
    let st = call("agent_session_state", json!({})).unwrap();
    assert_eq!(st["state"], "busy");
    assert!(matches!(
        call("agent_turn_result", json!({ "turnId": tid })),
        Err(crate::error::CoreError::IpcRefused {
            code: "agent_turn_unknown"
        })
    ));
    call("agent_turn_stop", json!({ "turnId": tid })).unwrap();
    s.recv_req("turn/interrupt");
    s.complete("st-1", "interrupted", None);
    assert_eq!(r.events.ended(&tid).0, AgentTurnOutcome::Stopped);
    assert!(call("agent_turn_stop", json!({ "turnId": "0".repeat(32) })).is_err());
    assert!(call("agent_session_end", json!({ "reason": "bogus" })).is_err());
}

#[test]
fn nothing_in_a_debug_output_of_the_host_or_the_events_holds_the_question() {
    let r = Rig::new();
    let acc = r.submit("SECRET-QUESTION-TEXT").unwrap();
    let s = r.spawner.next();
    s.handshake();
    s.start_turn("st-1");
    s.complete(
        "st-1",
        "completed",
        Some(&json!({"answer":"SECRET-ANSWER","sources":[]}).to_string()),
    );
    r.events.ended(&acc.turn_id);
    let shown = format!(
        "{:?} {:?} {:?} {:?}",
        r.host,
        r.events.all(),
        acc,
        r.host.session_state()
    );
    assert!(!shown.contains("SECRET-QUESTION"), "{shown}");
    assert!(!shown.contains("SECRET-ANSWER"), "{shown}");
    let res = r.host.turn_result(&acc.turn_id).unwrap();
    assert!(!format!("{res:?}").contains("SECRET-ANSWER"));
}

// ---------- the recorded genie 0.5.0 transcripts ----------

const FIXTURE_TURN: &str =
    include_str!("../../../tests/fixtures/genie-0.5.0/appserver-ephemeral-turn.jsonl");
const FIXTURE_APPROVAL: &str =
    include_str!("../../../tests/fixtures/genie-0.5.0/appserver-approval-request.jsonl");

#[test]
fn fixture_folder_name_follows_the_pin() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("genie-{}", crate::agent_run::pin::GENIE_PINNED));
    assert!(dir.is_dir(), "no fixtures for the pinned version: {dir:?}");
    assert!(dir.join("README.md").is_file());
}

#[test]
fn fixture_the_recorded_turn_replays_to_an_answered_turn_with_the_recorded_tokens() {
    let lines: Vec<Value> = FIXTURE_TURN
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // The tokens that the recording reports: the sum of `last` input and output.
    let want: u64 = lines
        .iter()
        .filter(|m| m["method"] == "thread/tokenUsage/updated")
        .map(|m| {
            m["params"]["tokenUsage"]["last"]["inputTokens"]
                .as_u64()
                .unwrap()
                + m["params"]["tokenUsage"]["last"]["outputTokens"]
                    .as_u64()
                    .unwrap()
        })
        .sum();
    assert!(want > 0);
    let r = Rig::new();
    let acc = r.submit("recorded").unwrap();
    let s = r.spawner.next();
    // The recording was made with the same request ids as Bottle uses: 1, 2 and 3.
    s.recv_req("initialize");
    s.send(lines[0].clone());
    s.send(lines[1].clone());
    assert_eq!(s.recv()["method"], "initialized");
    s.recv_req("thread/start");
    for l in &lines[2..6] {
        s.send(l.clone());
    }
    s.recv_req("turn/start");
    for l in &lines[6..] {
        s.send(l.clone());
    }
    let (o, calls, tokens) = r.events.ended(&acc.turn_id);
    assert_eq!(
        (o, calls, tokens),
        (AgentTurnOutcome::Answered, 0, Some(want))
    );
    // The final message of the recording is plain text, not the schema JSON: hidden.
    assert_eq!(
        r.host.turn_result(&acc.turn_id).unwrap().answer_kind,
        AgentAnswerKind::Hidden
    );
}

#[test]
fn fixture_the_recorded_approval_request_is_refused_and_ends_the_session() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    let mut b = FIXTURE_APPROVAL.trim().as_bytes().to_vec();
    b.push(b'\n');
    if let Some(tx) = s.out.lock().unwrap().as_ref() {
        tx.send(b).unwrap();
    }
    let refusal = s.recv();
    assert_eq!(refusal["id"], 0);
    assert!(refusal["error"].is_object());
    assert_eq!(r.events.session_ended(0), AgentSessionEndReason::Lockdown);
    assert_eq!(r.events.ended(&t).0, AgentTurnOutcome::Lockdown);
}

#[test]
fn the_third_refusal_interrupts_the_turn_and_ends_it_with_refusals() {
    let r = Rig::new();
    let (s, t) = r.first_turn("q");
    let hook = r.host.refusal_hook();
    // Another turn id is ignored.
    hook("not-the-live-turn");
    assert!(s.quiet());
    hook(&t);
    s.recv_req("turn/interrupt");
    assert!(r.guard.has(&format!("stop:{t}:Failed")));
    s.complete("st-1", "interrupted", None);
    assert_eq!(r.events.ended(&t), (AgentTurnOutcome::Refusals, 0, None));
    // The session stays for the next turn.
    assert!(!s.killed());
}
