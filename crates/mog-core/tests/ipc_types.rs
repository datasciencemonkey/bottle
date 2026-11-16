//! Round-trip test for the IPC-facing types in `mog_core::types` (#222).
//!
//! Every file in `tests/fixtures/ipc-types/<Type>[.<case>].json` must deserialize into its
//! Rust type and serialize back to equal JSON. `tests/ipc-types.test.ts` type-checks the same
//! files against the TS types in `src/shared/interfaces.ts`, so both sides share one shape.

use std::fs;
use std::path::{Path, PathBuf};

use mog_core::types::{
    CaptureMode, CaptureState, Intent, MogId, Mood, OthersTrack, Profile, ProfileInfo, ProfileName,
    RouterCommand, RouterDecision, SegmentRef, SessionPolicy, TagApp, TagOp, Tags, TokenState,
    TokenStatus, Track, Transcript, TranscriptProvider,
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
