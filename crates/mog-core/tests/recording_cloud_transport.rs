//! R7 recording routes (ADR 0103, PR C) on a std loopback server: raw upload body, raw
//! download reply, status kept for 403/404, volume path slot rules, page refusal, the model
//! route with tags, and no reach from the IPC `dbx_http` route map. Tokens are plainly fake.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::egress_policy::TokenEgressPolicy;
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::host_policy::AllowlistEgressPolicy;
use mog_core::http_transport::rate_bucket::AimdBucket;
use mog_core::http_transport::rate_bucket::RateBucket;
use mog_core::http_transport::routes::{recording_route, route, RECORDING_ROUTE_TABLE};
use mog_core::http_transport::{
    HttpTransport, Pause, ProfileHosts, ReqwestHttpTransport, TransportSeams,
};
use mog_core::ipc::JsonObject;
use mog_core::recording_cloud::http::{CallBounds, RecordingHttp, ReplyBody, ReqBody};
use mog_core::recording_cloud::opener::{open_link, UrlOpener};
use mog_core::request_identity::{BottleIdentity, TAGS_HEADER};
use mog_core::token_broker::{TokenBroker, TokenLease};
use mog_core::types::Tags;
use serde_json::{json, Value};

const TOKEN: &str = "test-token-not-real";

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn header<'a>(s: &'a Seen, name: &str) -> Option<&'a str> {
    s.headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Serve `(status, content type, body)` replies in order and record each request.
fn serve(replies: Vec<(u16, Option<&'static str>, Vec<u8>)>) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let rec = seen.clone();
    thread::spawn(move || {
        for (status, ct, body) in replies {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            let head_end = loop {
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&chunk[..n]);
            };
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let mut lines = head.split("\r\n");
            let mut first = lines.next().unwrap().split(' ');
            let method = first.next().unwrap().to_owned();
            let target = first.next().unwrap().to_owned();
            let headers: Vec<(String, String)> = lines
                .filter_map(|l| l.split_once(':'))
                .map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))
                .collect();
            let len = headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.parse::<usize>().ok())
                .unwrap_or(0);
            let mut b = buf[head_end..].to_vec();
            while b.len() < len {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                b.extend_from_slice(&chunk[..n]);
            }
            rec.lock().unwrap().push(Seen {
                method,
                target,
                headers,
                body: b,
            });
            let mut out = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            if let Some(ct) = ct {
                out.push_str(&format!("Content-Type: {ct}\r\n"));
            }
            out.push_str("\r\n");
            let _ = s.write_all(out.as_bytes());
            let _ = s.write_all(&body);
        }
    });
    (format!("http://127.0.0.1:{port}"), seen)
}

struct Hosts(String);
impl ProfileHosts for Hosts {
    fn workspace_url(&self, _p: &str) -> CoreResult<String> {
        Ok(self.0.clone())
    }
}

struct Broker;
impl TokenBroker for Broker {
    fn with_token(&self, _p: &str, _h: &str, f: &mut dyn FnMut(&[u8])) -> CoreResult<TokenLease> {
        f(TOKEN.as_bytes());
        Ok(TokenLease::default())
    }
    fn invalidate(&self, _p: &str, _l: TokenLease) -> CoreResult<()> {
        Ok(())
    }
    fn workspace_id(&self, _p: &str) -> CoreResult<String> {
        Ok("1".into())
    }
}

/// TEST ONLY: plain HTTP to 127.0.0.1; rule 6 is the production rule.
struct Loopback;
impl TokenEgressPolicy for Loopback {
    fn may_send_token(&self, profile_host: &str, url: &str) -> bool {
        profile_host == "127.0.0.1" && url.starts_with("http://127.0.0.1:")
    }
    fn may_follow_redirect_with_authorization(&self, _p: &str, _f: &str, _t: &str) -> bool {
        false
    }
    fn reply_for_ts(&self, ct: Option<&str>, body: &[u8]) -> Option<Value> {
        AllowlistEgressPolicy.reply_for_ts(ct, body)
    }
    fn may_retry_with_token_after_non_json(&self) -> bool {
        false
    }
}

#[derive(Debug, Default)]
struct NoPause(SimClock);
impl Pause for NoPause {
    fn pause(&self, d: Duration) {
        self.0.advance(d);
    }
}

fn transport(origin: &str) -> ReqwestHttpTransport {
    let pause = Arc::new(NoPause::default());
    let clock: Arc<dyn MonotonicClock> = Arc::new(pause.0.clone());
    ReqwestHttpTransport::with_seams(TransportSeams {
        hosts: Arc::new(Hosts(origin.to_owned())),
        tokens: Arc::new(Broker),
        policy: Arc::new(Loopback),
        bucket: Arc::new(AimdBucket::new(clock)),
        decorator: Arc::new(BottleIdentity::new()),
        pause,
    })
    .unwrap()
}

fn params(v: Value) -> JsonObject {
    serde_json::from_value(v).unwrap()
}

const PART: &str =
    "/Volumes/main/team/audio/Bottle/Recordings/2026-11-19_0905/2026-11-19_0905.part01.flac";

#[test]
fn rec_transport_upload_sends_raw_bytes_with_put_and_overwrite_false() {
    let (origin, seen) = serve(vec![(204, None, vec![])]);
    let t = transport(&origin);
    let r = t
        .send(
            "p",
            "files.upload",
            &params(json!({"path": PART, "overwrite": "false"})),
            ReqBody::Bytes(b"fLaC\x00\x01"),
            None,
        )
        .unwrap();
    assert_eq!((r.status, r.body), (204, ReplyBody::Empty));
    let s = seen.lock().unwrap()[0].clone();
    assert_eq!(s.method, "PUT");
    assert_eq!(s.target, format!("/api/2.0/fs/files{PART}?overwrite=false"));
    assert_eq!(s.body, b"fLaC\x00\x01");
    assert_eq!(header(&s, "content-type"), Some("application/octet-stream"));
    assert_eq!(
        header(&s, "authorization"),
        Some(&*format!("Bearer {TOKEN}"))
    );
    assert!(header(&s, "user-agent").unwrap().starts_with("bottle/"));
}

#[test]
fn rec_transport_download_bytes_and_error_status_are_kept() {
    let (origin, _seen) = serve(vec![
        (
            200,
            Some("application/octet-stream"),
            b"{\"version\":1}".to_vec(),
        ),
        (
            404,
            Some("application/json"),
            br#"{"error_code":"NOT_FOUND","message":"The file being accessed is not found."}"#
                .to_vec(),
        ),
        (
            403,
            Some("application/json"),
            br#"{"error_code":"PERMISSION_DENIED","message":"no"}"#.to_vec(),
        ),
    ]);
    let t = transport(&origin);
    let p = params(json!({"path": PART}));
    let ok = t
        .send("p", "files.download", &p, ReqBody::None, None)
        .unwrap();
    assert_eq!(ok.body, ReplyBody::Bytes(b"{\"version\":1}".to_vec()));
    let nf = t
        .send("p", "files.download", &p, ReqBody::None, None)
        .unwrap();
    assert_eq!((nf.status, nf.error_code()), (404, "NOT_FOUND"));
    let dir = params(json!({"path": "/Volumes/c/s/v/"}));
    let den = t
        .send("p", "files.list_directory", &dir, ReqBody::None, None)
        .unwrap();
    assert_eq!((den.status, den.error_code()), (403, "PERMISSION_DENIED"));
}

#[test]
fn arc_6_rec_a_page_is_refused_and_bad_paths_send_nothing() {
    let (origin, seen) = serve(vec![(
        200,
        Some("text/html"),
        b"<html>Sign in</html>".to_vec(),
    )]);
    let t = transport(&origin);
    let e = t
        .send(
            "p",
            "files.download",
            &params(json!({"path": PART})),
            ReqBody::None,
            None,
        )
        .unwrap_err();
    assert_eq!(e.code(), "non_json_reply");
    for bad in [
        "/Volumes/c/s/v/../../x",
        "/Volumes/c/s/v/a?b",
        "https://evil.example/x",
        "/Volumes/c/s",
    ] {
        let e = t
            .send(
                "p",
                "files.download",
                &params(json!({"path": bad})),
                ReqBody::None,
                None,
            )
            .unwrap_err();
        assert_eq!(e.code(), "invalid_params", "{bad}");
    }
    let e = t
        .send(
            "p",
            "files.download",
            &params(json!({"path": PART, "extra": "x"})),
            ReqBody::None,
            None,
        )
        .unwrap_err();
    assert_eq!(e.code(), "invalid_params");
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn arc_6_rec_recording_routes_are_not_ipc_routes() {
    for r in RECORDING_ROUTE_TABLE {
        assert_eq!(route(r.id).unwrap_err().code(), "unknown_route", "{}", r.id);
        assert!(recording_route(r.id).is_ok());
        assert!(r.path.starts_with("/api/"), "{}", r.id);
    }
    let t = transport("http://127.0.0.1:9");
    let e = t
        .request(
            "p",
            "files.upload",
            &BTreeMap::new(),
            None,
            &serde_json::from_value(json!({"app":"bottle","mog":"m1","op":"cleanup"})).unwrap(),
        )
        .unwrap_err();
    assert_eq!(e.code(), "unknown_route");
    let e = t
        .send("p", "scim.me", &BTreeMap::new(), ReqBody::None, None)
        .unwrap_err();
    assert_eq!(e.code(), "unknown_route");
}

#[test]
fn nfr_21a_nfr_21b_sec_06a_rec_model_calls_use_the_gateway_route_with_tags() {
    let (origin, seen) = serve(vec![(
        200,
        Some("application/json"),
        br#"{"choices":[{"message":{"content":"[00:00] Speaker 1: Hi."}}]}"#.to_vec(),
    )]);
    let t = transport(&origin);
    let body = json!({"model":"system.ai.gemini-3-8-flash","messages":[]});
    let e = t
        .send(
            "p",
            "mlflow.chat_completions",
            &JsonObject::new(),
            ReqBody::Json(&body),
            None,
        )
        .unwrap_err();
    assert_eq!(e.code(), "invalid_params", "no tags, no send");
    let tags: Tags = serde_json::from_value(json!({"app":"bottle","mog":"m1","op":"asr"})).unwrap();
    let r = t
        .send(
            "p",
            "mlflow.chat_completions",
            &JsonObject::new(),
            ReqBody::Json(&body),
            Some(&tags),
        )
        .unwrap();
    assert!(r.ok());
    let s = seen.lock().unwrap().clone();
    assert_eq!(s.len(), 1);
    assert_eq!(
        (s[0].method.as_str(), s[0].target.as_str()),
        ("POST", "/ai-gateway/mlflow/v1/chat/completions")
    );
    assert_eq!(header(&s[0], "content-type"), Some("application/json"));
    assert_eq!(
        header(&s[0], TAGS_HEADER),
        Some(r#"{"app":"bottle","mog":"m1","op":"asr"}"#)
    );
    assert!(!s[0].target.contains("serving-endpoints"));
}

// ---------- opener ----------

#[derive(Default)]
struct Opened(Mutex<Vec<String>>);
impl UrlOpener for Opened {
    fn open(&self, url: &str) -> Result<(), CoreError> {
        self.0.lock().unwrap().push(url.to_owned());
        Ok(())
    }
}

#[test]
fn rec_open_link_only_opens_explorer_links_on_the_profile_host() {
    let o = Opened::default();
    let host = "https://dbc-fake.cloud.databricks.com";
    let good = "https://dbc-fake.cloud.databricks.com/explore/data/volumes/main/team/audio?o=1&volumePath=%2FVolumes%2Fmain%2F";
    open_link(&o, host, good).unwrap();
    // A file link of a finished recording is the folder link (Databricks documents no file
    // deep link), so it passes the same strict rule.
    let file = "https://dbc-fake.cloud.databricks.com/explore/data/volumes/main/team/audio?o=1&volumePath=%2FVolumes%2Fmain%2Fteam%2Faudio%2FBottle%2FRecordings%2F2026-11-19_0905%2F";
    open_link(&o, host, file).unwrap();
    for bad in [
        // A file path with a quote, a space, an angle bracket or a foreign scheme.
        "https://dbc-fake.cloud.databricks.com/explore/data/volumes/m/t/a?o=1&volumePath=a\"b",
        "https://dbc-fake.cloud.databricks.com/explore/data/volumes/m/t/a?o=1&volumePath=a b",
        "https://dbc-fake.cloud.databricks.com/explore/data/volumes/m/t/a?o=1&x=<b>",
        "https://dbc-fake.cloud.databricks.com/explore/data/volumes/m/t/a?o=1&volumePath=a;b",
        "https://evil.example/explore/data/volumes/main/team/audio?o=1",
        "http://dbc-fake.cloud.databricks.com/explore/data/volumes/m/t/a?o=1",
        "https://dbc-fake.cloud.databricks.com/login?next=https://evil.example",
        "https://dbc-fake.cloud.databricks.com.evil.example/explore/data/volumes/m/t/a?o=1",
        "https://dbc-fake.cloud.databricks.com/explore/data/volumes/m/t/a?o=1 --flag",
    ] {
        assert!(open_link(&o, host, bad).is_err(), "{bad}");
    }
    assert_eq!(
        o.0.lock().unwrap().clone(),
        vec![good.to_owned(), file.to_owned()]
    );
}

// ---------- bounded model calls (ADR 0122) ----------

const RESPONSES: &str = "mlflow.responses";

fn bounded_transport(
    origin: &str,
    tokens: Arc<dyn TokenBroker>,
) -> (ReqwestHttpTransport, Arc<AimdBucket>) {
    let pause = Arc::new(NoPause::default());
    let clock: Arc<dyn MonotonicClock> = Arc::new(pause.0.clone());
    let bucket = Arc::new(AimdBucket::new(clock));
    let t = ReqwestHttpTransport::with_seams(TransportSeams {
        hosts: Arc::new(Hosts(origin.to_owned())),
        tokens,
        policy: Arc::new(Loopback),
        bucket: bucket.clone(),
        decorator: Arc::new(BottleIdentity::new()),
        pause,
    })
    .unwrap();
    (t, bucket)
}

fn send_bounded(
    t: &ReqwestHttpTransport,
    budget: Duration,
    abandoned: &AtomicBool,
) -> CoreResult<mog_core::recording_cloud::http::Reply> {
    let tags: Tags =
        serde_json::from_value(json!({"app":"bottle","mog":"m1","op":"router"})).unwrap();
    t.send_bounded(
        "p",
        RESPONSES,
        &JsonObject::new(),
        ReqBody::Json(&json!({"model": "m", "input": []})),
        Some(&tags),
        CallBounds { budget, abandoned },
    )
}

#[test]
fn adr_0122_bounded_call_abandoned_before_sending_is_never_sent() {
    let (origin, seen) = serve(vec![(200, Some("application/json"), b"{}".to_vec())]);
    let (t, _) = bounded_transport(&origin, Arc::new(Broker));
    let e = send_bounded(&t, Duration::from_secs(5), &AtomicBool::new(true)).unwrap_err();
    assert_eq!(e.code(), "timeout");
    assert!(seen.lock().unwrap().is_empty());
}

/// A broker that sets the abandoned flag while it fetches the token.
struct AbandoningBroker(Arc<AtomicBool>);
impl TokenBroker for AbandoningBroker {
    fn with_token(&self, p: &str, h: &str, f: &mut dyn FnMut(&[u8])) -> CoreResult<TokenLease> {
        self.0.store(true, Ordering::SeqCst);
        Broker.with_token(p, h, f)
    }
    fn invalidate(&self, _p: &str, _l: TokenLease) -> CoreResult<()> {
        Ok(())
    }
    fn workspace_id(&self, _p: &str) -> CoreResult<String> {
        Ok("1".into())
    }
}

#[test]
fn adr_0122_bounded_call_abandoned_during_the_token_fetch_is_never_sent() {
    let (origin, seen) = serve(vec![(200, Some("application/json"), b"{}".to_vec())]);
    let flag = Arc::new(AtomicBool::new(false));
    let (t, _) = bounded_transport(&origin, Arc::new(AbandoningBroker(flag.clone())));
    let e = send_bounded(&t, Duration::from_secs(5), &flag).unwrap_err();
    assert_eq!(e.code(), "timeout");
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn adr_0122_bounded_call_with_a_bucket_held_past_the_budget_times_out_unsent() {
    let (origin, seen) = serve(vec![(200, Some("application/json"), b"{}".to_vec())]);
    let (t, bucket) = bounded_transport(&origin, Arc::new(Broker));
    bucket.hold("p", Duration::from_secs(60));
    let e = send_bounded(&t, Duration::from_millis(200), &AtomicBool::new(false)).unwrap_err();
    assert_eq!(e.code(), "timeout");
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn adr_0122_bounded_call_to_a_slow_server_returns_about_at_the_budget() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let hold = thread::spawn(move || {
        // Accept and say nothing for longer than the budget.
        let c = listener.accept();
        thread::sleep(Duration::from_secs(3));
        drop(c);
    });
    let (t, _) = bounded_transport(&origin, Arc::new(Broker));
    let started = std::time::Instant::now();
    let e = send_bounded(&t, Duration::from_millis(300), &AtomicBool::new(false)).unwrap_err();
    assert_eq!(e.code(), "timeout");
    assert!(
        started.elapsed() < Duration::from_millis(2500),
        "{:?}",
        started.elapsed()
    );
    drop(hold);
}
