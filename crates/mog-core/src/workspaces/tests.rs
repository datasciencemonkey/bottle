//! Tests of the workspace store, the allow-list and the `workspace_*` component (ADR 0117).
//! Fake T7 and R6 only: no CLI, no token, no network, no real profile.

use super::*;
use crate::mcp_transport::{McpCall, McpCallComponent, McpReply, McpTransport};
use crate::types::Tags;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh directory under the OS temp dir, removed on drop. It holds only test files.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "bottle-workspaces-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&p);
        Self(p)
    }
    fn configs(&self) -> Arc<MogConfigStore> {
        Arc::new(MogConfigStore::new(self.0.join("mogs")))
    }
    fn file(&self) -> PathBuf {
        self.0.join(FILE_NAME)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn obj(v: Value) -> crate::ipc::JsonObject {
    match v {
        Value::Object(m) => m.into_iter().collect(),
        _ => panic!("not an object"),
    }
}

fn put_config(configs: &MogConfigStore, id: &str, name: &str, home: &str) {
    configs
        .write(
            id,
            &obj(json!({"schemaVersion": 1, "name": name, "binding": {"home": home}})),
        )
        .unwrap();
}

struct FakeT7(Vec<Profile>);

impl ProfileSource for FakeT7 {
    fn profiles(&self, _refresh: bool) -> CoreResult<Vec<Profile>> {
        Ok(self.0.clone())
    }
}

fn profile(name: &str, supported: bool) -> Profile {
    Profile {
        name: name.to_owned(),
        host: format!("https://{name}.example.test"),
        auth_type: if supported { "databricks-cli" } else { "" }.to_owned(),
        supported,
        workspace_id: None,
    }
}

/// A fake R6: a cached state and a probe answer for each profile.
#[derive(Default)]
struct FakeR6 {
    states: HashMap<String, TokenState>,
    answers: HashMap<String, Result<(), &'static str>>,
    probed: Mutex<Vec<String>>,
    running: AtomicUsize,
    peak: AtomicUsize,
    delay: Duration,
}

impl SignInSource for FakeR6 {
    fn state(&self, profile: &str) -> TokenState {
        self.states
            .get(profile)
            .copied()
            .unwrap_or(TokenState::Unknown)
    }
    fn probe(&self, profile: &str, _host: &str) -> CoreResult<()> {
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        self.running.fetch_sub(1, Ordering::SeqCst);
        self.probed.lock().unwrap().push(profile.to_owned());
        match self.answers.get(profile) {
            Some(Err(reason)) => Err(CoreError::Unavailable {
                component: "R6",
                reason,
            }),
            _ => Ok(()),
        }
    }
}

fn store(s: &Scratch, t7: Vec<Profile>) -> WorkspaceStore {
    WorkspaceStore::new(&s.0, s.configs(), Arc::new(FakeT7(t7)))
}

fn row<'a>(l: &'a WorkspaceList, p: &str) -> &'a WorkspaceRow {
    l.workspaces.iter().find(|r| r.profile == p).unwrap()
}

// ---------- migration ----------

#[test]
fn ac_1_1_three_configs_on_three_profiles_give_three_available_workspaces_and_one_lamp() {
    let s = Scratch::new();
    let cfg = s.configs();
    put_config(&cfg, "c3", "Three", "p3");
    put_config(&cfg, "c1", "One", "p1");
    put_config(&cfg, "c2", "Two", "p2");
    let w = store(
        &s,
        vec![
            profile("p1", true),
            profile("p2", true),
            profile("p3", true),
        ],
    );
    let l = w.list(false).unwrap();
    assert_eq!(l.workspaces.len(), 3);
    assert!(l.workspaces.iter().all(|r| r.available));
    assert_eq!(l.lamp_mog_id.as_deref(), Some("c1"));
    // FLAG 5: three old homes, so no home is picked.
    assert_eq!(l.home, None);
    assert!(l.workspaces.iter().all(|r| !r.is_home));
}

#[test]
fn ac_1_3_two_configs_on_one_profile_give_one_workspace_with_both_ids_sorted() {
    let s = Scratch::new();
    let cfg = s.configs();
    put_config(&cfg, "b", "B", "p1");
    put_config(&cfg, "a", "A", "p1");
    let w = store(&s, vec![profile("p1", true)]);
    let l = w.list(false).unwrap();
    assert_eq!(l.workspaces.len(), 1);
    assert_eq!(l.workspaces[0].mog_ids, ["a", "b"]);
    // Exactly one distinct profile: it is the home.
    assert_eq!(l.home.as_deref(), Some("p1"));
    assert!(l.workspaces[0].is_home && l.workspaces[0].available);
}

#[test]
fn migration_never_changes_an_old_config_and_writes_a_private_file() {
    let s = Scratch::new();
    let cfg = s.configs();
    put_config(&cfg, "a", "A", "p1");
    let before = fs::read(s.0.join("mogs/a.json")).unwrap();
    store(&s, vec![profile("p1", true)]).list(false).unwrap();
    assert_eq!(fs::read(s.0.join("mogs/a.json")).unwrap(), before);
    let data: Value = serde_json::from_slice(&fs::read(s.file()).unwrap()).unwrap();
    assert_eq!(data["schemaVersion"], 1);
    assert_eq!(data["agent"]["crossWorkspaceData"], false);
    assert_eq!(data["agent"]["genieCallBudget"], 10);
    assert_eq!(data["agent"]["endpoint"], "system.ai.claude-haiku-5-5");
    assert_eq!(data["agent"]["cleanDictation"], true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(s.file()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn flag_5_an_unreadable_config_means_no_home_is_picked() {
    let s = Scratch::new();
    put_config(&s.configs(), "a", "A", "p1");
    // A config file that does not pass the checks may hide a second home.
    fs::write(s.0.join("mogs/bad.json"), b"{ not a config").unwrap();
    let w = store(&s, vec![profile("p1", true)]);
    let l = w.list(false).unwrap();
    assert_eq!(l.home, None);
    assert_eq!(l.workspaces.len(), 1);
    assert_eq!(l.lamp_mog_id.as_deref(), Some("a"));
}

#[test]
fn an_existing_file_is_not_migrated_again() {
    let s = Scratch::new();
    put_config(&s.configs(), "a", "A", "p1");
    let w = store(&s, vec![profile("p1", true), profile("p2", true)]);
    w.list(false).unwrap();
    put_config(&s.configs(), "b", "B", "p2");
    let l = w.list(false).unwrap();
    assert!(row(&l, "p2").mog_ids.is_empty());
    assert!(!row(&l, "p2").available);
}

#[test]
fn a_corrupt_file_is_an_error_and_is_kept() {
    let s = Scratch::new();
    fs::create_dir_all(&s.0).unwrap();
    fs::write(s.file(), b"{ not json").unwrap();
    let w = store(&s, vec![profile("p1", true)]);
    assert_eq!(
        w.list(false).unwrap_err(),
        CoreError::IpcRefused {
            code: "workspace_storage_failed"
        }
    );
    assert_eq!(
        w.set_available("p1", true, Some("a")).unwrap_err(),
        CoreError::IpcRefused {
            code: "workspace_storage_failed"
        }
    );
    assert_eq!(fs::read(s.file()).unwrap(), b"{ not json");
    // The gate fails closed.
    assert!(!w.allows("p1"));
}

// ---------- the list ----------

#[test]
fn ac_1_5_a_new_supported_profile_shows_with_the_switch_off() {
    let s = Scratch::new();
    put_config(&s.configs(), "a", "A", "p1");
    let w = store(&s, vec![profile("p1", true), profile("new", true)]);
    let l = w.list(false).unwrap();
    let r = row(&l, "new");
    assert!(!r.available && !r.is_home && r.mog_ids.is_empty());
    assert_eq!(l.workspaces[0].profile, "p1", "home first");
}

#[test]
fn sign_in_comes_from_the_cached_r6_state_and_t7() {
    let s = Scratch::new();
    let cfg = s.configs();
    for (id, p) in [
        ("a", "ok"),
        ("b", "old"),
        ("c", "unk"),
        ("d", "pat"),
        ("e", "gone"),
    ] {
        put_config(&cfg, id, id, p);
    }
    let w = store(
        &s,
        vec![
            profile("ok", true),
            profile("old", true),
            profile("unk", true),
            profile("pat", false),
        ],
    );
    let r6 = FakeR6 {
        states: HashMap::from([
            ("ok".to_owned(), TokenState::Expiring),
            ("old".to_owned(), TokenState::LoginRequired),
        ]),
        ..FakeR6::default()
    };
    w.bind_sign_in(Arc::new(r6));
    let l = w.list(false).unwrap();
    assert_eq!(row(&l, "ok").sign_in, WorkspaceSignIn::SignedIn);
    assert_eq!(row(&l, "old").sign_in, WorkspaceSignIn::Expired);
    assert_eq!(row(&l, "unk").sign_in, WorkspaceSignIn::Unknown);
    assert_eq!(row(&l, "pat").sign_in, WorkspaceSignIn::Unsupported);
    let gone = row(&l, "gone");
    assert_eq!(
        (gone.sign_in, gone.host.as_str()),
        (WorkspaceSignIn::Missing, "")
    );
}

#[test]
fn refresh_probes_only_unknown_profiles_and_maps_an_auth_error_to_expired() {
    let s = Scratch::new();
    let cfg = s.configs();
    for (id, p) in [("a", "ok"), ("b", "old"), ("c", "cached"), ("d", "off")] {
        put_config(&cfg, id, id, p);
    }
    let w = store(
        &s,
        vec![
            profile("ok", true),
            profile("old", true),
            profile("cached", true),
            profile("off", true),
            profile("pat", false),
        ],
    );
    let r6 = Arc::new(FakeR6 {
        states: HashMap::from([("cached".to_owned(), TokenState::Valid)]),
        answers: HashMap::from([
            ("old".to_owned(), Err("login_required")),
            ("off".to_owned(), Err("offline")),
        ]),
        ..FakeR6::default()
    });
    w.bind_sign_in(r6.clone());
    let l = w.list(true).unwrap();
    assert_eq!(row(&l, "ok").sign_in, WorkspaceSignIn::SignedIn);
    assert_eq!(row(&l, "old").sign_in, WorkspaceSignIn::Expired);
    assert_eq!(row(&l, "cached").sign_in, WorkspaceSignIn::SignedIn);
    // A network failure is no proof of an expired sign-in.
    assert_eq!(row(&l, "off").sign_in, WorkspaceSignIn::Unknown);
    let mut probed = r6.probed.lock().unwrap().clone();
    probed.sort();
    assert_eq!(probed, ["off", "ok", "old"]);
    // Without refresh nothing is probed.
    let r6b = Arc::new(FakeR6::default());
    w.bind_sign_in(r6b.clone()); // the first bind wins, so r6b stays unused
    w.list(false).unwrap();
    assert_eq!(r6.probed.lock().unwrap().len(), 3);
    assert!(r6b.probed.lock().unwrap().is_empty());
}

#[test]
fn refresh_runs_at_most_four_probes_at_once() {
    let s = Scratch::new();
    let names: Vec<String> = (0..9).map(|i| format!("p{i}")).collect();
    let w = store(&s, names.iter().map(|n| profile(n, true)).collect());
    let r6 = Arc::new(FakeR6 {
        delay: Duration::from_millis(40),
        ..FakeR6::default()
    });
    w.bind_sign_in(r6.clone());
    let l = w.list(true).unwrap();
    assert!(l
        .workspaces
        .iter()
        .all(|r| r.sign_in == WorkspaceSignIn::SignedIn));
    assert_eq!(r6.probed.lock().unwrap().len(), 9);
    let peak = r6.peak.load(Ordering::SeqCst);
    assert!((1..=PROBE_PARALLEL).contains(&peak), "peak {peak}");
}

#[test]
fn probes_slower_than_the_timeout_never_run_more_than_four_at_once_across_calls() {
    let s = Scratch::new();
    let names: Vec<String> = (0..10).map(|i| format!("p{i}")).collect();
    let w = store(&s, names.iter().map(|n| profile(n, true)).collect())
        .with_probe_timeout(Duration::from_millis(30));
    let r6 = Arc::new(FakeR6 {
        delay: Duration::from_millis(250),
        ..FakeR6::default()
    });
    w.bind_sign_in(r6.clone());
    let started = Instant::now();
    // Each call ends at its one deadline, and its late probes keep their slots.
    for _ in 0..3 {
        let l = w.list(true).unwrap();
        assert!(l
            .workspaces
            .iter()
            .all(|r| r.sign_in == WorkspaceSignIn::Unknown));
    }
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "{:?}",
        started.elapsed()
    );
    // Let the late probes return.
    std::thread::sleep(Duration::from_millis(700));
    let peak = r6.peak.load(Ordering::SeqCst);
    assert!((1..=PROBE_PARALLEL).contains(&peak), "peak {peak}");
}

#[test]
fn a_network_style_token_reply_error_is_not_expired() {
    let s = Scratch::new();
    let w = store(&s, vec![profile("p", true)]);
    w.bind_sign_in(Arc::new(FakeR6 {
        answers: HashMap::from([("p".to_owned(), Err("token_reply_invalid"))]),
        ..FakeR6::default()
    }));
    let l = w.list(true).unwrap();
    assert_eq!(row(&l, "p").sign_in, WorkspaceSignIn::Unknown);
}

#[test]
fn a_slow_probe_times_out_and_leaves_the_row_unknown() {
    let s = Scratch::new();
    let w = store(&s, vec![profile("slow", true)]).with_probe_timeout(Duration::from_millis(30));
    w.bind_sign_in(Arc::new(FakeR6 {
        delay: Duration::from_millis(400),
        ..FakeR6::default()
    }));
    let l = w.list(true).unwrap();
    assert_eq!(row(&l, "slow").sign_in, WorkspaceSignIn::Unknown);
}

// ---------- switches and home ----------

/// Write the file behind the store, as an old build or a test does, and drop the cache.
fn write_file(w: &WorkspaceStore, s: &Scratch, data: &Value) {
    fs::write(s.file(), serde_json::to_vec(data).unwrap()).unwrap();
    *w.cache.lock().unwrap() = None;
}

/// L5: the home and the model read the cache. Only a write of the store empties it.
#[test]
fn the_home_and_model_reads_use_the_cache_until_a_write_of_the_store() {
    let s = Scratch::new();
    let w = ready(&s);
    assert_eq!(w.home().as_deref(), Some("home"));
    let model = w.orchestrator_model().unwrap();
    // A read that hit the disk would fail on this file; the cache answers instead.
    fs::write(s.file(), b"{ not json").unwrap();
    assert_eq!(w.home().as_deref(), Some("home"));
    assert_eq!(w.orchestrator_model().as_deref(), Some(model.as_str()));
    // A write goes on the cached copy and saves it, so the new value shows at once.
    w.settings_write(&WorkspaceSettingsWriteArgs {
        cross_workspace_data: None,
        genie_call_budget: None,
        agent_endpoint: Some(ep("system.ai.gpt-oss-20b")),
        clean_dictation: None,
        cloud_dictation: None,
    })
    .unwrap();
    assert_eq!(
        w.orchestrator_model().as_deref(),
        Some("system.ai.gpt-oss-20b")
    );
}

fn ready(s: &Scratch) -> WorkspaceStore {
    // One old MOG on `home`: home is set. `other` is a new connection.
    put_config(&s.configs(), "a", "A", "home");
    store(
        s,
        vec![
            profile("home", true),
            profile("other", true),
            profile("pat", false),
        ],
    )
}

fn ep(s: &str) -> OrchestratorModel {
    OrchestratorModel::try_from(s.to_owned()).unwrap()
}

fn budget(n: u8) -> GenieCallBudget {
    GenieCallBudget::try_from(n).unwrap()
}

fn code(e: CoreError) -> &'static str {
    match e {
        CoreError::IpcRefused { code } => code,
        other => panic!("not an IPC refusal: {other:?}"),
    }
}

#[test]
fn ac_1_6_home_off_is_refused_and_the_disk_is_unchanged() {
    let s = Scratch::new();
    let w = ready(&s);
    w.list(false).unwrap();
    let before = fs::read(s.file()).unwrap();
    assert_eq!(
        code(w.set_available("home", false, None).unwrap_err()),
        "home_always_available"
    );
    assert_eq!(fs::read(s.file()).unwrap(), before);
}

#[test]
fn a_new_connection_needs_a_mog_id_bound_to_its_profile_to_switch_on() {
    let s = Scratch::new();
    let w = ready(&s);
    w.list(false).unwrap();
    put_config(&s.configs(), "n", "Other", "other");
    put_config(&s.configs(), "x", "Wrong", "elsewhere");
    let before = fs::read(s.file()).unwrap();
    // No mogId, a missing config and a config of another profile are all refused.
    for id in [None, Some("nope"), Some("x")] {
        assert_eq!(
            code(w.set_available("other", true, id).unwrap_err()),
            "invalid_args"
        );
    }
    // A PAT profile and an unknown profile are `profile_invalid`.
    for p in ["pat", "unknown"] {
        assert_eq!(
            code(w.set_available(p, true, Some("n")).unwrap_err()),
            "profile_invalid"
        );
    }
    assert_eq!(
        fs::read(s.file()).unwrap(),
        before,
        "a refused write changes nothing"
    );
    let l = w.set_available("other", true, Some("n")).unwrap();
    let r = row(&l, "other");
    assert!(r.available);
    assert_eq!(r.mog_ids, ["n"]);
    // Off again keeps the ids; on again needs no mogId.
    let l = w.set_available("other", false, None).unwrap();
    assert!(!row(&l, "other").available);
    let l = w.set_available("other", true, None).unwrap();
    assert!(row(&l, "other").available);
}

#[test]
fn make_home_needs_a_supported_profile_and_sets_it_available() {
    let s = Scratch::new();
    let w = ready(&s);
    w.list(false).unwrap();
    let before = fs::read(s.file()).unwrap();
    assert_eq!(
        code(w.make_home("pat", None).unwrap_err()),
        "profile_invalid"
    );
    assert_eq!(
        code(w.make_home("gone", None).unwrap_err()),
        "profile_invalid"
    );
    assert_eq!(fs::read(s.file()).unwrap(), before);
    let l = w.make_home("other", None).unwrap();
    assert_eq!(l.home.as_deref(), Some("other"));
    assert!(row(&l, "other").available && row(&l, "other").is_home);
    assert!(!row(&l, "home").is_home);
    assert_eq!(l.workspaces[0].profile, "other", "home first");
}

#[test]
fn the_first_connect_with_no_home_makes_the_new_profile_home_and_the_lamp() {
    let s = Scratch::new();
    let w = store(&s, vec![profile("first", true)]);
    assert_eq!(w.list(false).unwrap().home, None);
    put_config(&s.configs(), "n", "First", "first");
    let l = w.make_home("first", Some("n")).unwrap();
    assert_eq!(l.home.as_deref(), Some("first"));
    assert_eq!(l.lamp_mog_id.as_deref(), Some("n"));
    assert_eq!(row(&l, "first").mog_ids, ["n"]);
}

#[test]
fn settings_write_stores_d1_d2_and_the_endpoint_and_a_bad_value_changes_nothing() {
    let s = Scratch::new();
    let w = ready(&s);
    let l = w.list(false).unwrap();
    assert_eq!(
        l.agent,
        AgentSettings {
            cross_workspace_data: false,
            cross_workspace_locked: false,
            genie_call_budget: GenieCallBudget::DEFAULT,
            endpoint: ep("system.ai.claude-haiku-5-5"),
            clean_dictation: true,
            cloud_dictation: false,
        }
    );
    // A bad budget or model cannot be built, and the command refuses it on the wire.
    assert!(GenieCallBudget::try_from(7).is_err());
    assert!(OrchestratorModel::try_from("a b/c".to_owned()).is_err());
    let bad = w.call("workspace_settings_write", json!({"genieCallBudget": 7}));
    assert_eq!(code(bad.unwrap_err()), "invalid_args");
    for v in [
        json!("a b/c"),
        json!("databricks-claude-sonnet-5-5"),
        json!("system.ai."),
        json!("system.ai.a.b"),
        json!("other.ai.model"),
        json!(5),
    ] {
        let bad = w.call("workspace_settings_write", json!({"agentEndpoint": v}));
        assert_eq!(code(bad.unwrap_err()), "invalid_args");
    }
    let l = w
        .settings_write(&WorkspaceSettingsWriteArgs {
            cross_workspace_data: Some(true),
            genie_call_budget: Some(budget(20)),
            agent_endpoint: Some(ep("system.ai.gpt-oss-20b")),
            clean_dictation: Some(false),
            cloud_dictation: None,
        })
        .unwrap();
    assert_eq!(
        (
            l.agent.cross_workspace_data,
            l.agent.genie_call_budget.get(),
            l.agent.endpoint.as_str(),
            l.agent.clean_dictation
        ),
        (true, 20, "system.ai.gpt-oss-20b", false)
    );
    // A later partial write keeps the rest.
    let l = w
        .settings_write(&WorkspaceSettingsWriteArgs {
            cross_workspace_data: None,
            genie_call_budget: Some(budget(5)),
            agent_endpoint: None,
            clean_dictation: None,
            cloud_dictation: None,
        })
        .unwrap();
    assert!(l.agent.cross_workspace_data);
    assert_eq!(l.agent.endpoint.as_str(), "system.ai.gpt-oss-20b");
    assert!(!l.agent.clean_dictation);
}

/// ADR 0122: `cleanDictation` is a bool. Any other JSON type is refused.
#[test]
fn settings_write_refuses_a_non_bool_clean_dictation() {
    let s = Scratch::new();
    let w = ready(&s);
    for v in [json!("yes"), json!(1), json!(null), json!([])] {
        let bad = w.call("workspace_settings_write", json!({"cleanDictation": v}));
        assert_eq!(code(bad.unwrap_err()), "invalid_args");
    }
    let l = w
        .call("workspace_settings_write", json!({"cleanDictation": false}))
        .unwrap();
    assert_eq!(l["agent"]["cleanDictation"], false);
}

/// ADR 0123: `cloudDictation` is a bool, off by default, and the read-only reader follows it.
/// A file from before it has no field and reads as off.
#[test]
fn adr_0123_cloud_dictation_is_a_bool_off_by_default() {
    let s = Scratch::new();
    let w = ready(&s);
    assert!(!w.cloud_dictation());
    assert!(!w.cloud_dictation_fast());
    assert!(!w.list(false).unwrap().agent.cloud_dictation);
    for v in [json!("yes"), json!(1), json!(null), json!([])] {
        let bad = w.call("workspace_settings_write", json!({"cloudDictation": v}));
        assert_eq!(code(bad.unwrap_err()), "invalid_args");
    }
    let l = w
        .call("workspace_settings_write", json!({"cloudDictation": true}))
        .unwrap();
    assert_eq!(l["agent"]["cloudDictation"], true);
    assert!(w.cloud_dictation());
    // The fast atomic read follows the write with no extra load (ADR 0123).
    assert!(w.cloud_dictation_fast());
    // A later write of another field keeps it.
    w.call("workspace_settings_write", json!({"cleanDictation": false}))
        .unwrap();
    assert!(w.cloud_dictation());
    assert!(w.cloud_dictation_fast());
    // An old file with no field reads as off. The fast flag is stale until the next `load()`
    // (a cache-clearing disk write, same as `cloud_dictation()`'s own read-through).
    let mut data: Value = serde_json::from_slice(&fs::read(s.file()).unwrap()).unwrap();
    data["agent"]
        .as_object_mut()
        .unwrap()
        .remove("cloudDictation");
    write_file(&w, &s, &data);
    assert!(!w.cloud_dictation());
    assert!(!w.cloud_dictation_fast());
}

/// ADR 0122: a file from before the orchestrator reads on. The old D9 default, or any other
/// value that is not a `system.ai.` id, becomes the default model. A file with no
/// `cleanDictation` reads as true. A valid `system.ai.` id stays.
#[test]
fn an_old_stored_endpoint_migrates_to_the_default_model_on_read() {
    for old in [
        "databricks-claude-sonnet-5-5",
        "my-endpoint",
        "system.ai.",
        "x y",
        "",
    ] {
        let s = Scratch::new();
        let w = ready(&s);
        w.list(false).unwrap();
        let mut data: Value = serde_json::from_slice(&fs::read(s.file()).unwrap()).unwrap();
        data["agent"]["endpoint"] = json!(old);
        data["agent"]
            .as_object_mut()
            .unwrap()
            .remove("cleanDictation");
        write_file(&w, &s, &data);
        let l = w.list(false).unwrap();
        assert_eq!(
            l.agent.endpoint.as_str(),
            "system.ai.claude-haiku-5-5",
            "{old}"
        );
        assert!(l.agent.clean_dictation);
    }
    let s = Scratch::new();
    let w = ready(&s);
    w.list(false).unwrap();
    let mut data: Value = serde_json::from_slice(&fs::read(s.file()).unwrap()).unwrap();
    data["agent"]["endpoint"] = json!("system.ai.gpt-oss-20b");
    write_file(&w, &s, &data);
    assert_eq!(
        w.list(false).unwrap().agent.endpoint.as_str(),
        "system.ai.gpt-oss-20b"
    );
}

/// ADR 0122: an old file with the `runProfile` setting still loads. The command refuses the field,
/// and the next save drops it.
#[test]
fn an_old_file_with_run_profile_loads_and_the_next_save_drops_it() {
    let s = Scratch::new();
    let w = ready(&s);
    w.list(false).unwrap();
    let mut data: Value = serde_json::from_slice(&fs::read(s.file()).unwrap()).unwrap();
    data["agent"]["runProfile"] = json!("other");
    write_file(&w, &s, &data);
    let l = w.call("workspace_settings_write", json!({"genieCallBudget": 5}));
    assert!(l.unwrap()["agent"].get("runProfile").is_none());
    let saved: Value = serde_json::from_slice(&fs::read(s.file()).unwrap()).unwrap();
    assert!(saved["agent"].get("runProfile").is_none());
    let bad = w.call("workspace_settings_write", json!({"runProfile": "other"}));
    assert_eq!(code(bad.unwrap_err()), "invalid_args");
}

/// Slice 2a: the agent session snapshot.
#[test]
fn catalog_snapshot_gives_home_first_with_aliases_and_the_agent_profile_and_host() {
    let s = Scratch::new();
    let cfg = s.configs();
    for (id, name, home) in [
        ("a", "A", "Sales Prod"),
        ("b", "B", "sales-prod"),
        ("c", "C", "fin"),
    ] {
        put_config(&cfg, id, name, home);
    }
    let w = WorkspaceStore::new(
        &s.0,
        cfg,
        Arc::new(FakeT7(vec![
            profile("Sales Prod", true),
            profile("sales-prod", true),
            profile("fin", true),
            profile("pat", false),
        ])),
    );
    w.make_home("fin", None).unwrap();
    let snap = w.catalog_snapshot().unwrap();
    let got: Vec<(&str, &str, bool, bool)> = snap
        .catalog
        .iter()
        .map(|c| {
            (
                c.alias.as_str(),
                c.profile.as_str(),
                c.is_home,
                c.is_agent_workspace,
            )
        })
        .collect();
    // Home first, then by name; an unsupported profile is not in the catalog; two profiles that
    // slug to one alias get -2.
    assert_eq!(
        got,
        [
            ("fin", "fin", true, true),
            ("sales-prod", "Sales Prod", false, false),
            ("sales-prod-2", "sales-prod", false, false),
        ]
    );
    assert_eq!(snap.home_profile.as_deref(), Some("fin"));
    assert_eq!(snap.model, crate::model_gateway::DEFAULT_ORCHESTRATOR_MODEL);
    assert_eq!((snap.cross_workspace, snap.budget), (false, 10));
    // The model runs on the home profile, so the agent workspace is the home.
    assert_eq!(
        snap.catalog.iter().filter(|c| c.is_agent_workspace).count(),
        1
    );
    // A workspace switched off leaves the catalog.
    w.set_available("Sales Prod", false, None).unwrap();
    assert_eq!(w.catalog_snapshot().unwrap().catalog.len(), 2);
    // No home: no home profile and an empty catalog.
    let s2 = Scratch::new();
    let none = WorkspaceStore::new(
        &s2.0,
        s2.configs(),
        Arc::new(FakeT7(vec![profile("x", true)])),
    );
    let snap = none.catalog_snapshot().unwrap();
    assert!(snap.home_profile.is_none() && snap.catalog.is_empty());
}

/// I1: the whole alias, `-N` suffix included, stays within the 32 chars of the `workspace` arg.
#[test]
fn a_long_profile_name_with_a_collision_suffix_stays_within_32_chars() {
    let s = Scratch::new();
    let cfg = s.configs();
    let (long_a, long_b) = ("a".repeat(80), "A".repeat(80));
    for (id, name, home) in [("a", "A", &long_a), ("b", "B", &long_b)] {
        put_config(&cfg, id, name, home);
    }
    let w = WorkspaceStore::new(
        &s.0,
        cfg,
        Arc::new(FakeT7(vec![profile(&long_a, true), profile(&long_b, true)])),
    );
    let aliases: Vec<String> = w
        .catalog_snapshot()
        .unwrap()
        .catalog
        .iter()
        .map(|c| c.alias.clone())
        .collect();
    assert_eq!(aliases.len(), 2);
    assert_ne!(aliases[0], aliases[1]);
    assert!(aliases.iter().all(|a| a.len() <= 32), "{aliases:?}");
    assert!(aliases[1].ends_with("-2"), "{aliases:?}");
}

#[test]
fn alias_slugs_are_safe_short_and_never_empty() {
    assert_eq!(alias_slug("Sales (EU)"), "sales-eu");
    assert_eq!(alias_slug("--"), "ws");
    assert_eq!(alias_slug(&"a".repeat(80)).len(), 32);
    assert_eq!(alias_slug("ÄÖ"), "ws");
}

// ---------- the allow-list ----------

#[test]
fn the_gate_allows_home_and_available_only() {
    let s = Scratch::new();
    let w = ready(&s);
    assert!(w.allows("home"));
    assert!(!w.allows("other"), "a new connection is off");
    assert!(!w.allows("unknown"));
    put_config(&s.configs(), "n", "Other", "other");
    w.set_available("other", true, Some("n")).unwrap();
    assert!(w.allows("other"));
    w.set_available("other", false, None).unwrap();
    assert!(!w.allows("other"));
}

#[test]
fn with_no_home_the_gate_refuses_only_the_profiles_that_are_not_available() {
    let s = Scratch::new();
    let cfg = s.configs();
    put_config(&cfg, "a", "A", "p1");
    put_config(&cfg, "b", "B", "p2");
    let w = store(
        &s,
        vec![
            profile("p1", true),
            profile("p2", true),
            profile("p3", true),
        ],
    );
    assert_eq!(w.list(false).unwrap().home, None);
    assert!(w.allows("p1") && w.allows("p2"));
    assert!(!w.allows("p3"));
}

/// A fake R8 that counts every send.
#[derive(Default)]
struct CountingR8(AtomicUsize);

impl McpTransport for CountingR8 {
    fn call_tool(&self, _call: McpCall<'_>) -> CoreResult<McpReply> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(McpReply {
            structured_content: Value::Null,
            text: String::new(),
            is_error: false,
            http_status: 200,
            rpc_error: None,
        })
    }
    fn list_tools(&self, _profile: &str, _tags: &Tags) -> CoreResult<Vec<Value>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }
}

fn mcp_args(profile: &str, tool: &str, extra: Value) -> Value {
    let mut a = json!({
        "profile": profile,
        "server": "genie_one",
        "tool": tool,
        "args": {"question": "canary?"},
        "opId": "op-1",
        "tags": {"app": "bottle", "mog": "00000000-0000-4000-8000-0000000000b7", "op": "genie"}
    });
    for (k, v) in extra.as_object().unwrap() {
        a[k] = v.clone();
    }
    a
}

#[test]
fn ac_1_7_genie_ask_for_a_switched_off_profile_is_refused_before_any_send_on_every_path() {
    let s = Scratch::new();
    let w = Arc::new(ready(&s));
    let r8 = Arc::new(CountingR8::default());
    let comp = McpCallComponent::with_gate(r8.clone(), Some(w.clone()));
    let refusal = CoreError::IpcRefused {
        code: "workspace_not_available",
    };
    // Plain, with an agent-call marker, with a warehouse meta and with a timeout.
    for extra in [
        json!({}),
        json!({"meta": {"agentCall": true}}),
        json!({"meta": {"warehouse_id": "w1"}, "timeoutMs": 5000}),
    ] {
        let a = mcp_args("other", "genie_ask", extra);
        assert_eq!(comp.call("mcp_call", a).unwrap_err(), refusal);
    }
    assert_eq!(r8.0.load(Ordering::SeqCst), 0, "R8 sent nothing");
    // Home and a switched-on profile go through.
    assert!(comp
        .call("mcp_call", mcp_args("home", "genie_ask", json!({})))
        .is_ok());
    put_config(&s.configs(), "n", "Other", "other");
    w.set_available("other", true, Some("n")).unwrap();
    assert!(comp
        .call("mcp_call", mcp_args("other", "genie_ask", json!({})))
        .is_ok());
    assert_eq!(r8.0.load(Ordering::SeqCst), 2);
}

#[test]
fn polls_cancels_and_results_of_a_switched_off_profile_need_a_recorded_pair_and_listing_does_not() {
    let s = Scratch::new();
    let w = Arc::new(ready(&s));
    let r8 = Arc::new(CountingR8::default());
    let comp = McpCallComponent::with_gate(r8.clone(), Some(w));
    let calls = [
        (
            "genie_poll_response",
            json!({"conversation_id": "c-1", "response_id": "r-1"}),
        ),
        ("genie_cancel_response", json!({"conversation_id": "c-1"})),
        (
            "genie_get_query_result",
            json!({"conversation_id": "c-1", "response_id": "r-1", "item_id": "pub_1_output"}),
        ),
    ];
    // ADR 0117 follow-up: no recorded pair and a switched-off profile, so each is refused.
    for (tool, args) in &calls {
        let e = comp
            .call("mcp_call", mcp_args("other", tool, json!({"args": args})))
            .unwrap_err();
        assert_eq!(e.code(), "workspace_not_available", "{tool}");
    }
    assert_eq!(r8.0.load(Ordering::SeqCst), 0, "nothing was sent");
    // A pair that a successful ask recorded lets the call in flight end.
    comp.pairs().record("other", "c-1");
    for (tool, args) in &calls {
        assert!(
            comp.call("mcp_call", mcp_args("other", tool, json!({"args": args})))
                .is_ok(),
            "{tool}"
        );
    }
    assert!(comp
        .call("mcp_call", mcp_args("other", "tools/list", json!({})))
        .is_ok());
    assert_eq!(r8.0.load(Ordering::SeqCst), 4);
}

#[test]
fn a_component_without_a_gate_allows_every_profile() {
    let r8 = Arc::new(CountingR8::default());
    let comp = McpCallComponent::new(r8.clone());
    assert!(comp
        .call("mcp_call", mcp_args("any", "genie_ask", json!({})))
        .is_ok());
}

// ---------- the component ----------

#[test]
fn the_component_answers_a_workspace_list_and_refuses_other_commands() {
    let s = Scratch::new();
    let w = ready(&s);
    let v = w.call("workspace_list", json!({})).unwrap();
    assert_eq!(v["home"], "home");
    assert_eq!(v["agent"]["genieCallBudget"], 10);
    assert_eq!(v["workspaces"][0]["signIn"], "unknown");
    assert_eq!(
        w.call(
            "workspace_set_available",
            json!({"profile": "home", "available": false})
        )
        .unwrap_err(),
        CoreError::IpcRefused {
            code: "home_always_available"
        }
    );
    assert_eq!(
        w.call("workspace_list", json!({"refresh": "yes"}))
            .unwrap_err(),
        CoreError::IpcRefused {
            code: "invalid_args"
        }
    );
    assert_eq!(
        w.call("profile_login", json!({})).unwrap_err(),
        CoreError::IpcRefused {
            code: "not_implemented"
        }
    );
}
