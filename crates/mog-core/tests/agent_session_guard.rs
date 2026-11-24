//! Slice 2a integration (R1 + R2): the real session host (R1) with an in-process fake
//! `genie app-server`, the real `AgentGuard` (R2) on a real unix socket, and the real shim
//! function in this process. R8, R6 and the clocks are fakes. No process starts, no network, no
//! model, no profile. The questions and answers here are canaries only.
#![cfg(unix)]

use std::collections::VecDeque;
use std::io::{BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::agent_guard::AuditActionLog;
use mog_core::agent_run::guard::peer::own_parent_pid;
use mog_core::agent_run::guard::session_log::SessionUuidLog;
use mog_core::agent_run::guard::shim::{self, ShimOpts};
use mog_core::agent_run::guard::{AgentGuard, GuardOptions, Sleeper};
use mog_core::agent_run::seam::{
    AgentEvent, CatalogEntry, GuardDeps, NetWatch, RunEvents, RunSecret, SessionGuard, SHIM_ARG,
};
use mog_core::agent_run::session::approval::Approvals;
use mog_core::agent_run::session::genie_bin::BinInfo;
use mog_core::agent_run::session::install::InstallError;
use mog_core::agent_run::session::{
    AgentEnv, BinaryCheck, Installer, PreflightFail, ServerIo, ServerProc, SessionConfig,
    SessionDeps, SessionHost, ShimLaunch, ShimProvider, Snapshot, SpawnSpec, Spawner,
    WorkspaceView,
};
use mog_core::audit::{AuditLog, AuditRetention, MemoryAuditStore};
use mog_core::clock::{SimClock, SimWallClock};
use mog_core::error::CoreResult;
use mog_core::ipc::AgentTurnSubmitArgs;
use mog_core::mcp_transport::{McpCall, McpReply, McpTransport};
use mog_core::types::{
    AgentAnswerKind, AgentCallState, AgentProfileState, AgentSessionEndReason, AgentSessionEvent,
    AgentTurnEvent, AgentTurnOutcome, GenieCodeBinary, Tags,
};
use mog_core::workspaces::WorkspaceGate;
use serde_json::{json, Value};

const WAIT: Duration = Duration::from_secs(10);
const MOG: &str = "0123456789abcdef0123456789abcdef";
const Q: &str = "QUESTION-CANARY what was revenue in Q3?";
const SALES_ANSWER: &str = "ANSWER-CANARY revenue was 4.2M";
const FINANCE_ANSWER: &str = "FINANCE-CANARY cost was 3.1M";

// ---------- the fake app-server ----------

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

struct Server {
    spec: SpawnSpec,
    from_client: Mutex<Receiver<Vec<u8>>>,
    buf: Mutex<Vec<u8>>,
    out: SharedTx,
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

    /// `initialize`, `initialized` and `thread/start`, the way genie 0.5.0 answers.
    fn handshake(&self) {
        let (id, _) = self.recv_req("initialize");
        self.reply(id, json!({ "userAgent": "x" }));
        self.notify(
            "remoteControl/status/changed",
            json!({ "status": "disabled" }),
        );
        assert_eq!(self.recv()["method"], "initialized");
        let (id, _) = self.recv_req("thread/start");
        self.reply(
            id,
            json!({
                "thread": { "id": "th-1", "ephemeral": true, "cliVersion": "0.5.0",
                            "modelProvider": "databricks" },
                "modelProvider": "databricks",
                "approvalPolicy": "never",
                "sandbox": { "type": "readOnly", "networkAccess": false }
            }),
        );
    }

    fn start_turn(&self, server_turn: &str) {
        let (id, _) = self.recv_req("turn/start");
        self.reply(
            id,
            json!({ "turn": { "id": server_turn, "status": "inProgress" } }),
        );
        self.notify(
            "turn/started",
            json!({ "threadId": "th-1", "turn": { "id": server_turn } }),
        );
    }

    fn complete(&self, server_turn: &str, final_text: &str) {
        self.notify(
            "item/completed",
            json!({ "item": { "type": "agentMessage", "text": final_text,
                              "phase": "final_answer" } }),
        );
        self.notify(
            "turn/completed",
            json!({ "threadId": "th-1", "turn": { "id": server_turn,
                                                  "status": "completed", "items": [] } }),
        );
    }

    /// The turn ends as `status` (for an interrupt: `interrupted`).
    fn finish_turn(&self, server_turn: &str, status: &str) {
        self.notify(
            "turn/completed",
            json!({ "threadId": "th-1", "turn": { "id": server_turn,
                                                  "status": status, "items": [] } }),
        );
    }

    fn env(&self, name: &str) -> Option<String> {
        self.spec
            .env
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }
}

/// The fake process reports the parent of this test process as its PID, so the real peer check of
/// the guard (the parent of the shim is the app-server) sees the same relation as in production.
struct FakeProc {
    io: Option<ServerIo>,
    out: SharedTx,
    killed: Arc<AtomicBool>,
}

impl ServerProc for FakeProc {
    fn pid(&self) -> u32 {
        own_parent_pid()
    }
    fn take_io(&mut self) -> Option<ServerIo> {
        self.io.take()
    }
    fn terminate(&mut self) {
        self.killed.store(true, Ordering::SeqCst);
        *self.out.lock().unwrap() = None;
    }
}

#[derive(Default)]
struct FakeSpawner {
    servers: Mutex<VecDeque<Arc<Server>>>,
}

impl Spawner for FakeSpawner {
    fn spawn(&self, spec: &SpawnSpec) -> std::io::Result<Box<dyn ServerProc>> {
        let (c2s_tx, c2s_rx) = mpsc::channel();
        let (out_tx, out_rx) = mpsc::channel();
        let (_err_tx, err_rx) = mpsc::channel::<Vec<u8>>();
        let out: SharedTx = Arc::new(Mutex::new(Some(out_tx)));
        let killed = Arc::new(AtomicBool::new(false));
        self.servers.lock().unwrap().push_back(Arc::new(Server {
            spec: spec.clone(),
            from_client: Mutex::new(c2s_rx),
            buf: Mutex::new(Vec::new()),
            out: Arc::clone(&out),
            killed: Arc::clone(&killed),
        }));
        // The error pipe stays open (its sender leaks into the reader) until the process ends.
        std::mem::forget(_err_tx);
        Ok(Box::new(FakeProc {
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
}

// ---------- the other fakes ----------

/// A fake R8: `genie_ask` is in progress, the first poll is completed with the answer of the
/// profile. It records `(profile, tool)`.
#[derive(Default)]
struct FakeR8 {
    calls: Mutex<Vec<(String, String)>>,
}

fn reply(sc: Value) -> McpReply {
    McpReply {
        structured_content: sc,
        text: String::new(),
        is_error: false,
        http_status: 200,
        rpc_error: None,
    }
}

impl McpTransport for FakeR8 {
    fn call_tool(&self, call: McpCall<'_>) -> CoreResult<McpReply> {
        self.calls
            .lock()
            .unwrap()
            .push((call.profile.to_owned(), call.tool.to_owned()));
        Ok(match call.tool {
            "genie_ask" => reply(json!({
                "status": "in_progress", "conversation_id": format!("conv-{}", call.profile),
                "response_id": "r-1"})),
            "genie_poll_response" => reply(json!({
                "status": "completed",
                "final_answer": if call.profile == "p-sales" { SALES_ANSWER } else { FINANCE_ANSWER },
                "total_row_count": 3, "deep_link": "https://h/x"})),
            _ => reply(json!({"status": "cancellation_requested"})),
        })
    }
    fn list_tools(&self, _: &str, _: &Tags) -> CoreResult<Vec<Value>> {
        Ok(Vec::new())
    }
}

impl FakeR8 {
    fn count(&self, tool: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, t)| t == tool)
            .count()
    }
    fn profiles(&self, tool: &str) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, t)| t == tool)
            .map(|(p, _)| p.clone())
            .collect()
    }
}

#[derive(Debug)]
struct OpenGate;

impl WorkspaceGate for OpenGate {
    fn allows(&self, _: &str) -> bool {
        true
    }
}

/// Moves the simulated clock by the sleep time, so a poll wait costs no real time.
struct SimSleeper(SimClock);

impl Sleeper for SimSleeper {
    fn sleep(&self, d: Duration, _: &AtomicBool) {
        self.0.advance(d);
        std::thread::yield_now();
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<AgentEvent>>);

impl RunEvents for Events {
    fn emit(&self, e: AgentEvent) {
        self.0.lock().unwrap().push(e);
    }
}

impl Events {
    fn all(&self) -> Vec<AgentEvent> {
        self.0.lock().unwrap().clone()
    }

    fn wait(&self, what: &str, f: impl Fn(&AgentEvent) -> bool) -> AgentEvent {
        let end = std::time::Instant::now() + WAIT;
        loop {
            if let Some(e) = self.all().into_iter().find(|e| f(e)) {
                return e;
            }
            assert!(
                std::time::Instant::now() < end,
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn ended(&self, turn: &str) -> (AgentTurnOutcome, u32) {
        match self.wait("turn ended", |e| {
            matches!(e, AgentEvent::Turn(AgentTurnEvent::Ended { turn_id, .. }) if turn_id == turn)
        }) {
            AgentEvent::Turn(AgentTurnEvent::Ended { outcome, calls, .. }) => (outcome, calls),
            _ => unreachable!(),
        }
    }
}

struct NetOn;

impl NetWatch for NetOn {
    fn online(&self) -> bool {
        true
    }
    fn subscribe(&self, _: Box<dyn Fn(bool) + Send + Sync>) {}
}

struct Ws(Snapshot);

impl WorkspaceView for Ws {
    fn snapshot(&self) -> CoreResult<Snapshot> {
        Ok(self.0.clone())
    }
}

struct Approve;

impl Approvals for Approve {
    fn redeem(&self, _: &str, _: Option<&str>) -> bool {
        true
    }
}

struct Env;

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
        Ok(())
    }
    fn profile_state(&self, _: &str) -> AgentProfileState {
        AgentProfileState::Ready
    }
}

struct Bin;

impl BinaryCheck for Bin {
    fn check(&self, _: bool) -> BinInfo {
        BinInfo {
            state: GenieCodeBinary::Ready,
            version: Some("0.5.0".into()),
            path: Some(PathBuf::from("/fake/genie")),
        }
    }
}

struct NoInstall;

impl Installer for NoInstall {
    fn install(&self) -> Result<(), InstallError> {
        Err(InstallError::Failed)
    }
}

/// The shim provider of the app (`src-tauri/src/agent_session_host.rs`, `GuardShim`): the shim
/// command and `guard::shim::env_pairs`.
struct GuardShim {
    socket: PathBuf,
}

impl ShimProvider for GuardShim {
    fn launch(&self, session_id: &str, secret: &RunSecret) -> ShimLaunch {
        ShimLaunch {
            command: PathBuf::from("/Applications/Bottle.app/Contents/MacOS/bottle"),
            args: vec![SHIM_ARG.to_owned()],
            env: shim::env_pairs(&self.socket, session_id, secret)
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
        }
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

// ---------- the rig ----------

struct Rig {
    host: Arc<SessionHost>,
    guard: Arc<AgentGuard>,
    events: Arc<Events>,
    spawner: Arc<FakeSpawner>,
    r8: Arc<FakeR8>,
    audit: Arc<AuditActionLog<MemoryAuditStore>>,
    cfg: SessionConfig,
    dir: PathBuf,
}

impl Rig {
    /// `budget` is the D2 number of Genie calls for one question.
    fn new(budget: u8) -> Self {
        let mut n = [0u8; 6];
        getrandom::fill(&mut n).unwrap();
        // Short: a unix socket path has a small length cap.
        let dir = std::env::temp_dir().join(format!(
            "s2a-{}",
            n.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = SessionConfig {
            home: dir.join("genie-home"),
            tmp_root: dir.join("tmp"),
            pid_file: dir.join("agent.pid"),
            tick: Duration::from_secs(3600),
        };
        std::fs::create_dir_all(&cfg.tmp_root).unwrap();
        let clock = SimClock::new();
        let events = Arc::new(Events::default());
        let r8 = Arc::new(FakeR8::default());
        let audit = Arc::new(AuditActionLog::new(
            AuditLog::new(MemoryAuditStore::default(), AuditRetention::default()),
            Arc::new(SimWallClock::at(1_700_000_000_000)),
        ));
        let guard = AgentGuard::with_options(
            GuardDeps {
                transport: r8.clone(),
                gate: Arc::new(OpenGate),
                events: events.clone(),
                action_log: Arc::new(SessionUuidLog(audit.clone())),
                clock: Arc::new(clock.clone()),
            },
            GuardOptions {
                sleeper: Arc::new(SimSleeper(clock.clone())),
                random: Arc::new(|| 0.5),
                asks: None,
            },
        );
        let socket = guard.listen(&dir.join("agent")).unwrap();
        let snapshot = Snapshot {
            catalog: vec![
                entry("sales", "p-sales", true, true),
                entry("finance", "p-finance", false, false),
            ],
            cross_workspace: false,
            budget,
            agent_profile: Some("p-sales".into()),
            agent_host: Some("sales.cloud.databricks.com".into()),
        };
        let spawner = Arc::new(FakeSpawner::default());
        let guard_dyn: Arc<dyn SessionGuard> = guard.clone();
        let host = SessionHost::new(
            SessionDeps {
                guard: guard_dyn,
                net: Arc::new(NetOn),
                events: events.clone(),
                clock: Arc::new(clock),
                wall: Arc::new(SimWallClock::at(1_700_000_000_000)),
                spawner: spawner.clone(),
                workspaces: Arc::new(Ws(snapshot)),
                approvals: Arc::new(Approve),
                env: Arc::new(Env),
                shim: Arc::new(GuardShim { socket }),
                binary: Arc::new(Bin),
                installer: Arc::new(NoInstall),
            },
            cfg.clone(),
        );
        guard.set_refusal_hook(host.refusal_hook());
        Self {
            host,
            guard,
            events,
            spawner,
            r8,
            audit,
            cfg,
            dir,
        }
    }

    fn submit(&self, q: &str) -> String {
        let a: AgentTurnSubmitArgs =
            serde_json::from_value(json!({ "mogId": MOG, "opId": "op-1", "question": q })).unwrap();
        self.host.submit(&a).expect("the turn is accepted").turn_id
    }

    /// Submit, take the server, finish the handshake and the turn start.
    fn first_turn(&self) -> (Arc<Server>, String) {
        let turn = self.submit(Q);
        let s = self.spawner.next();
        s.handshake();
        s.start_turn("st-1");
        (s, turn)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.host.shutdown();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---------- the shim as Genie Code runs it ----------

#[derive(Clone, Default)]
struct Out(Arc<Mutex<Vec<u8>>>);

impl Write for Out {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The real `shim::run_with` on the real socket, started with the environment that the server
/// got in its spawn spec (the way `config.toml` env_vars forward it). The test is the MCP client.
struct ShimRun {
    to_shim: UnixStream,
    out: Out,
    done: std::thread::JoinHandle<i32>,
    next_id: u64,
}

impl ShimRun {
    fn start(server: &Server) -> Self {
        let sock = server
            .env(shim::ENV_SOCKET)
            .expect("the socket is in the env");
        let sid = server
            .env(shim::ENV_SESSION)
            .expect("the session is in the env");
        let secret = server
            .env(shim::ENV_SECRET)
            .expect("the secret is in the env");
        let stream = UnixStream::connect(sock).unwrap();
        let (to_shim, shim_in) = UnixStream::pair().unwrap();
        let out = Out::default();
        let o = out.clone();
        let done = std::thread::spawn(move || {
            shim::run_with(
                stream,
                &sid,
                &secret,
                BufReader::new(shim_in),
                o,
                ShimOpts {
                    exit_on_socket_eof: false,
                    drain: Duration::from_secs(10),
                },
            )
        });
        let mut s = Self {
            to_shim,
            out,
            done,
            next_id: 0,
        };
        let id = s.send("initialize", json!({"protocolVersion": "2025-06-18"}));
        assert_eq!(s.reply(id)["result"]["protocolVersion"], "2025-06-18");
        s
    }

    fn send(&mut self, method: &str, params: Value) -> u64 {
        self.next_id += 1;
        let line = json!({"jsonrpc": "2.0", "id": self.next_id, "method": method,
                          "params": params});
        writeln!(self.to_shim, "{line}").unwrap();
        self.next_id
    }

    fn reply(&self, id: u64) -> Value {
        let end = std::time::Instant::now() + WAIT;
        loop {
            let text = String::from_utf8(self.out.0.lock().unwrap().clone()).unwrap();
            if let Some(v) = text
                .lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .find(|v| v["id"] == id)
            {
                return v;
            }
            assert!(std::time::Instant::now() < end, "no reply to {id}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// One `tools/call` of `ask_genie`. Gives `(isError, text)`.
    fn ask(&mut self, workspace: &str) -> (bool, String) {
        let id = self.send(
            "tools/call",
            json!({"name": "ask_genie", "arguments": {"workspace": workspace, "question": Q}}),
        );
        let r = self.reply(id)["result"].clone();
        (
            r["isError"].as_bool().unwrap(),
            r["content"][0]["text"].as_str().unwrap().to_owned(),
        )
    }

    fn finish(self) -> i32 {
        drop(self.to_shim);
        self.done.join().unwrap()
    }
}

fn call_id(text: &str) -> String {
    serde_json::from_str::<Value>(text).unwrap()["callId"]
        .as_str()
        .unwrap()
        .to_owned()
}

// ---------- the tests ----------

#[test]
fn the_server_gets_the_shim_env_and_the_config_names_it_without_a_value() {
    let r = Rig::new(10);
    let (s, _) = r.first_turn();
    for name in [shim::ENV_SOCKET, shim::ENV_SESSION, shim::ENV_SECRET] {
        assert!(s.env(name).is_some(), "{name} is in the server env");
    }
    assert_eq!(s.env(shim::ENV_SECRET).unwrap().len(), 64);
    assert!(s.env("AWS_SECRET_ACCESS_KEY").is_none());
    assert!(s.env(shim::ENV_SESSION).unwrap().len() == 32);
    let cfg = std::fs::read_to_string(r.cfg.home.join("config.toml")).unwrap();
    for name in [shim::ENV_SOCKET, shim::ENV_SESSION, shim::ENV_SECRET] {
        assert!(cfg.contains(name), "config.toml names {name}");
    }
    assert!(cfg.contains(SHIM_ARG));
    assert!(!cfg.contains(&s.env(shim::ENV_SECRET).unwrap()));
    assert!(!cfg.contains(&s.env(shim::ENV_SOCKET).unwrap()));
}

#[test]
fn one_turn_asks_the_agent_workspace_in_text_and_another_workspace_as_a_receipt() {
    let r = Rig::new(10);
    let (s, turn) = r.first_turn();
    let mut shim = ShimRun::start(&s);

    // Another workspace first (D1 off): a receipt, never the answer text.
    let (err, finance) = shim.ask("finance");
    assert!(!err, "{finance}");
    let v: Value = serde_json::from_str(&finance).unwrap();
    assert_eq!(v["status"], "answered");
    assert_eq!(v["answerChars"], FINANCE_ANSWER.chars().count());
    assert!(v.get("answer").is_none());
    assert!(!finance.contains("FINANCE-CANARY"));
    let finance_id = call_id(&finance);

    // The agent workspace: the text block.
    let (err, sales) = shim.ask("sales");
    assert!(!err, "{sales}");
    let v: Value = serde_json::from_str(&sales).unwrap();
    assert_eq!(v["status"], "answered");
    assert!(v["answer"].as_str().unwrap().contains("ANSWER-CANARY"));
    let sales_id = call_id(&sales);

    // The text taints the session: the other workspace is refused now, and nothing is sent.
    let (err, refused) = shim.ask("finance");
    assert!(err);
    assert_eq!(refused, "cross_workspace_off");
    assert_eq!(r.r8.profiles("genie_ask"), vec!["p-finance", "p-sales"]);

    s.complete(
        "st-1",
        &json!({ "answer": "Revenue was 4.2M", "sources": [
            { "workspace": "sales", "callId": sales_id } ] })
        .to_string(),
    );
    assert_eq!(r.events.ended(&turn), (AgentTurnOutcome::Answered, 2));
    let res = r.host.turn_result(&turn).unwrap();
    assert_ne!(res.answer_kind, AgentAnswerKind::Hidden);
    assert_eq!(res.sources.len(), 1);

    // The user view of the receipt call has its answer; the model got none.
    let d = r.host.turn_call(&turn, &finance_id).unwrap();
    assert_eq!(d.state, AgentCallState::Answered);
    assert_eq!(shim.finish(), 0);
    // One content-free audit entry for each sent ask, on the real audit log (UUID mog id).
    let sid = s.env(shim::ENV_SESSION).unwrap();
    let uuid = format!(
        "{}-{}-{}-{}-{}",
        &sid[..8],
        &sid[8..12],
        &sid[12..16],
        &sid[16..20],
        &sid[20..]
    );
    assert_eq!(r.audit.entries(&uuid).unwrap().len(), 2);
}

#[test]
fn the_budget_refuses_the_call_after_the_last_one_and_sends_nothing() {
    // The smallest D2 value is 5.
    let r = Rig::new(5);
    let (s, turn) = r.first_turn();
    let mut shim = ShimRun::start(&s);
    for _ in 0..5 {
        assert!(!shim.ask("sales").0);
    }
    let (err, refused) = shim.ask("sales");
    assert!(err);
    assert_eq!(refused, "budget_reached");
    assert_eq!(r.r8.count("genie_ask"), 5, "the refused call sent nothing");
    r.events.wait("a refusal event", |e| {
        matches!(e, AgentEvent::Turn(AgentTurnEvent::Refused { .. }))
    });
    assert_eq!(r.guard.refusals(&turn), 1);
    s.complete("st-1", "just words");
    assert_eq!(r.events.ended(&turn), (AgentTurnOutcome::Answered, 5));
    assert_eq!(shim.finish(), 0);
}

#[test]
fn the_third_refusal_stops_the_turn_through_the_host_hook() {
    let r = Rig::new(10);
    let (s, turn) = r.first_turn();
    let mut shim = ShimRun::start(&s);
    for _ in 0..3 {
        let (err, code) = shim.ask("nowhere");
        assert!(err);
        assert_eq!(code, "workspace_not_available");
    }
    // The host hook interrupts the turn; the server answers the interrupt and ends the turn.
    let (id, _) = s.recv_req("turn/interrupt");
    s.reply(id, json!({}));
    s.finish_turn("st-1", "interrupted");
    assert_eq!(r.events.ended(&turn), (AgentTurnOutcome::Refusals, 0));
    assert_eq!(r.r8.count("genie_ask"), 0);
    assert_eq!(shim.finish(), 0);
}

#[test]
fn a_shim_with_a_wrong_secret_is_refused_and_a_second_shim_too() {
    let r = Rig::new(10);
    let (s, _) = r.first_turn();
    let sock = s.env(shim::ENV_SOCKET).unwrap();
    let sid = s.env(shim::ENV_SESSION).unwrap();
    let bad = shim::run_with(
        UnixStream::connect(&sock).unwrap(),
        &sid,
        &"00".repeat(32),
        BufReader::new(std::io::empty()),
        Out::default(),
        ShimOpts {
            exit_on_socket_eof: false,
            drain: Duration::ZERO,
        },
    );
    assert_eq!(bad, 1, "hello refused");
    // The real one still connects, once.
    let shim = ShimRun::start(&s);
    let second = shim::run_with(
        UnixStream::connect(&sock).unwrap(),
        &sid,
        &s.env(shim::ENV_SECRET).unwrap(),
        BufReader::new(std::io::empty()),
        Out::default(),
        ShimOpts {
            exit_on_socket_eof: false,
            drain: Duration::ZERO,
        },
    );
    assert_eq!(second, 1, "one shim for each server process");
    assert_eq!(shim.finish(), 0);
}

#[test]
fn the_session_end_deletes_genie_home_kills_the_server_and_drops_the_ledger() {
    let r = Rig::new(10);
    let (s, turn) = r.first_turn();
    let mut shim = ShimRun::start(&s);
    let (err, text) = shim.ask("sales");
    assert!(!err);
    let sid = s.env(shim::ENV_SESSION).unwrap();
    s.complete(
        "st-1",
        &json!({ "answer": "x", "sources": [
            { "workspace": "sales", "callId": call_id(&text) } ] })
        .to_string(),
    );
    r.events.ended(&turn);
    assert!(r.cfg.home.join("config.toml").exists());
    assert!(!r.guard.calls(&sid).is_empty());
    assert_eq!(shim.finish(), 0);

    r.host.end_blocking(AgentSessionEndReason::Wipe);

    assert!(!r.cfg.home.exists(), "GENIE_HOME is deleted");
    assert!(s.killed.load(Ordering::SeqCst), "the server is killed");
    assert!(
        r.guard.calls(&sid).is_empty(),
        "Wipe drops the ledger at once"
    );
    r.events.wait("session ended", |e| {
        matches!(
            e,
            AgentEvent::Session(AgentSessionEvent::Ended {
                reason: AgentSessionEndReason::Wipe,
                ..
            })
        )
    });
    // The secret of the ended session opens no new channel.
    let sock = s.env(shim::ENV_SOCKET).unwrap();
    let late = shim::run_with(
        UnixStream::connect(&sock).unwrap(),
        &sid,
        &s.env(shim::ENV_SECRET).unwrap(),
        BufReader::new(std::io::empty()),
        Out::default(),
        ShimOpts {
            exit_on_socket_eof: false,
            drain: Duration::ZERO,
        },
    );
    assert_eq!(late, 1);
}
