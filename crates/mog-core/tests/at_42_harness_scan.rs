//! AT-42 (RTR-27, RTR-28, ARC-7): an environment and memory scan of a REAL harness process
//! finds no token, after the harness asked Genie One through the loopback proxy (#139).
//!
//! The parent test starts R16 [`SharedAgentHost`] with the real [`StdRuntimeLauncher`]. The
//! "harness runtime" is this test binary again, in child mode
//! ([`at_42_harness_child_process_entry`]). The child reads its `open_session` frame on stdin,
//! asks Genie One and polls through the `genieOneMcp` endpoint of the frame, and then scans:
//! - its environment and its arguments;
//! - every byte it got from outside (stdin and every loopback reply);
//! - every writable page of its own memory (Linux `/proc/self/mem`, Windows `VirtualQuery` and
//!   `ReadProcessMemory`, macOS `mach_vm_region` and `mach_vm_read_overwrite`).
//!
//! The fake R6 broker lends random fake tokens (`test-token-not-real-<n>`). The child never gets
//! a token to look for: it gets only the SHA-256 of each one. It also gets a canary of the same
//! shape in plain text, which it keeps in memory: the scan must find the canary, so a scan that
//! reads nothing cannot pass. No live workspace.

#[path = "support/fake_mcp.rs"]
mod fake_mcp;

use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fake_mcp::*;
use mog_core::agent_host::model_proxy::{ModelProxy, ModelTarget, UnavailableUpstream};
use mog_core::agent_host::runtime::{RuntimeConfig, StdRuntimeLauncher};
use mog_core::agent_host::{
    AgentError, AgentHost, AgentHostDeps, AgentStatus, AgentStatusSink, Harness, ModelTargets,
    RuntimeConfigs, SharedAgentHost,
};
use mog_core::bottle_mcp::{BottleMcpDeps, UnavailableLocalTools, UnavailableRelay};
use mog_core::error::CoreResult;
use mog_core::mcp_transport::proxy::{GenieProxyDeps, GenieTarget, GenieTargets};
use mog_core::mcp_transport::{McpLimits, McpSeams, McpTransport, ReqwestMcpTransport};
use mog_core::request_identity::BottleIdentity;
use mog_core::token_broker::{TokenBroker, TokenLease};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// The name of the child entry test, and the extra argument that turns child mode on.
const CHILD_TEST: &str = "at_42_harness_child_process_entry";
const CHILD_MODE: &str = "bottle-at-42-child-mode";
const TARGETS_FILE: &str = "scan-targets.json";
const REPORT_FILE: &str = "scan-report.json";
/// A busy machine can take long to start a process. Every wait ends on an event first.
const BOUND: Duration = Duration::from_secs(180);

fn sha256_hex(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).unwrap();
    u64::from_le_bytes(b)
}

// ---------- the parent ----------

/// Lends fake tokens from a fixed pool, in order.
struct PoolBroker {
    pool: Vec<String>,
    next: Mutex<usize>,
}

impl TokenBroker for PoolBroker {
    fn with_token(
        &self,
        _profile: &str,
        _host: &str,
        f: &mut dyn FnMut(&[u8]),
    ) -> CoreResult<TokenLease> {
        let mut n = self.next.lock().unwrap();
        let t = &self.pool[*n % self.pool.len()];
        *n += 1;
        f(t.as_bytes());
        Ok(TokenLease::new(*n as u64))
    }
    fn invalidate(&self, _profile: &str, _lease: TokenLease) -> CoreResult<()> {
        Ok(())
    }
    fn workspace_id(&self, _profile: &str) -> CoreResult<String> {
        Ok("1234".into())
    }
}

struct Target;

impl GenieTargets for Target {
    fn target(&self, _mog_id: &str) -> CoreResult<GenieTarget> {
        Ok(GenieTarget {
            profile: PROFILE.to_owned(),
            warehouse_id: None,
        })
    }
}

struct Models;

impl ModelTargets for Models {
    fn target(&self, _mog_id: &str) -> Result<ModelTarget, AgentError> {
        Ok(ModelTarget::Gateway {
            profile: PROFILE.into(),
            host: "example.cloud.databricks.com".into(),
        })
    }
}

struct NoStatus;

impl AgentStatusSink for NoStatus {
    fn status(&self, _s: &AgentStatus) {}
}

fn wait_for_report(path: &PathBuf) -> Value {
    let start = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                return v;
            }
        }
        assert!(
            start.elapsed() < BOUND,
            "the harness child wrote no scan report"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn rtr_28_rtr_27_at_42_harness_env_and_memory_hold_no_token() {
    let home = std::env::temp_dir().join(format!("bottle-at-42-{:016x}", random_u64()));
    std::fs::create_dir_all(&home).unwrap();
    let pool: Vec<String> = (0..64)
        .map(|_| format!("{TOKEN_PREFIX}{}", random_u64()))
        .collect();
    let canary = format!("{TOKEN_PREFIX}{}", random_u64());
    let hashes: Vec<String> = pool.iter().map(|t| sha256_hex(t.as_bytes())).collect();
    std::fs::write(
        home.join(TARGETS_FILE),
        json!({"hashes": hashes, "canary": canary}).to_string(),
    )
    .unwrap();

    let s = genie_one(|_| None);
    let broker = Arc::new(PoolBroker {
        pool: pool.clone(),
        next: Mutex::new(0),
    });
    let r8: Arc<dyn McpTransport> = Arc::new(
        ReqwestMcpTransport::with_seams(McpSeams {
            hosts: Hosts::one(&s.origin()),
            endpoints: Endpoints::one(&s.url()),
            tokens: broker.clone(),
            policy: Arc::new(LoopbackPolicy),
            decorator: Arc::new(BottleIdentity::new()),
            log: Arc::new(Lines::default()),
            limits: McpLimits::default(),
        })
        .unwrap(),
    );
    let exe = std::env::current_exe().unwrap();
    let runtime = RuntimeConfig::new(
        exe,
        vec![
            CHILD_TEST.into(),
            CHILD_MODE.into(),
            "--exact".into(),
            "--nocapture".into(),
            "--test-threads=1".into(),
        ],
    )
    .with_home(&home);
    let host = SharedAgentHost::new(AgentHostDeps {
        launcher: Arc::new(StdRuntimeLauncher::default()),
        proxy: Arc::new(ModelProxy::start(broker.clone(), Arc::new(UnavailableUpstream)).unwrap()),
        targets: Arc::new(Models),
        status: Arc::new(NoStatus),
        runtimes: RuntimeConfigs {
            claude: runtime.clone(),
            codex: runtime,
        },
        bottle_mcp: BottleMcpDeps::new(Arc::new(UnavailableLocalTools), Arc::new(UnavailableRelay))
            .with_genie_proxy(GenieProxyDeps::new(r8, Arc::new(Target))),
    });
    host.start_session(MOG, Harness::Claude).unwrap();
    let report = wait_for_report(&home.join(REPORT_FILE));
    host.stop_session(MOG).unwrap();
    let _ = std::fs::remove_dir_all(&home);
    // Failure messages show the counts only, never the child's environment or bytes.
    let mut brief = report.clone();
    if let Some(o) = brief.as_object_mut() {
        o.remove("env");
        o.remove("inflow");
    }

    // The child asked Genie One through the proxy, and the proxy added a pool token to each
    // upstream request: so tokens were live in the Bottle process during the scan.
    assert_eq!(report["askOk"], json!(true), "{brief}");
    assert_eq!(report["pollOk"], json!(true), "{brief}");
    let calls: Vec<Seen> = s
        .seen()
        .into_iter()
        .filter(|x| x.rpc_method() == "tools/call")
        .collect();
    assert_eq!(calls.len(), 2, "one ask and one poll");
    for x in s.seen() {
        let b = x.bearer().expect("the proxy added a token");
        assert!(pool.contains(&b), "a pool token");
    }

    // No token in the env, the arguments, or any byte that reached the harness.
    assert_eq!(report["inflowTokenHits"], json!(0), "{brief}");
    assert_eq!(report["inflowPrefixHits"], json!(0), "{brief}");
    let env = report["env"].as_array().unwrap();
    assert!(!env.is_empty());
    for kv in env {
        let kv = kv.as_str().unwrap();
        let name = kv.split('=').next().unwrap().to_ascii_uppercase();
        assert!(!name.starts_with("DATABRICKS_"), "{kv}");
        assert!(!name.contains("TOKEN") && !name.contains("SECRET"), "{kv}");
        assert!(!kv.contains(TOKEN_PREFIX), "{kv}");
    }
    let inflow = report["inflow"].as_str().unwrap();
    for t in &pool {
        assert!(!inflow.contains(t.as_str()), "a token reached the harness");
    }

    // The memory scan read real memory (it found the canary) and found no token.
    assert!(
        report["scannedBytes"].as_u64().unwrap() > 1 << 20,
        "{brief}"
    );
    assert_eq!(report["canaryFound"], json!(true), "{brief}");
    assert_eq!(report["memoryTokenHits"], json!(0), "{brief}");
}

// ---------- the child ----------

/// The harness child of [`rtr_28_rtr_27_at_42_harness_env_and_memory_hold_no_token`]. The
/// parent starts this binary with [`CHILD_MODE`] in the arguments; in a normal test run the
/// argument is absent and this entry does nothing.
#[test]
fn at_42_harness_child_process_entry() {
    if !std::env::args().any(|a| a == CHILD_MODE) {
        return;
    }
    child_main();
}

/// Every byte the child got from outside.
#[derive(Default)]
struct Inflow(Vec<u8>);

fn rpc(inflow: &mut Inflow, url: &str, auth: &str, msg: &Value) -> Value {
    let rest = url.strip_prefix("http://").unwrap();
    let (authority, path) = rest.split_once('/').unwrap();
    let body = msg.to_string();
    let req = format!(
        "POST /{path} HTTP/1.1\r\nHost: {authority}\r\nAuthorization: {auth}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut s = TcpStream::connect(authority).unwrap();
    s.set_read_timeout(Some(BOUND)).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    inflow.0.extend_from_slice(&out);
    let text = String::from_utf8_lossy(&out).into_owned();
    text.split_once("\r\n\r\n")
        .and_then(|(_, b)| serde_json::from_str(b).ok())
        .unwrap_or(Value::Null)
}

fn tool_ok(v: &Value) -> bool {
    v["result"]["isError"] == json!(false)
}

/// Every candidate `test-token-not-real-<digits>` in `bytes`.
fn candidates(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let prefix = TOKEN_PREFIX.as_bytes();
    let mut i = 0usize;
    std::iter::from_fn(move || {
        while i + prefix.len() <= bytes.len() {
            let at = i;
            i += 1;
            if &bytes[at..at + prefix.len()] != prefix {
                continue;
            }
            let mut end = at + prefix.len();
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > at + prefix.len() {
                return Some(&bytes[at..end]);
            }
        }
        None
    })
}

struct Scan<'a> {
    hashes: &'a [String],
    canary_hash: String,
    token_hits: u64,
    canary_found: bool,
    scanned: u64,
}

impl Scan<'_> {
    fn bytes(&mut self, chunk: &[u8]) {
        self.scanned += chunk.len() as u64;
        for c in candidates(chunk) {
            let h = sha256_hex(c);
            if self.hashes.contains(&h) {
                self.token_hits += 1;
            }
            if h == self.canary_hash {
                self.canary_found = true;
            }
        }
    }
}

fn child_main() {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .expect("the runtime has a scratch home");
    let home = PathBuf::from(home);
    let targets: Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(TARGETS_FILE)).unwrap()).unwrap();
    let hashes: Vec<String> = targets["hashes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h.as_str().unwrap().to_owned())
        .collect();
    // The canary stays in memory as a String: the memory scan must find it.
    let canary = targets["canary"].as_str().unwrap().to_owned();

    let mut inflow = Inflow::default();
    let stdin = std::io::stdin();
    let mut lines = stdin.lock();
    let mut first = String::new();
    lines.read_line(&mut first).unwrap();
    inflow.0.extend_from_slice(first.as_bytes());
    let frame: Value = serde_json::from_str(&first).unwrap();
    let url = frame["genieOneMcp"]["url"].as_str().unwrap().to_owned();
    let auth = frame["genieOneMcp"]["authorization"]
        .as_str()
        .unwrap()
        .to_owned();

    rpc(
        &mut inflow,
        &url,
        &auth,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-06-18", "capabilities": {}}}),
    );
    let ask = rpc(
        &mut inflow,
        &url,
        &auth,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": {"name": "genie_ask", "arguments": {"question": QUESTION}}}),
    );
    let poll = rpc(
        &mut inflow,
        &url,
        &auth,
        &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "genie_poll_response",
                           "arguments": {"conversation_id": "c-1", "response_id": "r-1"}}}),
    );

    let env: Vec<String> = std::env::vars_os()
        .map(|(k, v)| format!("{}={}", k.to_string_lossy(), v.to_string_lossy()))
        .collect();
    for e in &env {
        inflow.0.extend_from_slice(e.as_bytes());
    }
    for a in std::env::args() {
        inflow.0.extend_from_slice(a.as_bytes());
    }
    #[cfg(target_os = "linux")]
    if let Ok(b) = std::fs::read("/proc/self/environ") {
        inflow.0.extend_from_slice(&b);
    }

    let mut flow = Scan {
        hashes: &hashes,
        canary_hash: sha256_hex(canary.as_bytes()),
        token_hits: 0,
        canary_found: false,
        scanned: 0,
    };
    flow.bytes(&inflow.0);
    let inflow_prefix_hits = inflow
        .0
        .windows(TOKEN_PREFIX.len())
        .filter(|w| *w == TOKEN_PREFIX.as_bytes())
        .count();
    let inflow_token_hits = flow.token_hits;

    let mut mem = Scan {
        hashes: &hashes,
        canary_hash: sha256_hex(canary.as_bytes()),
        token_hits: 0,
        canary_found: false,
        scanned: 0,
    };
    memscan::scan_writable(&mut |chunk| mem.bytes(chunk));
    std::hint::black_box(&canary);

    let report = json!({
        "askOk": tool_ok(&ask),
        "pollOk": tool_ok(&poll),
        "env": env,
        "inflow": String::from_utf8_lossy(&inflow.0),
        "inflowTokenHits": inflow_token_hits,
        "inflowPrefixHits": inflow_prefix_hits,
        "memoryTokenHits": mem.token_hits,
        "canaryFound": mem.canary_found,
        "scannedBytes": mem.scanned,
    });
    let tmp = home.join("scan-report.tmp");
    std::fs::write(&tmp, report.to_string()).unwrap();
    std::fs::rename(&tmp, home.join(REPORT_FILE)).unwrap();
    // Stay up until the host stops the runtime, so it does not restart it.
    let mut rest = String::new();
    while lines.read_line(&mut rest).map(|n| n > 0).unwrap_or(false) {
        rest.clear();
    }
}

/// A scan of every writable page of this process. A page that cannot be read is skipped.
mod memscan {
    const CHUNK: usize = 1 << 20;
    const PAGE: u64 = 4096;
    /// Consecutive chunks overlap by this much, so no token or canary is cut in two.
    const OVERLAP: u64 = 128;

    /// Read `start..end` in chunks with `read(addr, buf) -> bytes read`, and give each chunk to
    /// `f`. A chunk that cannot be read is read again page by page; an unreadable page is
    /// skipped, so one bad page never hides the rest of the region.
    fn region(
        start: u64,
        end: u64,
        buf: &mut [u8],
        read: &mut dyn FnMut(u64, &mut [u8]) -> Option<usize>,
        f: &mut dyn FnMut(&[u8]),
    ) {
        let mut at = start;
        let mut page_mode = false;
        while at < end {
            let want = if page_mode { PAGE } else { buf.len() as u64 };
            let n = (end - at).min(want) as usize;
            match read(at, &mut buf[..n]) {
                Some(got) if got > 0 => {
                    f(&buf[..got]);
                    let got = got as u64;
                    at += if at + got < end && got > OVERLAP {
                        got - OVERLAP
                    } else {
                        got
                    };
                }
                _ if !page_mode => page_mode = true,
                _ => {
                    // Skip the unreadable page, then try whole chunks again.
                    at = (at / PAGE + 1) * PAGE;
                    page_mode = false;
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    pub fn scan_writable(f: &mut dyn FnMut(&[u8])) {
        use std::io::{Read, Seek, SeekFrom};
        let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
        let mut mem = std::fs::File::open("/proc/self/mem").unwrap();
        let mut buf = vec![0u8; CHUNK];
        for line in maps.lines() {
            let mut parts = line.split_whitespace();
            let (Some(range), Some(perms)) = (parts.next(), parts.next()) else {
                continue;
            };
            if !perms.starts_with("rw") || line.ends_with("[vvar]") || line.ends_with("[vsyscall]")
            {
                continue;
            }
            let Some((a, b)) = range.split_once('-') else {
                continue;
            };
            let (Ok(at), Ok(end)) = (u64::from_str_radix(a, 16), u64::from_str_radix(b, 16)) else {
                continue;
            };
            region(
                at,
                end,
                &mut buf,
                &mut |addr, b| {
                    mem.seek(SeekFrom::Start(addr)).ok()?;
                    mem.read(b).ok()
                },
                f,
            );
        }
    }

    #[cfg(windows)]
    pub fn scan_writable(f: &mut dyn FnMut(&[u8])) {
        use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
        use windows_sys::Win32::System::Memory::{
            VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_EXECUTE_READWRITE,
            PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, PAGE_READWRITE, PAGE_WRITECOPY,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let writable =
            PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY;
        let mut buf = vec![0u8; CHUNK];
        let mut addr: usize = 0;
        loop {
            // SAFETY: an all-zero MEMORY_BASIC_INFORMATION is a valid value; VirtualQuery
            // fills it for the region at `addr` and only reads the address.
            let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            let got = unsafe {
                VirtualQuery(
                    addr as *const _,
                    &mut info,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if got == 0 {
                break;
            }
            let base = info.BaseAddress as usize;
            let size = info.RegionSize;
            if info.State == MEM_COMMIT
                && info.Protect & writable != 0
                && info.Protect & (PAGE_GUARD | PAGE_NOACCESS) == 0
            {
                region(
                    base as u64,
                    (base + size) as u64,
                    &mut buf,
                    &mut |at, b| {
                        let mut read = 0usize;
                        // SAFETY: ReadProcessMemory copies from this process into `b`, which
                        // holds `b.len()` bytes; a page that is gone makes it fail, not fault.
                        let ok = unsafe {
                            ReadProcessMemory(
                                GetCurrentProcess(),
                                at as usize as *const _,
                                b.as_mut_ptr().cast(),
                                b.len(),
                                &mut read,
                            )
                        };
                        (ok != 0).then_some(read)
                    },
                    f,
                );
            }
            match base.checked_add(size) {
                Some(next) if next > addr => addr = next,
                _ => break,
            }
        }
    }

    #[cfg(target_os = "macos")]
    pub fn scan_writable(f: &mut dyn FnMut(&[u8])) {
        #[repr(C, packed(4))]
        #[derive(Default)]
        struct BasicInfo64 {
            protection: i32,
            max_protection: i32,
            inheritance: u32,
            shared: u32,
            reserved: u32,
            offset: u64,
            behavior: i32,
            user_wired_count: u16,
        }
        const VM_REGION_BASIC_INFO_64: i32 = 9;
        const VM_PROT_READ: i32 = 1;
        const VM_PROT_WRITE: i32 = 2;
        extern "C" {
            /// The task port of this process (what `mach_task_self()` reads).
            static mach_task_self_: u32;
            fn mach_vm_region(
                task: u32,
                address: *mut u64,
                size: *mut u64,
                flavor: i32,
                info: *mut i32,
                count: *mut u32,
                object_name: *mut u32,
            ) -> i32;
            fn mach_vm_read_overwrite(
                task: u32,
                address: u64,
                size: u64,
                data: u64,
                out_size: *mut u64,
            ) -> i32;
        }
        // SAFETY: a plain read of a process global that the kernel sets before `main`.
        let task = unsafe { mach_task_self_ };
        let mut buf = vec![0u8; CHUNK];
        let mut addr: u64 = 0;
        loop {
            let mut size: u64 = 0;
            let mut info = BasicInfo64::default();
            let mut count = (std::mem::size_of::<BasicInfo64>() / 4) as u32;
            let mut object: u32 = 0;
            // SAFETY: every out pointer is a live local of the right size; `count` holds the
            // size of `info` in 4-byte words, as the call needs.
            let kr = unsafe {
                mach_vm_region(
                    task,
                    &mut addr,
                    &mut size,
                    VM_REGION_BASIC_INFO_64,
                    (&mut info as *mut BasicInfo64).cast(),
                    &mut count,
                    &mut object,
                )
            };
            if kr != 0 {
                break;
            }
            let prot = info.protection;
            if prot & VM_PROT_READ != 0 && prot & VM_PROT_WRITE != 0 {
                region(
                    addr,
                    addr + size,
                    &mut buf,
                    &mut |at, b| {
                        let mut out: u64 = 0;
                        // SAFETY: the kernel copies at most `b.len()` bytes into `b`; a page
                        // that is gone makes the call fail, not fault.
                        let kr = unsafe {
                            mach_vm_read_overwrite(
                                task,
                                at,
                                b.len() as u64,
                                b.as_mut_ptr() as u64,
                                &mut out,
                            )
                        };
                        (kr == 0).then_some(out as usize)
                    },
                    f,
                );
            }
            addr += size;
        }
    }
}
