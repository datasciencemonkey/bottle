//! Round-trip test for the IPC-facing types in `mog_core::types` (#222).
//!
//! Every file in `tests/fixtures/ipc-types/<Type>[.<case>].json` must deserialize into its
//! Rust type and serialize back to equal JSON. `tests/ipc-types.test.ts` type-checks the same
//! files against the TS types in `src/shared/interfaces.ts`, so both sides share one shape.

use std::fs;
use std::path::{Path, PathBuf};

use mog_core::types::{
    AgentCallState, AgentSessionEndReason, AgentSessionEvent, AgentSessionState, AgentTurnAccepted,
    AgentTurnCall, AgentTurnEvent, AgentTurnResult, AgentTurnView, AppSetting, AppSettingKey,
    CaptureMode, CaptureState, CardUpdate, ConsentRecordKind, ConsentRecordView, GenieCodeStatus,
    Intent, MicHealth, MicLevelFrame, MogId, MogViewState, Mood, NetStatus, OthersTrack, Profile,
    ProfileInfo, ProfileName, RecordToggle, RecordingDetail, RecordingErrorKind, RecordingSettings,
    RecordingState, RecordingStatus, RecordingSummary, RouterCommand, RouterDecision, SegmentRef,
    SessionPolicy, StorageProbe, TagApp, TagOp, Tags, TalkKeyReason, TalkKeyState, TalkKeyStatus,
    TokenState, TokenStatus, Track, Transcript, TranscriptProvider, WorkspaceList,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/ipc-types")
}

fn fixtures() -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = fs::read_dir(fixture_dir())
        .expect("fixture dir exists")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            let text = fs::read_to_string(&p).expect("fixture reads");
            (name, serde_json::from_str(&text).expect("fixture is JSON"))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The type name is the part of the file stem before the first `.`.
fn type_of(stem: &str) -> &str {
    stem.split('.').next().unwrap()
}

fn round_trip<T: DeserializeOwned + Serialize>(json: &Value) -> Result<Value, String> {
    let typed: T = serde_json::from_value(json.clone()).map_err(|e| e.to_string())?;
    serde_json::to_value(&typed).map_err(|e| e.to_string())
}

/// Deserialize into the Rust type the file name points to and serialize again.
fn round_trip_by_name(type_name: &str, json: &Value) -> Result<Value, String> {
    match type_name {
        "MogId" => round_trip::<MogId>(json),
        "ProfileName" => round_trip::<ProfileName>(json),
        "Mood" => round_trip::<Vec<Mood>>(json),
        "Track" => round_trip::<Vec<Track>>(json),
        "CaptureMode" => round_trip::<Vec<CaptureMode>>(json),
        "Intent" => round_trip::<Vec<Intent>>(json),
        "TagApp" => round_trip::<Vec<TagApp>>(json),
        "TagOp" => round_trip::<Vec<TagOp>>(json),
        "OthersTrack" => round_trip::<Vec<OthersTrack>>(json),
        "TranscriptProvider" => round_trip::<Vec<TranscriptProvider>>(json),
        "TokenState" => round_trip::<Vec<TokenState>>(json),
        "RouterCommand" => round_trip::<Vec<RouterCommand>>(json),
        "Tags" => round_trip::<Tags>(json),
        "SessionPolicy" => round_trip::<SessionPolicy>(json),
        "CaptureState" => round_trip::<CaptureState>(json),
        "SegmentRef" => round_trip::<SegmentRef>(json),
        "Transcript" => round_trip::<Transcript>(json),
        "Profile" => round_trip::<Profile>(json),
        "ProfileInfo" => round_trip::<Vec<ProfileInfo>>(json),
        "TokenStatus" => round_trip::<TokenStatus>(json),
        "RouterDecision" => round_trip::<RouterDecision>(json),
        // Brain-to-view payloads (#285, ADR 0040).
        "MogViewState" => round_trip::<MogViewState>(json),
        "CardUpdate" => round_trip::<CardUpdate>(json),
        "MicLevelFrame" => round_trip::<MicLevelFrame>(json),
        // The `hotkey.status` payload (#297, #294, ADR 0039).
        "TalkKeyStatus" => round_trip::<TalkKeyStatus>(json),
        "TalkKeyState" => round_trip::<Vec<TalkKeyState>>(json),
        "TalkKeyReason" => round_trip::<Vec<TalkKeyReason>>(json),
        // The `net.status` payload (#466, ARC-F7, MEM-19d).
        "NetStatus" => round_trip::<NetStatus>(json),
        // The `consent_records_list` reply (#430, ADR 0060).
        "ConsentRecordView" => round_trip::<ConsentRecordView>(json),
        "ConsentRecordKind" => round_trip::<Vec<ConsentRecordKind>>(json),
        // The mic fault in `capture.state` and the app setting (#453, CAP-17b, CAP-22).
        "MicHealth" => round_trip::<Vec<MicHealth>>(json),
        "AppSettingKey" => round_trip::<Vec<AppSettingKey>>(json),
        "AppSetting" => round_trip::<AppSetting>(json),
        // Call recording (ADR 0103).
        "RecordingSettings" => round_trip::<RecordingSettings>(json),
        "RecordingState" => round_trip::<RecordingState>(json),
        "RecordingSummary" => round_trip::<RecordingSummary>(json),
        "RecordingDetail" => round_trip::<RecordingDetail>(json),
        "RecordingStatus" => round_trip::<Vec<RecordingStatus>>(json),
        "RecordingErrorKind" => round_trip::<Vec<RecordingErrorKind>>(json),
        "StorageProbe" => round_trip::<StorageProbe>(json),
        "RecordToggle" => round_trip::<RecordToggle>(json),
        // Slice 2a: the MOG agent on one live Genie Code session, and the workspace list (ADR 0117).
        "GenieCodeStatus" => round_trip::<GenieCodeStatus>(json),
        "AgentTurnAccepted" => round_trip::<AgentTurnAccepted>(json),
        "AgentTurnCall" => round_trip::<AgentTurnCall>(json),
        "AgentTurnResult" => round_trip::<AgentTurnResult>(json),
        "AgentSessionState" => round_trip::<AgentSessionState>(json),
        "AgentSessionEvent" => round_trip::<AgentSessionEvent>(json),
        "AgentTurnEvent" => round_trip::<AgentTurnEvent>(json),
        "AgentTurnView" => round_trip::<AgentTurnView>(json),
        "WorkspaceList" => round_trip::<WorkspaceList>(json),
        "AgentCallState" => round_trip::<Vec<AgentCallState>>(json),
        "AgentSessionEndReason" => round_trip::<Vec<AgentSessionEndReason>>(json),
        other => panic!("fixture for unknown type {other}: add it to round_trip_by_name"),
    }
}

/// The struct types; each one must reject a field it does not know.
const STRUCTS: &[&str] = &[
    "Tags",
    "SessionPolicy",
    "CaptureState",
    "SegmentRef",
    "Transcript",
    "Profile",
    "ProfileInfo",
    "TokenStatus",
    "RouterDecision",
    "MogViewState",
    "CardUpdate",
    "MicLevelFrame",
    "TalkKeyStatus",
    "NetStatus",
    "ConsentRecordView",
    "AppSetting",
    "RecordingSettings",
    "RecordingState",
    "RecordingSummary",
    "RecordingDetail",
    "StorageProbe",
    "RecordToggle",
    "GenieCodeStatus",
    "AgentTurnAccepted",
    "AgentTurnCall",
    "AgentTurnResult",
    "AgentSessionState",
    "AgentSessionEvent",
    "AgentTurnEvent",
    "AgentTurnView",
    "WorkspaceList",
];

#[test]
fn every_ipc_type_has_a_fixture() {
    let names: Vec<String> = fixtures()
        .iter()
        .map(|(n, _)| type_of(n).to_owned())
        .collect();
    for t in [
        "MogId",
        "ProfileName",
        "Mood",
        "Track",
        "CaptureMode",
        "Intent",
        "TagApp",
        "TagOp",
        "OthersTrack",
        "TranscriptProvider",
        "TokenState",
        "RouterCommand",
        "Tags",
        "SessionPolicy",
        "CaptureState",
        "SegmentRef",
        "Transcript",
        "Profile",
        "ProfileInfo",
        "TokenStatus",
        "RouterDecision",
        "MogViewState",
        "CardUpdate",
        "MicLevelFrame",
        "TalkKeyStatus",
        "TalkKeyState",
        "TalkKeyReason",
        "NetStatus",
        "ConsentRecordView",
        "ConsentRecordKind",
        "MicHealth",
        "AppSettingKey",
        "AppSetting",
        "RecordingSettings",
        "RecordingState",
        "RecordingSummary",
        "RecordingDetail",
        "RecordingStatus",
        "RecordingErrorKind",
        "StorageProbe",
        "RecordToggle",
        "GenieCodeStatus",
        "AgentTurnAccepted",
        "AgentTurnCall",
        "AgentTurnResult",
        "AgentSessionState",
        "AgentSessionEvent",
        "AgentTurnEvent",
        "AgentTurnView",
        "WorkspaceList",
        "AgentCallState",
        "AgentSessionEndReason",
    ] {
        assert!(names.iter().any(|n| n == t), "no fixture for {t}");
    }
}

#[test]
fn every_fixture_round_trips_to_equal_json() {
    for (name, json) in fixtures() {
        let back = round_trip_by_name(type_of(&name), &json)
            .unwrap_or_else(|e| panic!("{name}.json does not deserialize: {e}"));
        assert_eq!(back, json, "{name}.json changed in the round trip");
    }
}

#[test]
fn optional_fields_absent_stay_absent() {
    let minimal: Vec<_> = fixtures()
        .into_iter()
        .filter(|(n, _)| n.ends_with(".minimal"))
        .collect();
    assert!(
        minimal.len() >= 5,
        "expected the optional-field-absent cases"
    );
    for (name, json) in minimal {
        let back = round_trip_by_name(type_of(&name), &json).unwrap();
        let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(&back), keys(&json), "{name}: serialized a None field");
    }
}

fn with_extra_field(json: &Value, key: &str) -> Value {
    let mut v = json.clone();
    let obj = match &mut v {
        Value::Array(items) => items[0].as_object_mut().unwrap(),
        other => other.as_object_mut().unwrap(),
    };
    obj.insert(key.to_owned(), Value::String("x".into()));
    v
}

#[test]
fn ipc_structs_reject_an_unknown_field() {
    for (name, json) in fixtures() {
        let t = type_of(&name);
        if !STRUCTS.contains(&t) {
            continue;
        }
        let err = round_trip_by_name(t, &with_extra_field(&json, "surprise"))
            .expect_err(&format!("{name}: accepted an unknown field"));
        assert!(err.contains("unknown field"), "{name}: {err}");
    }
}

/// ARC-6: no Databricks token crosses IPC. No IPC struct has a token field, so a payload
/// that tries to carry one fails to deserialize instead of being passed along.
#[test]
fn arc_6_ipc_structs_reject_a_token_field() {
    for (name, json) in fixtures() {
        let t = type_of(&name);
        if !STRUCTS.contains(&t) {
            continue;
        }
        for key in ["token", "accessToken", "access_token", "authorization"] {
            assert!(
                round_trip_by_name(t, &with_extra_field(&json, key)).is_err(),
                "{name}: accepted a {key} field"
            );
        }
    }
}

#[test]
fn enums_reject_an_unknown_value() {
    assert!(serde_json::from_str::<Mood>("\"happy\"").is_err());
    assert!(serde_json::from_str::<Intent>("\"chat\"").is_err());
    assert!(serde_json::from_str::<CaptureMode>("\"handsFree\"").is_err());
    let mut ts: Value = serde_json::from_str(r#"{"profile":"p","state":"expired"}"#).unwrap();
    assert!(serde_json::from_value::<TokenStatus>(ts.take()).is_err());
}

#[test]
fn cap_02a_live_capture_state_carries_its_lease_id() {
    let live: CaptureState = serde_json::from_str(
        &std::fs::read_to_string(fixture_dir().join("CaptureState.json")).unwrap(),
    )
    .unwrap();
    assert!(live.recording);
    assert_eq!(live.lease_id.as_deref(), Some("17"));
    let back = serde_json::to_value(&live).unwrap();
    assert_eq!(back["leaseId"], "17");
}

/// CAP-17b (#453): a live capture state carries the mic health for the fix-it card.
#[test]
fn cap_17b_capture_state_carries_the_mic_fault_code() {
    let fault: CaptureState = serde_json::from_str(
        &std::fs::read_to_string(fixture_dir().join("CaptureState.micFault.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(fault.mic, Some(MicHealth::ZeroBuffers));
    let back = serde_json::to_value(&fault).unwrap();
    assert_eq!(back["mic"], "zero_buffers");
    for (health, code) in [
        (MicHealth::Ok, "ok"),
        (MicHealth::ZeroBuffers, "zero_buffers"),
        (MicHealth::PermissionDenied, "permission_denied"),
        (MicHealth::NoDevice, "no_device"),
    ] {
        assert_eq!(serde_json::to_value(health).unwrap(), code);
    }
}

/// CAP-17b (#453): `mic` is optional. A payload from before #453 still parses, and an
/// unset field is not written.
#[test]
fn cap_17b_a_capture_state_without_mic_still_parses() {
    let old: CaptureState =
        serde_json::from_str(r#"{"recording":false,"paused":false,"othersTrack":"off"}"#).unwrap();
    assert_eq!(old.mic, None);
    let back = serde_json::to_value(&old).unwrap();
    assert!(back.get("mic").is_none(), "an unset mic was serialized");
}

/// CAP-17b, CAP-01b (#453): the fault is a fixed code. It holds no device name or ID, so an
/// object or a free string is refused.
#[test]
fn cap_17b_cap_01b_the_mic_fault_is_a_fixed_code_without_device_identity() {
    for bad in [
        r#""AirPods Pro""#,
        r#""zero_buffers:MacBook Pro Microphone""#,
        r#"{"code":"zero_buffers","device":"AirPods Pro"}"#,
    ] {
        let json =
            format!(r#"{{"recording":false,"paused":false,"othersTrack":"off","mic":{bad}}}"#);
        assert!(
            serde_json::from_str::<CaptureState>(&json).is_err(),
            "accepted mic = {bad}"
        );
    }
}

/// CAP-22 (#453): the app setting has one key with a boolean value.
#[test]
fn cap_22_app_setting_prefer_built_in_mic_is_a_boolean() {
    let s: AppSetting = serde_json::from_str(
        &std::fs::read_to_string(fixture_dir().join("AppSetting.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        s,
        AppSetting {
            key: AppSettingKey::CapturePreferBuiltInMic,
            value: true
        }
    );
    assert_eq!(
        serde_json::to_value(AppSettingKey::CapturePreferBuiltInMic).unwrap(),
        "app.capture.preferBuiltInMic"
    );
    for bad in [
        r#"{"key":"app.capture.preferBuiltInMic","value":"yes"}"#,
        r#"{"key":"app.capture.preferBuiltInMic"}"#,
        r#"{"key":"app.capture.other","value":true}"#,
    ] {
        assert!(
            serde_json::from_str::<AppSetting>(bad).is_err(),
            "accepted {bad}"
        );
    }
}

#[test]
fn cap_02a_every_segment_names_its_lease() {
    use mog_core::types::SegmentRef;
    for f in ["SegmentRef.json", "SegmentRef.minimal.json"] {
        let seg: SegmentRef =
            serde_json::from_str(&std::fs::read_to_string(fixture_dir().join(f)).unwrap()).unwrap();
        assert!(!seg.lease_id.is_empty(), "{f}");
    }
    // A segment without a lease does not parse.
    let no_lease = serde_json::json!({"segmentId": "s", "mogId": "m", "track": "you", "tStartMs": 0, "tEndMs": 1, "final": true});
    assert!(serde_json::from_value::<SegmentRef>(no_lease).is_err());
}

#[test]
fn arc_5_ux_06b_a_closed_mic_frame_has_a_null_owner() {
    let closed: MicLevelFrame =
        serde_json::from_str(r#"{"owner":null,"dbfs":-120.5,"t":900.5}"#).unwrap();
    assert_eq!(closed.owner, None);
    // `owner` is required: a frame must say who owns the mic, or `null`.
    assert!(serde_json::from_str::<MicLevelFrame>(r#"{"dbfs":-1.5,"t":1.5}"#).is_err());
    assert_eq!(serde_json::to_value(&closed).unwrap()["owner"], Value::Null);
}

#[test]
fn cap_39c_cap_42_talk_key_status_reason_is_required_and_nullable() {
    let active: TalkKeyStatus =
        serde_json::from_str(r#"{"talkKey":"active","reason":null}"#).unwrap();
    assert_eq!(active.talk_key, TalkKeyState::Active);
    assert_eq!(active.reason, None);
    assert_eq!(
        serde_json::to_value(&active).unwrap()["reason"],
        Value::Null
    );
    // `reason` is required: a status with no `reason` key does not parse.
    assert!(serde_json::from_str::<TalkKeyStatus>(r#"{"talkKey":"active"}"#).is_err());
    let blocked: TalkKeyStatus =
        serde_json::from_str(r#"{"talkKey":"paused","reason":"elevated_foreground"}"#).unwrap();
    assert_eq!(blocked.reason, Some(TalkKeyReason::ElevatedForeground));
    // No other value, and no window title, process name or key field.
    assert!(serde_json::from_str::<TalkKeyStatus>(r#"{"talkKey":"off","reason":null}"#).is_err());
    assert!(
        serde_json::from_str::<TalkKeyStatus>(r#"{"talkKey":"paused","reason":"vpn"}"#).is_err()
    );
    for key in ["windowTitle", "processName", "key"] {
        let v = serde_json::json!({"talkKey": "paused", "reason": "secure_input", key: "x"});
        assert!(serde_json::from_value::<TalkKeyStatus>(v).is_err(), "{key}");
    }
}

#[test]
fn arc_f7_mem_19d_net_status_is_online_only() {
    let online: NetStatus = serde_json::from_str(r#"{"online":true}"#).unwrap();
    assert!(online.online);
    let offline: NetStatus = serde_json::from_str(r#"{"online":false}"#).unwrap();
    assert!(!offline.online);
    assert_eq!(
        serde_json::to_value(offline).unwrap(),
        serde_json::json!({"online": false})
    );
    // `online` is required and a bool: no empty payload, no null, no string.
    assert!(serde_json::from_str::<NetStatus>("{}").is_err());
    assert!(serde_json::from_str::<NetStatus>(r#"{"online":null}"#).is_err());
    assert!(serde_json::from_str::<NetStatus>(r#"{"online":"yes"}"#).is_err());
    // No host name, URL or error text.
    for key in ["host", "url", "error"] {
        let v = serde_json::json!({"online": false, key: "x"});
        assert!(serde_json::from_value::<NetStatus>(v).is_err(), "{key}");
    }
}

#[test]
fn sec_32a_consent_record_view_session_id_is_required_and_nullable_and_holds_no_user() {
    let notice =
        r#"{"recordId":"r1","kind":"notice","noticeVersion":1,"sessionId":null,"acceptedAt":1}"#;
    let v: ConsentRecordView = serde_json::from_str(notice).unwrap();
    assert_eq!(v.session_id, None);
    assert!(serde_json::from_str::<ConsentRecordView>(
        r#"{"recordId":"r1","kind":"notice","noticeVersion":1,"acceptedAt":1}"#
    )
    .is_err());
    for key in ["user", "device", "mogId"] {
        let mut v: serde_json::Value = serde_json::from_str(notice).unwrap();
        v[key] = serde_json::json!("x");
        assert!(
            serde_json::from_value::<ConsentRecordView>(v).is_err(),
            "{key}"
        );
    }
}
