//! R17 `bottle-mcp` (#113): the loopback MCP server of one agent session (RTR-25, RTR-26,
//! RTR-29e, ARC-6, ARC-7, 4.6 rule 9).
//!
//! Every test uses doubles: a local-tools double, a relay double or the [`BrainRelay`] with a
//! recording event sink. No test reads a CLI profile, calls a model or leaves loopback.
//! Waits end on events (channels); `BOUND` only keeps a broken run from hanging.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::bottle_mcp::{
    BottleMcpDeps, BottleMcpServer, BrainRelay, LocalTools, StoreLocalTools, ToolCall, ToolRelay,
    MCP_PATH, TOOLS,
};
use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc::events;
use mog_core::local_store::LocalStore;
use mog_core::loopback::MAX_CONNECTIONS;
use serde_json::{json, Value};

/// A loaded machine can be slow; no test asserts an upper time bound.
const BOUND: Duration = Duration::from_secs(120);
const MOG: &str = "0b9a3c1e-4f2d-4a8b-9c7d-1e2f3a4b5c6d";
/// A plainly fake value in a rule-7 shape (`password=`), so the scrubber must remove it.
const FAKE_SECRET: &str = "password=test-secret-not-real";

// ---------- doubles ----------

#[derive(Default)]
struct FakeLocal {
    calls: Mutex<Vec<(String, String, Value)>>,
    leak: bool,
}

impl FakeLocal {
    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
    fn answer(&self, tool: &str, mog_id: &str, args: Value) -> CoreResult<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((tool.to_owned(), mog_id.to_owned(), args));
        let text = if self.leak {
            FAKE_SECRET
        } else {
            "revenue grew"
        };
        Ok(json!({ "rows": [{ "text": text }] }))
    }
}

impl LocalTools for FakeLocal {
    fn transcript_search(&self, mog_id: &str, query: &str, limit: u32) -> CoreResult<Value> {
        self.answer(
            "transcript.search",
            mog_id,
            json!({ "query": query, "limit": limit }),
        )
    }
    fn memory_recall(&self, mog_id: &str, query: &str, limit: u32) -> CoreResult<Value> {
        self.answer(
            "memory.recall",
            mog_id,
            json!({ "query": query, "limit": limit }),
        )
    }
    fn memory_write(&self, mog_id: &str, item_type: &str, text: &str) -> CoreResult<Value> {
        self.answer(
            "memory.write",
            mog_id,
            json!({ "type": item_type, "text": text }),
        )
    }
}

/// A relay that holds every call until the test answers it.
struct HeldRelay {
    calls: Mutex<mpsc::Sender<(ToolCall, mpsc::Sender<Value>)>>,
    count: AtomicUsize,
}

impl HeldRelay {
    fn new() -> (Arc<Self>, mpsc::Receiver<(ToolCall, mpsc::Sender<Value>)>) {
        let (tx, rx) = mpsc::channel();
        (
            Arc::new(Self {
                calls: Mutex::new(tx),
                count: AtomicUsize::new(0),
            }),
            rx,
        )
    }
}

impl ToolRelay for HeldRelay {
    fn send(&self, call: &ToolCall) -> CoreResult<mpsc::Receiver<Value>> {
        self.count.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        let _ = self.calls.lock().unwrap().send((call.clone(), tx));
        Ok(rx)
    }
}

struct Rig {
    server: BottleMcpServer,
    local: Arc<FakeLocal>,
    relay: Arc<HeldRelay>,
    relayed: mpsc::Receiver<(ToolCall, mpsc::Sender<Value>)>,
}

fn rig_with(local: FakeLocal) -> Rig {
    let local = Arc::new(local);
    let (relay, relayed) = HeldRelay::new();
    let deps = BottleMcpDeps::new(local.clone(), relay.clone());
    let server = BottleMcpServer::start(MOG, &deps).expect("bottle-mcp binds loopback");
    Rig {
        server,
        local,
        relay,
        relayed,
    }
}

fn rig() -> Rig {
    rig_with(FakeLocal::default())
}

// ---------- a minimal HTTP client ----------

fn addr_of(url: &str) -> SocketAddr {
    let rest = url.strip_prefix("http://").expect("http url");
    rest.split('/').next().unwrap().parse().expect("ip:port")
}

/// Send one raw request; return the status and the JSON body (`Null` if none).
fn raw(addr: SocketAddr, request: &[u8]) -> (u16, Value) {
    let mut s = TcpStream::connect(addr).expect("connects");
    s.set_read_timeout(Some(BOUND)).unwrap();
    let _ = s.write_all(request);
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or("")
        .to_owned();
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

fn post_as(server: &BottleMcpServer, auth: Option<&str>, body: &[u8]) -> (u16, Value) {
    let addr = addr_of(&server.url());
    let mut req = format!(
        "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        addr.port(),
        body.len()
    );
    if let Some(a) = auth {
        req.push_str(&format!("Authorization: {a}\r\n"));
    }
    req.push_str("\r\n");
    let mut bytes = req.into_bytes();
    bytes.extend_from_slice(body);
    raw(addr, &bytes)
}

fn rpc(server: &BottleMcpServer, method: &str, params: Value) -> Value {
    let body = json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params });
    let (status, v) = post_as(
        server,
        Some(server.authorization()),
        body.to_string().as_bytes(),
    );
    assert_eq!(status, 200, "{method}: {v}");
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], 7);
    v
}

fn call(server: &BottleMcpServer, tool: &str, args: Value) -> Value {
    rpc(
        server,
        "tools/call",
        json!({ "name": tool, "arguments": args }),
    )
}

fn tools_list(server: &BottleMcpServer) -> Vec<Value> {
    rpc(server, "tools/list", json!({}))["result"]["tools"]
        .as_array()
        .expect("tools array")
        .clone()
}

// ---------- RTR-26 / AT-42: exactly the 7 tools ----------

#[test]
fn rtr_26_at_42_tools_list_returns_exactly_the_7_tools() {
    let r = rig();
    let tools = tools_list(&r.server);
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "transcript.search",
            "memory.recall",
            "memory.write",
            "show_blurb",
            "set_mood",
            "propose_page",
            "ask_user",
        ]
    );
    assert_eq!(names, TOOLS);
    for t in &tools {
        assert!(!t["description"].as_str().unwrap_or("").is_empty(), "{t}");
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
    }
}

#[test]
fn rtr_25_initialize_names_bottle_mcp_and_the_tools_capability() {
    let r = rig();
    let v = rpc(
        &r.server,
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } }),
    );
    assert_eq!(v["result"]["serverInfo"]["name"], "bottle-mcp");
    assert!(v["result"]["capabilities"]["tools"].is_object(), "{v}");
    assert!(v["result"]["protocolVersion"].is_string(), "{v}");
    let body = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    let (status, _) = post_as(
        &r.server,
        Some(r.server.authorization()),
        body.to_string().as_bytes(),
    );
    assert_eq!(status, 202);
    let v = rpc(&r.server, "no/such/method", json!({}));
    assert_eq!(v["error"]["code"], -32601);
}

// ---------- RTR-29e / AT-42: no forbidden action ----------

#[test]
fn rtr_29e_at_42_tools_list_contains_no_capture_approve_publish_or_switch_action() {
    let r = rig();
    for t in tools_list(&r.server) {
        let text = format!("{} {}", t["name"], t["description"]).to_ascii_lowercase();
        for word in [
            "capture",
            "approve",
            "publish",
            "switch",
            "record",
            "workspace",
        ] {
            assert!(!text.contains(word), "tool text names `{word}`: {t}");
        }
    }
}

#[test]
fn rtr_29e_forbidden_and_unknown_tools_are_refused_and_nothing_runs() {
    let r = rig();
    for name in [
        "start_capture",
        "approve_context",
        "publish_page",
        "switch_workspace",
        "genie_ask",
        "memory.forget",
        "",
    ] {
        let v = call(&r.server, name, json!({ "text": "do it" }));
        assert_eq!(v["error"]["code"], -32602, "{name}: {v}");
        assert!(v.get("result").is_none(), "{name}: {v}");
    }
    let v = rpc(&r.server, "tools/call", json!({ "arguments": {} }));
    assert_eq!(v["error"]["code"], -32602, "{v}");
    assert_eq!(r.local.count(), 0, "no local tool ran");
    assert_eq!(
        r.relay.count.load(Ordering::SeqCst),
        0,
        "nothing was relayed"
    );
}

#[test]
fn rtr_29e_bad_arguments_are_refused_and_nothing_runs() {
    let r = rig();
    for (tool, args) in [
        ("memory.write", json!({})),
        (
            "memory.write",
            json!({ "type": "genie_answer", "text": "x" }),
        ),
        ("memory.recall", json!({ "query": 3 })),
        ("transcript.search", json!({ "query": "" })),
        // `listening` says the mic is open; the agent cannot open it.
        ("set_mood", json!({ "mood": "listening" })),
        ("show_blurb", json!({})),
        ("ask_user", json!({ "question": "" })),
        ("propose_page", json!({ "title": "Revenue" })),
    ] {
        let v = call(&r.server, tool, args.clone());
        assert_eq!(v["error"]["code"], -32602, "{tool} {args}: {v}");
    }
    assert_eq!(r.local.count(), 0);
    assert_eq!(r.relay.count.load(Ordering::SeqCst), 0);
}

// ---------- RTR-25 / ARC-7 / rule 9: loopback, secret, caps ----------

#[test]
fn rtr_25_arc_7_binds_127_0_0_1_only() {
    let r = rig();
    let url = r.server.url();
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    assert!(url.ends_with(MCP_PATH), "{url}");
    let addr = addr_of(&url);
    assert!(addr.ip().is_loopback());
    assert_ne!(addr.port(), 0);
}

#[test]
fn rtr_25_arc_7_missing_or_wrong_secret_is_401_and_runs_no_tool() {
    let r = rig();
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "memory.recall", "arguments": { "query": "revenue" } } })
    .to_string();
    for auth in [
        None,
        Some("Bearer test-secret-not-real"),
        Some(""),
        Some(&r.server.authorization()[7..]),
    ] {
        let (status, v) = post_as(&r.server, auth, body.as_bytes());
        assert_eq!(status, 401, "{auth:?}: {v}");
    }
    assert_eq!(r.local.count(), 0, "no tool ran");
    // A Host that is not this loopback listener is refused too (DNS rebinding).
    let addr = addr_of(&r.server.url());
    let req = format!(
        "POST {MCP_PATH} HTTP/1.1\r\nHost: evil.example\r\nAuthorization: {}\r\nContent-Length: {}\r\n\r\n{body}",
        r.server.authorization(),
        body.len()
    );
    assert_eq!(raw(addr, req.as_bytes()).0, 403);
    assert_eq!(r.local.count(), 0, "no tool ran");
}

#[test]
fn arc_7_oversize_body_is_413_and_runs_no_tool() {
    let r = rig();
    let addr = addr_of(&r.server.url());
    let req = format!(
        "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: {}\r\nContent-Length: {}\r\n\r\n",
        addr.port(),
        r.server.authorization(),
        64 * 1024 * 1024
    );
    assert_eq!(raw(addr, req.as_bytes()).0, 413);
    assert_eq!(r.local.count(), 0);
}

#[test]
fn rtr_25_only_the_mcp_path_and_post_are_served() {
    let r = rig();
    let addr = addr_of(&r.server.url());
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string();
    for (method, path) in [("POST", "/other"), ("GET", MCP_PATH)] {
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: {}\r\nContent-Length: {}\r\n\r\n{body}",
            addr.port(),
            r.server.authorization(),
            body.len()
        );
        let (status, _) = raw(addr, req.as_bytes());
        assert!(status == 404 || status == 405, "{method} {path}: {status}");
    }
}

#[test]
fn arc_7_connection_cap_refuses_more_than_max_connections() {
    let r = rig();
    // Fill every slot with an `ask_user` call that the relay double holds.
    let mut threads = Vec::new();
    for i in 0..MAX_CONNECTIONS {
        let url = r.server.url();
        let auth = r.server.authorization().to_owned();
        threads.push(std::thread::spawn(move || {
            let addr = addr_of(&url);
            let body = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
                "params": { "name": "ask_user", "arguments": { "question": format!("q{i}") } } })
            .to_string();
            let req = format!(
                "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: {auth}\r\nContent-Length: {}\r\n\r\n{body}",
                addr.port(),
                body.len()
            );
            raw(addr, req.as_bytes())
        }));
    }
    let mut held = Vec::new();
    for _ in 0..MAX_CONNECTIONS {
        held.push(r.relayed.recv_timeout(BOUND).expect("call relayed"));
    }
    assert_eq!(r.server.open_connections(), MAX_CONNECTIONS);
    // One more connection is closed with no reply, and its tool does not run.
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "memory.recall", "arguments": { "query": "revenue" } } })
    .to_string();
    let (status, _) = post_as(&r.server, Some(r.server.authorization()), body.as_bytes());
    assert_eq!(status, 0, "over the cap: no reply");
    assert_eq!(r.local.count(), 0);
    // Answer the held calls: every one of them completes.
    for (_, tx) in held {
        tx.send(json!({ "answer": "yes" })).unwrap();
    }
    for t in threads {
        let (status, v) = t.join().unwrap();
        assert_eq!(status, 200, "{v}");
        assert_eq!(v["result"]["structuredContent"]["answer"], "yes", "{v}");
    }
}

#[test]
fn rtr_25_stop_closes_the_listener() {
    let r = rig();
    let addr = addr_of(&r.server.url());
    let auth = r.server.authorization().to_owned();
    r.server.stop();
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string();
    let req = format!(
        "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: {auth}\r\nContent-Length: {}\r\n\r\n{body}",
        addr.port(),
        body.len()
    );
    let answered = TcpStream::connect(addr)
        .ok()
        .map(|mut s| {
            s.set_read_timeout(Some(BOUND)).unwrap();
            let _ = s.write_all(req.as_bytes());
            let mut out = Vec::new();
            let _ = s.read_to_end(&mut out);
            !out.is_empty()
        })
        .unwrap_or(false);
    assert!(!answered, "a stopped server answers nothing");
}

// ---------- RTR-25: the local tools through the R10 seam ----------

#[test]
fn rtr_25_local_tools_run_for_the_session_mog_only() {
    let r = rig();
    let v = call(
        &r.server,
        "memory.recall",
        json!({ "query": "revenue", "limit": 5, "mogId": "someone-else" }),
    );
    assert_eq!(v["result"]["isError"], false, "{v}");
    assert_eq!(
        v["result"]["structuredContent"]["rows"][0]["text"],
        "revenue grew"
    );
    assert_eq!(v["result"]["content"][0]["type"], "text");
    call(&r.server, "transcript.search", json!({ "query": "churn" }));
    call(
        &r.server,
        "memory.write",
        json!({ "type": "fact", "text": "Q3 revenue is net of refunds" }),
    );
    let calls = r.local.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec![
            (
                "memory.recall".to_owned(),
                MOG.to_owned(),
                json!({ "query": "revenue", "limit": 5 })
            ),
            (
                "transcript.search".to_owned(),
                MOG.to_owned(),
                json!({ "query": "churn", "limit": 10 })
            ),
            (
                "memory.write".to_owned(),
                MOG.to_owned(),
                json!({ "type": "fact", "text": "Q3 revenue is net of refunds" })
            ),
        ]
    );
}

#[derive(Default)]
struct RecordingStore(Mutex<Vec<(String, Value)>>);

impl LocalStore for RecordingStore {
    fn open(&self) -> CoreResult<()> {
        Ok(())
    }
    fn exec(&self, stmt_id: &str, params: Value) -> CoreResult<Value> {
        self.0.lock().unwrap().push((stmt_id.to_owned(), params));
        Ok(json!({ "rows": [] }))
    }
}

#[test]
fn rtr_25_store_local_tools_use_the_fixed_r10_catalog() {
    let store = Arc::new(RecordingStore::default());
    let tools = StoreLocalTools::new(store.clone(), "device-test-1");
    tools.memory_recall(MOG, "revenue", 5).unwrap();
    tools
        .memory_write(MOG, "fact", "Q3 revenue is net")
        .unwrap();
    let ran = store.0.lock().unwrap().clone();
    assert_eq!(ran.len(), 2);
    assert_eq!(ran[0].0, "mem.recall.fts");
    assert_eq!(
        ran[0].1,
        json!({ "mogId": MOG, "query": "revenue", "limit": 5 })
    );
    assert_eq!(ran[1].0, "mem.item.upsert");
    let p = &ran[1].1;
    assert_eq!(p["mogId"], MOG);
    assert_eq!(p["itemType"], "fact");
    assert_eq!(p["text"], "Q3 revenue is net");
    assert_eq!(p["source"], "agent");
    assert_eq!(p["deviceId"], "device-test-1");
    assert_eq!(p["itemId"].as_str().unwrap().len(), 36);
    // Each statement the tools use must be a brain-access entry of the fixed catalog.
    for (id, _) in &ran {
        assert!(mog_core::local_store::catalog::find(id).is_some(), "{id}");
    }
    // The local catalog has no transcript statement yet (TODO(#61)).
    assert!(matches!(
        tools.transcript_search(MOG, "churn", 5),
        Err(CoreError::NotImplemented { .. })
    ));
}

// ---------- ARC-6 / rule 7: no secret reaches the harness ----------

#[test]
fn arc_6_tool_results_are_scrubbed_before_they_reach_the_harness() {
    let r = rig_with(FakeLocal {
        leak: true,
        ..FakeLocal::default()
    });
    let v = call(&r.server, "memory.recall", json!({ "query": "revenue" }));
    assert!(!v.to_string().contains("test-secret-not-real"), "{v}");
    // A relayed result is scrubbed too.
    let url = r.server.url();
    let auth = r.server.authorization().to_owned();
    let t = std::thread::spawn(move || {
        let addr = addr_of(&url);
        let body = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": "ask_user", "arguments": { "question": "which quarter?" } } })
        .to_string();
        let req = format!(
            "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: {auth}\r\nContent-Length: {}\r\n\r\n{body}",
            addr.port(),
            body.len()
        );
        raw(addr, req.as_bytes())
    });
    let (_, tx) = r.relayed.recv_timeout(BOUND).expect("relayed");
    tx.send(json!({ "answer": FAKE_SECRET })).unwrap();
    let (status, v) = t.join().unwrap();
    assert_eq!(status, 200);
    assert!(!v.to_string().contains("test-secret-not-real"), "{v}");
}

// ---------- RTR-25: UI tools relay to the brain ----------

#[test]
fn rtr_25_ui_tools_round_trip_through_agent_tool_call_and_agent_tool_result() {
    let (etx, erx) = mpsc::channel::<(String, Value)>();
    let etx = Mutex::new(etx);
    let relay = Arc::new(BrainRelay::new(Arc::new(
        move |name: &str, payload: Value| {
            let _ = etx.lock().unwrap().send((name.to_owned(), payload));
        },
    )));
    let deps = BottleMcpDeps::new(Arc::new(FakeLocal::default()), relay.clone());
    let server = BottleMcpServer::start(MOG, &deps).unwrap();
    for (tool, args) in [
        ("show_blurb", json!({ "text": "Revenue grew 4 %" })),
        ("set_mood", json!({ "mood": "answering" })),
        (
            "propose_page",
            json!({ "title": "Net revenue", "body": "Revenue after refunds." }),
        ),
        ("ask_user", json!({ "question": "Which quarter?" })),
    ] {
        let url = server.url();
        let auth = server.authorization().to_owned();
        let a = args.clone();
        let t = std::thread::spawn(move || {
            let addr = addr_of(&url);
            let body = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
                "params": { "name": tool, "arguments": a } })
            .to_string();
            let req = format!(
                "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: {auth}\r\nContent-Length: {}\r\n\r\n{body}",
                addr.port(),
                body.len()
            );
            raw(addr, req.as_bytes())
        });
        let (name, payload) = erx.recv_timeout(BOUND).expect("agent.tool_call emitted");
        assert_eq!(name, events::AGENT_TOOL_CALL);
        assert_eq!(payload["mogId"], MOG);
        assert_eq!(payload["tool"], tool);
        assert_eq!(payload["args"], args);
        let call_id = payload["callId"].as_str().expect("callId").to_owned();
        assert!(!call_id.is_empty());
        // A result for another MOG or another call is ignored.
        assert!(!relay.deliver("another-mog", &call_id, json!({ "ok": false })));
        assert!(!relay.deliver(MOG, "tc-unknown", json!({ "ok": false })));
        assert!(relay.deliver(MOG, &call_id, json!({ "ok": true, "tool": tool })));
        let (status, v) = t.join().unwrap();
        assert_eq!(status, 200, "{v}");
        assert_eq!(v["result"]["isError"], false, "{v}");
        assert_eq!(v["result"]["structuredContent"]["tool"], tool, "{v}");
        // A second result for the same call is ignored.
        assert!(!relay.deliver(MOG, &call_id, json!({ "ok": true })));
    }
}

#[test]
fn rtr_25_a_relay_without_an_answer_times_out_as_a_tool_error() {
    let (etx, erx) = mpsc::channel::<(String, Value)>();
    let etx = Mutex::new(etx);
    let relay = Arc::new(BrainRelay::new(Arc::new(
        move |name: &str, payload: Value| {
            let _ = etx.lock().unwrap().send((name.to_owned(), payload));
        },
    )));
    let deps = BottleMcpDeps::new(Arc::new(FakeLocal::default()), relay.clone())
        .with_relay_timeout(Duration::from_millis(300));
    let server = BottleMcpServer::start(MOG, &deps).unwrap();
    let v = call(&server, "show_blurb", json!({ "text": "hello" }));
    assert_eq!(v["result"]["isError"], true, "{v}");
    let (_, payload) = erx.recv_timeout(BOUND).expect("emitted");
    let call_id = payload["callId"].as_str().unwrap();
    assert!(
        !relay.deliver(MOG, call_id, json!({})),
        "a late result is dropped"
    );
}
