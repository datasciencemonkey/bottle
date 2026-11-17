//! R6 TokenBroker (#18): in-memory tokens, refresh at expiry - 60 s, single-flight, drop on
//! 401, host binding and the cached `workspace_id`.
//!
//! Doubles only: a fake CliRunner, a fake SCIM seam and simulated clocks. No live workspace and
//! no wall-clock window: every wait is on a condition, and time moves only when a test moves it.
//! Every token is a plainly fake value (`test-token-not-real`), never a real format.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use mog_core::cli_runner::{auth_token_args, refuse_token_command, CliRunner};
use mog_core::clock::{MonotonicClock, SimClock, SimWallClock, WallClock};
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::ProfileHosts;
use mog_core::token_broker::{
    CliTokenBroker, ScimMe, TokenBroker, DEFAULT_LIFETIME, LOGIN_REQUIRED, ORG_ID_MISSING,
    PROFILE_INVALID, REFRESH_MARGIN, TOKEN_HOST_MISMATCH, TOKEN_REPLY_INVALID,
    WORKSPACE_ID_INVALID,
};
use mog_core::types::TokenState;

const TOKEN: &str = "test-token-not-real";
const TOKEN_2: &str = "test-token-not-real-2";
const PROFILE: &str = "bottle-test";
const OTHER: &str = "bottle-other";
const HOST: &str = "h.cloud.databricks.com";
const OTHER_HOST: &str = "other.cloud.databricks.com";
/// 2026-11-17T12:00:00Z in Unix milliseconds.
const T0_MS: i64 = 1_794_916_800_000;

// ---------- doubles ----------

/// What the fake CLI answers for one `auth token` run.
#[derive(Clone)]
enum Answer {
    /// `{"access_token": <token>, "token_type": "Bearer", "expiry": <expiry>}`.
    Expiry(&'static str, &'static str),
    /// `{"access_token": <token>, "expires_in": <secs>}`.
    ExpiresIn(&'static str, u64),
    /// A JSON reply with no expiry field.
    NoExpiry(&'static str),
    /// Raw stdout.
    Raw(&'static str),
    /// The CLI failed (for example the sign-in has expired).
    Fail,
}

/// A fake R5: counts every call, answers `auth token` from a script, and can hold the first
/// `auth token` run at a gate until the test opens it.
#[derive(Default)]
struct FakeCli {
    script: Mutex<BTreeMap<String, Vec<Answer>>>,
    token_runs: Mutex<Vec<String>>,
    json_runs: AtomicUsize,
    gate: Mutex<Option<bool>>,
    gate_cv: Condvar,
    entered: AtomicUsize,
}

impl FakeCli {
    fn answering(profile: &str, answers: Vec<Answer>) -> Arc<Self> {
        let cli = Self::default();
        cli.script
            .lock()
            .unwrap()
            .insert(profile.to_owned(), answers);
        Arc::new(cli)
    }
    fn also(self: &Arc<Self>, profile: &str, answers: Vec<Answer>) -> Arc<Self> {
        self.script
            .lock()
            .unwrap()
            .insert(profile.to_owned(), answers);
        Arc::clone(self)
    }
    fn runs(&self) -> usize {
        self.token_runs.lock().unwrap().len()
    }
    fn runs_for(&self, profile: &str) -> usize {
        self.token_runs
            .lock()
            .unwrap()
            .iter()
            .filter(|p| *p == profile)
            .count()
    }
    /// Close the gate: the next `auth token` run blocks until [`FakeCli::open`].
    fn close(&self) {
        *self.gate.lock().unwrap() = Some(false);
    }
    fn open(&self) {
        *self.gate.lock().unwrap() = Some(true);
        self.gate_cv.notify_all();
    }
}

impl CliRunner for FakeCli {
    fn resolve(&self) -> CoreResult<PathBuf> {
        Ok(PathBuf::from("/usr/local/bin/databricks"))
    }

    fn run_json(&self, args: &[&str]) -> CoreResult<serde_json::Value> {
        refuse_token_command(args)?;
        self.json_runs.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({}))
    }

    fn run_auth_token(&self, profile: &str) -> CoreResult<Vec<u8>> {
        self.token_runs.lock().unwrap().push(profile.to_owned());
        self.entered.fetch_add(1, Ordering::SeqCst);
        let mut g = self.gate.lock().unwrap();
        while *g == Some(false) {
            g = self.gate_cv.wait(g).unwrap();
        }
        drop(g);
        let answer = {
            let mut script = self.script.lock().unwrap();
            let list = script.get_mut(profile).expect("scripted profile");
            if list.len() > 1 {
                list.remove(0)
            } else {
                list[0].clone()
            }
        };
        let text = match answer {
            Answer::Expiry(t, e) => {
                format!(r#"{{"access_token":"{t}","token_type":"Bearer","expiry":"{e}"}}"#)
            }
            Answer::ExpiresIn(t, s) => format!(r#"{{"access_token":"{t}","expires_in":{s}}}"#),
            Answer::NoExpiry(t) => format!(r#"{{"access_token":"{t}","token_type":"Bearer"}}"#),
            Answer::Raw(r) => r.to_owned(),
            Answer::Fail => {
                return Err(CoreError::Unavailable {
                    component: "R5",
                    reason: "cli_failed",
                })
            }
        };
        Ok(text.into_bytes())
    }
}

struct Hosts(BTreeMap<String, String>);

impl ProfileHosts for Hosts {
    fn workspace_url(&self, profile: &str) -> CoreResult<String> {
        self.0.get(profile).cloned().ok_or(CoreError::Unavailable {
            component: "test",
            reason: "no_profile",
        })
    }
}

fn hosts() -> Arc<Hosts> {
    Arc::new(Hosts(BTreeMap::from([
        (PROFILE.to_owned(), format!("https://{HOST}")),
        (OTHER.to_owned(), format!("https://{OTHER_HOST}/")),
    ])))
}

/// A fake SCIM seam: counts calls per profile and answers the `x-databricks-org-id` header.
#[derive(Default)]
struct FakeScim {
    calls: Mutex<Vec<String>>,
    header: Mutex<BTreeMap<String, Option<String>>>,
}

impl FakeScim {
    fn with(entries: &[(&str, Option<&str>)]) -> Arc<Self> {
        let s = Self::default();
        for (p, h) in entries {
            s.header
                .lock()
                .unwrap()
                .insert((*p).to_owned(), h.map(str::to_owned));
        }
        Arc::new(s)
    }
    fn calls_for(&self, profile: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|p| *p == profile)
            .count()
    }
}

impl ScimMe for FakeScim {
    fn org_id_header(&self, profile: &str) -> CoreResult<Option<String>> {
        self.calls.lock().unwrap().push(profile.to_owned());
        Ok(self.header.lock().unwrap().get(profile).cloned().flatten())
    }
}

struct Rig {
    broker: Arc<CliTokenBroker>,
    cli: Arc<FakeCli>,
    mono: SimClock,
    wall: SimWallClock,
}

impl Rig {
    fn advance(&self, d: Duration) {
        self.mono.advance(d);
        self.wall.advance_ms(d.as_millis() as i64);
    }
}

fn rig(cli: Arc<FakeCli>) -> Rig {
    let mono = SimClock::new();
    let wall = SimWallClock::at(T0_MS);
    let m: Arc<dyn MonotonicClock> = Arc::new(mono.clone());
    let w: Arc<dyn WallClock> = Arc::new(wall.clone());
    let broker = Arc::new(CliTokenBroker::new(cli.clone(), hosts(), m, w));
    Rig {
        broker,
        cli,
        mono,
        wall,
    }
}

/// One hour after T0, the usual U2M token lifetime.
const IN_ONE_HOUR: &str = "2026-11-17T13:00:00Z";

fn lend(b: &dyn TokenBroker, profile: &str, host: &str) -> CoreResult<String> {
    let mut seen = None;
    b.with_token(profile, host, &mut |bytes| {
        seen = Some(String::from_utf8(bytes.to_vec()).unwrap())
    })?;
    Ok(seen.expect("the broker lent the token"))
}

fn code(r: CoreResult<String>) -> &'static str {
    r.expect_err("an error").code()
}

/// Wait (without a time window) until `cond` holds.
fn wait_until(cond: impl Fn() -> bool) {
    while !cond() {
        thread::yield_now();
    }
}

// ---------- W-04a: only `databricks auth token --profile <p> --output json` ----------

#[test]
fn w_04a_the_token_command_is_exactly_auth_token_profile_output_json() {
    assert_eq!(
        auth_token_args("bottle-test"),
        [
            "auth",
            "token",
            "--profile",
            "bottle-test",
            "--output",
            "json"
        ]
    );
    // R5 still refuses the same command on the general path (ARC-6).
    assert_eq!(
        refuse_token_command(&auth_token_args("p"))
            .unwrap_err()
            .code(),
        "token_command_refused"
    );
}

#[test]
fn w_04a_tokens_come_only_from_the_auth_token_run_of_the_given_profile() {
    let r = rig(FakeCli::answering(
        PROFILE,
        vec![Answer::Expiry(TOKEN, IN_ONE_HOUR)],
    ));
    assert_eq!(lend(&*r.broker, PROFILE, HOST).unwrap(), TOKEN);
    assert_eq!(*r.cli.token_runs.lock().unwrap(), vec![PROFILE.to_owned()]);
    assert_eq!(
        r.cli.json_runs.load(Ordering::SeqCst),
        0,
        "no other CLI call"
    );
}

#[test]
fn w_04a_a_profile_name_that_could_be_a_flag_is_refused_before_any_cli_run() {
    let r = rig(FakeCli::answering(
        PROFILE,
        vec![Answer::Expiry(TOKEN, IN_ONE_HOUR)],
    ));
    for bad in ["", "-x", "--output", "a b", "p;rm", "p\n", &"p".repeat(65)] {
        assert_eq!(
            code(lend(&*r.broker, bad, HOST)),
            PROFILE_INVALID,
            "{bad:?}"
        );
    }
    assert_eq!(r.cli.runs(), 0);
}

#[test]
fn w_04a_a_cli_failure_is_login_required_with_no_token_text() {
    let r = rig(FakeCli::answering(PROFILE, vec![Answer::Fail]));
    let e = lend(&*r.broker, PROFILE, HOST).unwrap_err();
    assert_eq!(e.code(), LOGIN_REQUIRED);
    assert_eq!(r.broker.token_state(PROFILE), TokenState::LoginRequired);
}

#[test]
fn w_04a_a_bad_cli_reply_is_refused_and_never_echoed() {
    for raw in [
        "not json test-token-not-real",
        r#"{"token_type":"Bearer"}"#,
        r#"{"access_token":""}"#,
        r#"{"access_token":"test-token-not-real","expiry":"yesterday"}"#,
        // Already expired when the CLI handed it over.
        r#"{"access_token":"test-token-not-real","expiry":"2026-11-17T11:00:00Z"}"#,
    ] {
        let r = rig(FakeCli::answering(PROFILE, vec![Answer::Raw(raw)]));
        let e = lend(&*r.broker, PROFILE, HOST).unwrap_err();
        assert_eq!(e.code(), TOKEN_REPLY_INVALID, "{raw}");
        assert!(!format!("{e} {e:?}").contains(TOKEN), "{raw}");
    }
}

// ---------- W-01b, W-04b: memory only, redacted, dropped ----------

#[test]
fn w_01b_w_04b_debug_and_errors_never_show_the_token() {
    let r = rig(FakeCli::answering(
        PROFILE,
        vec![Answer::Expiry(TOKEN, IN_ONE_HOUR)],
    ));
    lend(&*r.broker, PROFILE, HOST).unwrap();
    let shown = format!("{:?}", r.broker);
    assert!(!shown.contains(TOKEN), "{shown}");
    assert!(!shown.contains("test-token"), "{shown}");
    let e = r
        .broker
        .with_token(PROFILE, OTHER_HOST, &mut |_| ())
        .unwrap_err();
    assert!(!format!("{e} {e:?}").contains(TOKEN));
}

#[test]
fn w_01b_token_broker_source_never_touches_the_file_system() {
    let src =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/token_broker.rs"))
            .unwrap();
    for banned in [
        "std::fs",
        "File::",
        "OpenOptions",
        "println!",
        "eprintln!",
        "log::",
        "tracing::",
        "dbg!",
    ] {
        assert!(!src.contains(banned), "token_broker.rs uses {banned}");
    }
}

#[test]
fn w_04b_drop_all_wipes_every_profile_so_the_next_call_runs_the_cli_again() {
    let cli = FakeCli::answering(
        PROFILE,
        vec![
            Answer::Expiry(TOKEN, IN_ONE_HOUR),
            Answer::Expiry(TOKEN_2, IN_ONE_HOUR),
        ],
    )
    .also(OTHER, vec![Answer::Expiry(TOKEN, IN_ONE_HOUR)]);
    let r = rig(cli);
    lend(&*r.broker, PROFILE, HOST).unwrap();
    lend(&*r.broker, OTHER, OTHER_HOST).unwrap();
    r.broker.drop_all();
    assert_eq!(r.broker.token_state(PROFILE), TokenState::Unknown);
    assert_eq!(r.broker.token_state(OTHER), TokenState::Unknown);
    assert_eq!(lend(&*r.broker, PROFILE, HOST).unwrap(), TOKEN_2);
    assert_eq!(r.cli.runs_for(PROFILE), 2);
}

// ---------- W-05a: refresh 60 s before expiry, one CLI call per profile ----------

#[test]
fn w_05a_n_mogs_on_one_profile_cause_one_cli_call_until_expiry_minus_60_s() {
    let r = rig(FakeCli::answering(
        PROFILE,
        vec![
            Answer::Expiry(TOKEN, IN_ONE_HOUR),
            Answer::Expiry(TOKEN_2, "2026-11-17T14:00:00Z"),
        ],
    ));
    for _mog in 0..8 {
        assert_eq!(lend(&*r.broker, PROFILE, HOST).unwrap(), TOKEN);
    }
    assert_eq!(r.cli.runs(), 1);
    r.advance(Duration::from_secs(3_600 - 60) - Duration::from_millis(1));
    for _mog in 0..8 {
        assert_eq!(lend(&*r.broker, PROFILE, HOST).unwrap(), TOKEN);
    }
    assert_eq!(r.cli.runs(), 1, "no refresh before expiry - 60 s");
    r.advance(Duration::from_millis(1));
    assert_eq!(r.mono.now(), Duration::from_secs(3_540));
    for _mog in 0..8 {
        assert_eq!(lend(&*r.broker, PROFILE, HOST).unwrap(), TOKEN_2);
    }
    assert_eq!(r.cli.runs(), 2, "one refresh for 8 MOGs at 3,540 s");
    assert_eq!(REFRESH_MARGIN, Duration::from_secs(60));
}

#[test]
fn w_05a_expires_in_and_an_rfc3339_offset_give_the_same_refresh_time() {
    for answer in [
        Answer::ExpiresIn(TOKEN, 3_600),
        Answer::Expiry(TOKEN, "2026-11-17T14:00:00.000000+01:00"),
    ] {
        let r = rig(FakeCli::answering(PROFILE, vec![answer]));
        lend(&*r.broker, PROFILE, HOST).unwrap();
        r.advance(Duration::from_secs(3_539));
        lend(&*r.broker, PROFILE, HOST).unwrap();
        assert_eq!(r.cli.runs(), 1);
        assert_eq!(r.broker.token_state(PROFILE), TokenState::Valid);
        r.advance(Duration::from_secs(1));
        assert_eq!(r.broker.token_state(PROFILE), TokenState::Expiring);
        lend(&*r.broker, PROFILE, HOST).unwrap();
        assert_eq!(r.cli.runs(), 2);
    }
}

#[test]
fn w_05a_a_reply_without_expiry_refreshes_after_the_default_lifetime() {
    let r = rig(FakeCli::answering(PROFILE, vec![Answer::NoExpiry(TOKEN)]));
    lend(&*r.broker, PROFILE, HOST).unwrap();
    r.advance(DEFAULT_LIFETIME - Duration::from_millis(1));
    lend(&*r.broker, PROFILE, HOST).unwrap();
    assert_eq!(r.cli.runs(), 1);
    r.advance(Duration::from_millis(1));
    lend(&*r.broker, PROFILE, HOST).unwrap();
    assert_eq!(r.cli.runs(), 2);
}

#[test]
fn w_05a_concurrent_callers_on_one_profile_share_one_cli_run() {
    let cli = FakeCli::answering(PROFILE, vec![Answer::Expiry(TOKEN, IN_ONE_HOUR)]);
    cli.close();
    let r = rig(cli);
    let mut joins = Vec::new();
    for _mog in 0..8 {
        let b = Arc::clone(&r.broker);
        joins.push(thread::spawn(move || lend(&*b, PROFILE, HOST)));
    }
    // One caller is inside the CLI and the other 7 wait for it.
    wait_until(|| r.cli.entered.load(Ordering::SeqCst) == 1 && r.broker.waiting(PROFILE) == 7);
    r.cli.open();
    for j in joins {
        assert_eq!(j.join().unwrap().unwrap(), TOKEN);
    }
    assert_eq!(r.cli.runs(), 1, "8 concurrent MOGs cause one CLI call");
}

#[test]
fn w_05a_a_failed_refresh_is_shared_and_other_profiles_keep_working() {
    let cli = FakeCli::answering(PROFILE, vec![Answer::Fail])
        .also(OTHER, vec![Answer::Expiry(TOKEN, IN_ONE_HOUR)]);
    cli.close();
    let r = rig(cli);
    let mut joins = Vec::new();
    for _mog in 0..4 {
        let b = Arc::clone(&r.broker);
        joins.push(thread::spawn(move || lend(&*b, PROFILE, HOST)));
    }
    wait_until(|| r.cli.entered.load(Ordering::SeqCst) == 1 && r.broker.waiting(PROFILE) == 3);
    r.cli.open();
    for j in joins {
        assert_eq!(code(j.join().unwrap()), LOGIN_REQUIRED);
    }
    assert_eq!(
        r.cli.runs_for(PROFILE),
        1,
        "the waiters share the one failure"
    );
    // ARC-F5: other profiles are not affected.
    assert_eq!(lend(&*r.broker, OTHER, OTHER_HOST).unwrap(), TOKEN);
}

// ---------- W-05b, W-05c: drop on 401, then one CLI run ----------

#[test]
fn w_05b_w_05c_invalidate_drops_the_token_and_the_next_call_runs_the_cli_once() {
    let r = rig(FakeCli::answering(
        PROFILE,
        vec![
            Answer::Expiry(TOKEN, IN_ONE_HOUR),
            Answer::Expiry(TOKEN_2, IN_ONE_HOUR),
        ],
    ));
    assert_eq!(lend(&*r.broker, PROFILE, HOST).unwrap(), TOKEN);
    r.broker.invalidate(PROFILE).unwrap();
    assert_eq!(r.broker.token_state(PROFILE), TokenState::Unknown);
    for _mog in 0..4 {
        assert_eq!(lend(&*r.broker, PROFILE, HOST).unwrap(), TOKEN_2);
    }
    assert_eq!(r.cli.runs(), 2, "one CLI run after the drop");
}

#[test]
fn w_05b_invalidate_touches_only_its_profile() {
    let cli = FakeCli::answering(PROFILE, vec![Answer::Expiry(TOKEN, IN_ONE_HOUR)])
        .also(OTHER, vec![Answer::Expiry(TOKEN, IN_ONE_HOUR)]);
    let r = rig(cli);
    lend(&*r.broker, PROFILE, HOST).unwrap();
    lend(&*r.broker, OTHER, OTHER_HOST).unwrap();
    r.broker.invalidate(PROFILE).unwrap();
    lend(&*r.broker, OTHER, OTHER_HOST).unwrap();
    assert_eq!(r.cli.runs_for(OTHER), 1);
    assert_eq!(r.broker.token_state(OTHER), TokenState::Valid);
}

// ---------- 4.6 rule 2: a token goes only to the host of its profile ----------

#[test]
fn arc_6_rule_2_a_token_is_lent_only_for_the_host_of_its_profile() {
    let r = rig(FakeCli::answering(
        PROFILE,
        vec![Answer::Expiry(TOKEN, IN_ONE_HOUR)],
    ));
    for wrong in [
        OTHER_HOST,
        "h.cloud.databricks.com.evil.example",
        "evil.example",
        "",
        "accounts.cloud.databricks.com",
    ] {
        let mut called = false;
        let e = r
            .broker
            .with_token(PROFILE, wrong, &mut |_| called = true)
            .unwrap_err();
        assert_eq!(e.code(), TOKEN_HOST_MISMATCH, "{wrong}");
        assert!(!called, "{wrong}: the token was lent");
    }
    // The host compare ignores ASCII case only.
    assert_eq!(
        lend(&*r.broker, PROFILE, "H.Cloud.Databricks.com").unwrap(),
        TOKEN
    );
}

// ---------- W-06a, W-06b: workspace_id from x-databricks-org-id, once per profile ----------

fn scim_rig(entries: &[(&str, Option<&str>)]) -> (Rig, Arc<FakeScim>) {
    let cli = FakeCli::answering(PROFILE, vec![Answer::Expiry(TOKEN, IN_ONE_HOUR)])
        .also(OTHER, vec![Answer::Expiry(TOKEN, IN_ONE_HOUR)]);
    let r = rig(cli);
    let scim = FakeScim::with(entries);
    let weak = Arc::downgrade(&scim);
    r.broker.bind_scim(weak);
    (r, scim)
}

#[test]
fn w_06a_w_06b_workspace_id_comes_from_the_org_id_header_once_per_profile() {
    let (r, scim) = scim_rig(&[(PROFILE, Some("1234")), (OTHER, Some("5678"))]);
    for _ in 0..5 {
        assert_eq!(r.broker.workspace_id(PROFILE).unwrap(), "1234");
    }
    assert_eq!(
        scim.calls_for(PROFILE),
        1,
        "one SCIM call per profile per launch"
    );
    assert_eq!(r.broker.workspace_id(OTHER).unwrap(), "5678");
    assert_eq!(scim.calls_for(OTHER), 1);
    // A dropped token does not drop the workspace ID.
    r.broker.invalidate(PROFILE).unwrap();
    r.broker.workspace_id(PROFILE).unwrap();
    assert_eq!(scim.calls_for(PROFILE), 1);
}

#[test]
fn w_06a_a_missing_or_bad_org_id_is_refused_and_not_cached() {
    let (r, scim) = scim_rig(&[(PROFILE, None), (OTHER, Some("12ab"))]);
    assert_eq!(code(r.broker.workspace_id(PROFILE)), ORG_ID_MISSING);
    assert_eq!(code(r.broker.workspace_id(PROFILE)), ORG_ID_MISSING);
    assert_eq!(scim.calls_for(PROFILE), 2, "a failure is not cached");
    assert_eq!(code(r.broker.workspace_id(OTHER)), WORKSPACE_ID_INVALID);
}

#[test]
fn w_06a_the_genie_one_endpoint_always_carries_o_workspace_id() {
    let (r, _scim) = scim_rig(&[(PROFILE, Some("1234"))]);
    let e = r.broker.genie_one_endpoint(PROFILE).unwrap();
    assert_eq!(
        e.url(),
        "https://h.cloud.databricks.com/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=1234"
    );
}

#[test]
fn w_06a_with_no_scim_seam_bound_workspace_id_is_unavailable() {
    let r = rig(FakeCli::answering(
        PROFILE,
        vec![Answer::Expiry(TOKEN, IN_ONE_HOUR)],
    ));
    assert!(r.broker.workspace_id(PROFILE).is_err());
}

// ---------- token status for TS: the state only ----------

#[test]
fn w_04b_token_state_is_unknown_before_the_first_token_and_never_holds_bytes() {
    let r = rig(FakeCli::answering(
        PROFILE,
        vec![Answer::Expiry(TOKEN, IN_ONE_HOUR)],
    ));
    assert_eq!(r.broker.token_state(PROFILE), TokenState::Unknown);
    lend(&*r.broker, PROFILE, HOST).unwrap();
    assert_eq!(r.broker.token_state(PROFILE), TokenState::Valid);
    let shown = format!("{:?}", r.broker.token_state(PROFILE));
    assert!(!shown.contains(TOKEN));
    let _ = r.wall.now_ms();
}

// ---------- ARC-6: only R6 runs the token command ----------

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn arc_6_only_token_broker_calls_run_auth_token() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../..");
    let mut files = Vec::new();
    for dir in [
        "crates/mog-core/src",
        "crates/mog-core-ffi/src",
        "src-tauri/src",
    ] {
        rust_files(&root.join(dir), &mut files);
    }
    let allowed = [
        manifest.join("src/token_broker.rs").canonicalize().unwrap(),
        manifest.join("src/cli_runner.rs").canonicalize().unwrap(),
    ];
    let offenders: Vec<_> = files
        .iter()
        .filter(|f| !allowed.contains(&f.canonicalize().unwrap()))
        .filter(|f| {
            std::fs::read_to_string(f)
                .unwrap()
                .contains("run_auth_token")
        })
        .collect();
    assert!(offenders.is_empty(), "{offenders:?}");
}
