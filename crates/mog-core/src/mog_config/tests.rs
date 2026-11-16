//! Tests of the MOG config store (#25). Names carry the requirement IDs (MB-02, AT-17).

use super::*;
use crate::ipc_gateway::{hardened_gateway_with, IpcGateway, NoLog};
use serde_json::json;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh directory under the OS temp dir, removed on drop. It holds only test files.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "bottle-mog-config-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&p);
        Self(p)
    }
    fn store(&self) -> MogConfigStore {
        MogConfigStore::new(self.0.join("mogs"))
    }
    fn files(&self) -> Vec<String> {
        match std::fs::read_dir(self.0.join("mogs")) {
            Ok(rd) => {
                let mut v: Vec<String> = rd
                    .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect();
                v.sort();
                v
            }
            Err(_) => Vec::new(),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn obj(v: Value) -> JsonObject {
    serde_json::from_value(v).expect("an object")
}

fn val(o: JsonObject) -> Value {
    serde_json::to_value(o).unwrap()
}

fn minimal() -> JsonObject {
    obj(json!({"schemaVersion": 1, "name": "MOG 1", "binding": {"home": "dev"}}))
}

fn with(path: &[&str], value: Value) -> JsonObject {
    let mut root = val(minimal());
    let mut cur = &mut root;
    for (i, k) in path.iter().enumerate() {
        let m = cur.as_object_mut().unwrap();
        if i + 1 == path.len() {
            m.insert((*k).to_owned(), value.clone());
            break;
        }
        cur = m.entry((*k).to_owned()).or_insert_with(|| json!({}));
    }
    obj(root)
}

/// Fake secrets, built at run time so no secret-shaped literal sits in the source.
fn fake_secrets() -> Vec<String> {
    vec![
        format!("dapi{}", "0123456789abcdef".repeat(2)),
        format!("eyJ{}.payload", "hbGciOiJIUzI1NiJ9"),
        format!("dose{}", "a1b2c3"),
        format!("AKIA{}", "ABCDEFGHIJ012345"),
        format!("ghp_{}", "x".repeat(36)),
        format!("-----BEGIN {} KEY-----", "PRIVATE"),
        format!("pass{}=hunter2", "word"),
    ]
}

// ---------- MB-02 / AT-17: secrets ----------

#[test]
fn mb_02_at_17_write_rejects_secret_named_keys() {
    let s = Scratch::new();
    let store = s.store();
    for key in [
        "token",
        "password",
        "secret",
        "pat",
        "accessToken",
        "db_password",
        "clientSecret",
        "PAT",
    ] {
        for path in [vec![key], vec!["binding", key], vec!["ui", key]] {
            let err = store.write("m1", &with(&path, json!("x"))).unwrap_err();
            assert_eq!(
                err,
                ConfigError::Secret(ConfigSecret::SecretNamedKey),
                "{path:?}"
            );
        }
    }
    // A secret-named key deep in a map value is caught too.
    let deep = with(&["binding", "profileAliases"], json!({"myToken": "dev"}));
    assert_eq!(
        store.write("m1", &deep).unwrap_err().code(),
        "secret_named_key"
    );
    assert!(s.files().is_empty());
}

#[test]
fn mb_02_at_17_write_rejects_token_like_values() {
    let s = Scratch::new();
    let store = s.store();
    for secret in fake_secrets() {
        for cfg in [
            with(&["name"], json!(secret)),
            with(&["persona"], json!(format!("use {secret} please"))),
            with(&["binding", "home"], json!(secret)),
            with(
                &["binding", "profileAliases"],
                json!({"prod": secret.clone()}),
            ),
            with(&["agent", "model"], json!(secret)),
        ] {
            let err = store.write("m1", &cfg).unwrap_err();
            assert_eq!(err, ConfigError::Secret(ConfigSecret::TokenLikeValue));
        }
    }
    // An unknown key with a token value is a secret refusal, not just a schema error.
    let unknown = with(&["extra"], json!([fake_secrets()[0].clone()]));
    assert_eq!(
        store.write("m1", &unknown).unwrap_err().code(),
        "token_like_value"
    );
    assert!(s.files().is_empty());
}

#[test]
fn mb_02_at_17_rejection_error_holds_no_secret_or_path() {
    let s = Scratch::new();
    let store = s.store();
    let secret = fake_secrets()[0].clone();
    let err = store
        .write("m1", &with(&["persona"], json!(secret)))
        .unwrap_err();
    let core: CoreError = err.into();
    let text = format!("{core} {core:?} {err} {err:?}");
    assert!(!text.contains(&secret));
    assert!(!text.contains("persona"));
    assert!(!text.contains(&*s.0.to_string_lossy()));
    assert_eq!(core.code(), "token_like_value");
}

#[test]
fn mb_02_at_17_gateway_config_write_rejects_secrets_and_reply_is_clean() {
    let s = Scratch::new();
    let gw = hardened_gateway_with(NoLog, MogConfigComponent::new(Arc::new(s.store())));
    let secret = fake_secrets()[3].clone();
    let args = json!({"id": "m1", "json": with(&["binding", "secret"], json!(secret))});
    let err = gw.dispatch("hub", "config_write", args).unwrap_err();
    assert_eq!(err.code(), "secret_named_key");
    assert!(!format!("{err:?}").contains(&secret));
    let args = json!({"id": "m1", "json": with(&["persona"], json!(secret))});
    assert_eq!(
        gw.dispatch("brain", "config_write", args)
            .unwrap_err()
            .code(),
        "token_like_value"
    );
    assert!(s.files().is_empty());
}

// ---------- MB-02: schema and bounds ----------

#[test]
fn mb_02_write_fills_appendix_f_defaults() {
    let s = Scratch::new();
    let saved = s.store().write("m1", &minimal()).unwrap();
    assert_eq!(saved.schema_version, SCHEMA_VERSION);
    assert_eq!(saved.ui, UiConfig::default());
    assert_eq!(saved.ui.size, MogSize::M);
    assert_eq!(saved.ui.doze_after_min, 20);
    assert_eq!(
        saved.binding.backend,
        Some(BackendConfig::GenieOneMcp {
            profile: "dev".to_owned(),
            warehouse_id: None
        })
    );
    assert!(saved.binding.genie_companion);
    assert_eq!(
        saved.capture.modes,
        vec![CaptureMode::Ptt, CaptureMode::HandsFree]
    );
    assert!(saved.capture.click_toggles_listening);
    assert_eq!(saved.capture.vad_end_ms, 1_500);
    assert!(saved.genie.auto_send_ptt);
    assert_eq!(saved.genie.preview_ms, 1_500);
    assert!(!saved.genie.share_context && !saved.genie.share_context_cross_workspace);
    assert_eq!(saved.agent.harness, Harness::Claude);
    assert!(saved.memory.enabled && !saved.memory.lakebase_sync);
    assert!(!saved.auto_start);
    assert_eq!(saved.card.auto_hide_sec, 60);
    assert_eq!(saved.card.max_blurbs, 3);
    assert_eq!(s.store().read("m1").unwrap(), saved);
}

#[test]
fn mb_02_default_config_matches_the_shared_fixture() {
    // The TS `MogConfig` type test reads the same file (src/brain/components/mog-config.test.ts).
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/mog-config-v1.json")).unwrap();
    let saved = parse_config(&minimal()).unwrap();
    assert_eq!(serde_json::to_value(&saved).unwrap(), fixture);
    assert_eq!(parse_config(&obj(fixture.clone())).unwrap(), saved);
}

#[test]
fn mb_02_write_round_trips_every_field() {
    let s = Scratch::new();
    let full = obj(json!({
        "schemaVersion": 1,
        "name": "Sales MOG",
        "ui": {"variant": "teal", "size": "L", "dozeAfterMin": 0},
        "binding": {
            "home": "prod",
            "backend": {"kind": "genie_agent", "profile": "prod", "spaceId": "01ef", "mode": "agent_mode"},
            "genieCompanion": false,
            "profileAliases": {"finance": "fin-ws"}
        },
        "capture": {"modes": ["ptt", "session"], "clickTogglesListening": false, "vadEndMs": 0},
        "hotkeys": {"chord": 3},
        "genie": {"autoSendPtt": false, "previewMs": 2000, "shareContext": true, "shareContextCrossWorkspace": false},
        "agent": {"harness": "codex", "model": "databricks-claude"},
        "pages": {"domainId": "dom-1"},
        "memory": {"enabled": false, "lakebaseSync": false, "transcriptSync": false},
        "persona": "Answer briefly.",
        "stt": {"provider": "local_parakeet"},
        "autoStart": true,
        "card": {"autoHideSec": 30, "maxBlurbs": 5}
    }));
    let saved = s.store().write("m-2", &full).unwrap();
    assert_eq!(serde_json::to_value(&saved).unwrap(), val(full));
    assert_eq!(s.store().read("m-2").unwrap(), saved);
}

#[test]
fn mb_02_write_rejects_unknown_fields_and_bad_enums() {
    let s = Scratch::new();
    let store = s.store();
    let bad = [
        with(&["colour"], json!("red")),
        with(&["ui", "theme"], json!("dark")),
        with(&["binding", "apiKey"], json!("abc")),
        with(
            &["binding", "backend"],
            json!({"kind": "genie_mcp", "profile": "dev"}),
        ),
        with(
            &["binding", "backend"],
            json!({"kind": "genie_one_mcp", "profile": "dev", "url": "https://x"}),
        ),
        with(&["capture", "modes"], json!(["ptt", "always"])),
        with(&["agent", "harness"], json!("gpt")),
        with(&["stt", "provider"], json!("cloud")),
        with(&["ui", "size"], json!("XL")),
        with(&["autoStart"], json!("yes")),
        obj(json!({"schemaVersion": 1, "binding": {"home": "dev"}})),
        obj(json!({"schemaVersion": 1, "name": "n"})),
    ];
    for cfg in bad {
        assert_eq!(
            store.write("m1", &cfg).unwrap_err(),
            ConfigError::InvalidConfig,
            "{cfg:?}"
        );
    }
    assert_eq!(
        store
            .write("m1", &with(&["schemaVersion"], json!(2)))
            .unwrap_err(),
        ConfigError::UnsupportedSchemaVersion
    );
    assert_eq!(
        store
            .write("m1", &obj(json!({"name": "n", "binding": {"home": "dev"}})))
            .unwrap_err(),
        ConfigError::InvalidConfig
    );
    assert!(s.files().is_empty());
}

#[test]
fn mb_02_write_rejects_oversized_and_out_of_range_values() {
    let s = Scratch::new();
    let store = s.store();
    let too_large = with(&["persona"], json!("a".repeat(MAX_CONFIG_BYTES)));
    assert_eq!(
        store.write("m1", &too_large).unwrap_err(),
        ConfigError::TooLarge
    );
    let ok_persona = with(&["persona"], json!("é".repeat(MAX_PERSONA_CHARS)));
    assert!(store.write("m1", &ok_persona).is_ok());
    let bad = [
        with(&["persona"], json!("a".repeat(MAX_PERSONA_CHARS + 1))),
        with(&["name"], json!("")),
        with(&["name"], json!("a".repeat(65))),
        with(&["name"], json!("line\nbreak")),
        with(&["binding", "home"], json!("")),
        with(&["binding", "home"], json!("p".repeat(257))),
        with(&["ui", "variant"], json!("v".repeat(33))),
        with(&["ui", "dozeAfterMin"], json!(1_441)),
        with(&["capture", "vadEndMs"], json!(60_001)),
        with(&["capture", "modes"], json!(["ptt", "ptt"])),
        with(&["genie", "previewMs"], json!(60_001)),
        with(&["hotkeys", "chord"], json!(10)),
        with(&["card", "maxBlurbs"], json!(0)),
        with(&["card", "maxBlurbs"], json!(11)),
        with(&["card", "autoHideSec"], json!(3_601)),
        with(&["agent", "model"], json!("")),
        with(
            &["binding", "profileAliases"],
            Value::Object((0..33).map(|i| (format!("w{i}"), json!("dev"))).collect()),
        ),
        with(&["binding", "profileAliases"], json!({"": "dev"})),
        with(
            &["binding", "backend"],
            json!({"kind": "custom_agent", "profile": "dev", "appUrl": "http://x.databricksapps.com", "protocol": "responses"}),
        ),
    ];
    for cfg in bad {
        assert_eq!(
            store.write("m2", &cfg).unwrap_err(),
            ConfigError::InvalidConfig,
            "{cfg:?}"
        );
    }
    assert_eq!(s.files(), ["m1.json"]);
}

#[test]
fn mb_02_ids_must_be_safe_file_names() {
    let s = Scratch::new();
    let store = s.store();
    for id in [
        "",
        "../x",
        "a/b",
        "a\\b",
        ".hidden",
        "a.json",
        "a b",
        "ü",
        &"a".repeat(MAX_ID_LEN + 1),
    ] {
        assert!(!valid_id(id), "{id:?}");
        assert_eq!(
            store.write(id, &minimal()).unwrap_err(),
            ConfigError::InvalidId
        );
        assert_eq!(store.read(id).unwrap_err(), ConfigError::InvalidId);
        assert_eq!(store.delete(id).unwrap_err(), ConfigError::InvalidId);
    }
    assert!(valid_id("8f14e45f-ceea-467f-a0e6-2b4c1d0a9e11"));
    assert!(valid_id(&"a".repeat(MAX_ID_LEN)));
    assert!(s.files().is_empty());
    assert!(!s.0.join("x.json").exists());
}

// ---------- MB-02: storage ----------

#[test]
fn mb_02_write_replaces_atomically_and_leaves_no_temp_file() {
    let s = Scratch::new();
    let store = s.store();
    store.write("m1", &minimal()).unwrap();
    store
        .write("m1", &with(&["name"], json!("Renamed")))
        .unwrap();
    assert_eq!(s.files(), ["m1.json"]);
    assert_eq!(store.read("m1").unwrap().name, "Renamed");
    // A rejected write keeps the old file as it was.
    let before = std::fs::read(s.0.join("mogs/m1.json")).unwrap();
    assert!(store
        .write("m1", &with(&["persona"], json!(fake_secrets()[1].clone())))
        .is_err());
    assert_eq!(std::fs::read(s.0.join("mogs/m1.json")).unwrap(), before);
    assert_eq!(s.files(), ["m1.json"]);
}

#[cfg(unix)]
#[test]
fn mb_02_files_are_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let s = Scratch::new();
    s.store().write("m1", &minimal()).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&s.0.join("mogs")), 0o700);
    assert_eq!(mode(&s.0.join("mogs/m1.json")), 0o600);
}

#[test]
fn mb_02_list_read_and_delete() {
    let s = Scratch::new();
    let store = s.store();
    assert_eq!(store.list().unwrap(), ConfigList::default());
    assert_eq!(store.read("m1").unwrap_err(), ConfigError::NotFound);
    store.write("b", &with(&["name"], json!("Bee"))).unwrap();
    store.write("a", &with(&["name"], json!("Ay"))).unwrap();
    // Files that are not configs are ignored.
    std::fs::write(s.0.join("mogs/notes.txt"), "x").unwrap();
    std::fs::write(s.0.join("mogs/.a.json.tmp-1"), "x").unwrap();
    let list = store.list().unwrap();
    assert_eq!(
        list.items,
        [
            ConfigSummary {
                id: "a".into(),
                name: "Ay".into()
            },
            ConfigSummary {
                id: "b".into(),
                name: "Bee".into()
            }
        ]
    );
    assert!(list.unreadable.is_empty());
    assert!(store.delete("a").unwrap());
    assert!(!store.delete("a").unwrap());
    assert_eq!(store.read("a").unwrap_err(), ConfigError::NotFound);
    assert_eq!(store.list().unwrap().items.len(), 1);
}

#[test]
fn mb_02_at_17_read_refuses_a_tampered_file_and_keeps_it() {
    let s = Scratch::new();
    let store = s.store();
    store.write("ok", &minimal()).unwrap();
    let dir = s.0.join("mogs");
    let mut tampered = val(minimal());
    tampered["persona"] = json!(fake_secrets()[0].clone());
    std::fs::write(dir.join("leak.json"), tampered.to_string()).unwrap();
    std::fs::write(dir.join("junk.json"), "{not json").unwrap();
    std::fs::write(dir.join("big.json"), " ".repeat(MAX_CONFIG_BYTES + 1)).unwrap();
    assert_eq!(
        store.read("leak").unwrap_err(),
        ConfigError::Secret(ConfigSecret::TokenLikeValue)
    );
    assert_eq!(store.read("junk").unwrap_err(), ConfigError::InvalidConfig);
    assert_eq!(store.read("big").unwrap_err(), ConfigError::TooLarge);
    let list = store.list().unwrap();
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.unreadable, ["big", "junk", "leak"]);
    // No automatic deletion of user content.
    assert_eq!(s.files(), ["big.json", "junk.json", "leak.json", "ok.json"]);
}

#[test]
fn mb_02_store_caps_the_number_of_configs() {
    let s = Scratch::new();
    let store = s.store();
    for i in 0..MAX_CONFIGS {
        store.write(&format!("m{i}"), &minimal()).unwrap();
    }
    assert_eq!(
        store.write("one-more", &minimal()).unwrap_err(),
        ConfigError::LimitReached
    );
    // Replacing an existing config is still allowed.
    store.write("m0", &minimal()).unwrap();
}

// ---------- MB-02: gateway seam ----------

#[test]
fn mb_02_gateway_serves_config_commands() {
    let s = Scratch::new();
    let gw = hardened_gateway_with(NoLog, MogConfigComponent::new(Arc::new(s.store())));
    let reply = gw
        .dispatch(
            "hub",
            "config_write",
            json!({"id": "m1", "json": minimal()}),
        )
        .unwrap();
    assert_eq!(reply["id"], "m1");
    assert_eq!(reply["json"]["capture"]["vadEndMs"], 1_500);
    let reply = gw
        .dispatch("brain", "config_read", json!({"id": "m1"}))
        .unwrap();
    assert_eq!(reply["json"]["name"], "MOG 1");
    let reply = gw.dispatch("brain", "config_list", json!({})).unwrap();
    assert_eq!(
        reply,
        json!({"items": [{"id": "m1", "name": "MOG 1"}], "unreadable": []})
    );
    assert_eq!(
        gw.dispatch("hub", "config_read", json!({"id": "nope"}))
            .unwrap_err()
            .code(),
        "config_not_found"
    );
    let reply = gw
        .dispatch("hub", "config_delete", json!({"id": "m1"}))
        .unwrap();
    assert_eq!(reply, json!({"deleted": true}));
    // View windows may not call config_* (4.5: brain, hub).
    assert_eq!(
        gw.dispatch("mog-m1", "config_list", json!({}))
            .unwrap_err()
            .code(),
        "window_not_allowed"
    );
    // Other commands are not this component's.
    assert_eq!(
        gw.dispatch("brain", "cli_status", json!({}))
            .unwrap_err()
            .code(),
        "not_implemented"
    );
}
