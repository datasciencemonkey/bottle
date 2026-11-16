//! `SessionPolicy` validation (#259): the fixed values and the `hardCapMin` range of the
//! stop policy (section 7.1, 4.7). Deserialization and `validate` reject any other value
//! with a typed error and never panic.

use mog_core::types::{SessionPolicy, SessionPolicyError};
use serde_json::{json, Value};

fn spec() -> Value {
    json!({
        "checkInAtMin": 55, "softCapMin": 60, "extendMin": 30, "silenceNudgeMin": 5,
        "silenceStopMin": 15, "hardCapMin": 240, "lockStopMin": 10, "holdMaxSec": 120,
        "handsFreeMaxMin": 5
    })
}

fn with(field: &str, value: u64) -> Value {
    let mut v = spec();
    v[field] = json!(value);
    v
}

fn parse(v: Value) -> Result<SessionPolicy, serde_json::Error> {
    serde_json::from_value(v)
}

fn fixed_error(field: &str, value: u64) -> SessionPolicyError {
    let e = parse(with(field, value)).expect_err("a value off the spec must be rejected");
    assert!(e.to_string().contains(field), "message names {field}: {e}");
    // The typed error is reachable through `validate` on the same value.
    let v = u32::try_from(value).expect("test value fits u32");
    let ok: SessionPolicy = serde_json::from_value(spec()).unwrap();
    let p = match field {
        "checkInAtMin" => SessionPolicy {
            check_in_at_min: v,
            ..ok
        },
        "softCapMin" => SessionPolicy {
            soft_cap_min: v,
            ..ok
        },
        "extendMin" => SessionPolicy {
            extend_min: v,
            ..ok
        },
        "silenceNudgeMin" => SessionPolicy {
            silence_nudge_min: v,
            ..ok
        },
        "silenceStopMin" => SessionPolicy {
            silence_stop_min: v,
            ..ok
        },
        "lockStopMin" => SessionPolicy {
            lock_stop_min: v,
            ..ok
        },
        "holdMaxSec" => SessionPolicy {
            hold_max_sec: v,
            ..ok
        },
        "handsFreeMaxMin" => SessionPolicy {
            hands_free_max_min: v,
            ..ok
        },
        other => panic!("unknown field {other}"),
    };
    p.validate().expect_err("validate must reject the value")
}

#[test]
fn cap_74b_spec_policy_is_accepted() {
    let p = parse(spec()).expect("the spec values are valid");
    assert_eq!(p.validate(), Ok(()));
}

#[test]
fn cap_74b_valid_policy_serializes_to_the_exact_spec_json() {
    let p = parse(spec()).expect("the spec values are valid");
    assert_eq!(serde_json::to_value(&p).unwrap(), spec());
}

#[test]
fn cap_74b_hard_cap_range_60_to_480_is_accepted() {
    for hard in [60, 240, 480] {
        assert!(parse(with("hardCapMin", hard)).is_ok(), "hardCapMin {hard}");
    }
}

#[test]
fn cap_78a_hard_cap_outside_60_to_480_is_rejected() {
    for hard in [0, 59, 481, 500, u32::MAX as u64] {
        let e = parse(with("hardCapMin", hard)).expect_err("out of range");
        assert!(e.to_string().contains("hardCapMin"), "{e}");
    }
    let mut p: SessionPolicy = serde_json::from_value(spec()).unwrap();
    p.hard_cap_min = 500;
    assert_eq!(
        p.validate(),
        Err(SessionPolicyError::OutOfRange {
            field: "hardCapMin",
            min: 60,
            max: 480,
            actual: 500
        })
    );
}

#[test]
fn cap_76a_check_in_must_be_55() {
    assert_eq!(
        fixed_error("checkInAtMin", 54),
        SessionPolicyError::NotFixed {
            field: "checkInAtMin",
            expected: 55,
            actual: 54
        }
    );
}

#[test]
fn cap_76b_soft_cap_must_be_60_and_extend_must_be_30() {
    assert_eq!(
        fixed_error("softCapMin", 61),
        SessionPolicyError::NotFixed {
            field: "softCapMin",
            expected: 60,
            actual: 61
        }
    );
    assert_eq!(
        fixed_error("extendMin", 31),
        SessionPolicyError::NotFixed {
            field: "extendMin",
            expected: 30,
            actual: 31
        }
    );
}

#[test]
fn cap_77a_silence_nudge_must_be_5() {
    assert_eq!(
        fixed_error("silenceNudgeMin", 6),
        SessionPolicyError::NotFixed {
            field: "silenceNudgeMin",
            expected: 5,
            actual: 6
        }
    );
}

#[test]
fn cap_77b_silence_stop_must_be_15() {
    assert_eq!(
        fixed_error("silenceStopMin", 16),
        SessionPolicyError::NotFixed {
            field: "silenceStopMin",
            expected: 15,
            actual: 16
        }
    );
}

#[test]
fn cap_81c_lock_stop_must_be_10() {
    assert_eq!(
        fixed_error("lockStopMin", 11),
        SessionPolicyError::NotFixed {
            field: "lockStopMin",
            expected: 10,
            actual: 11
        }
    );
}

#[test]
fn cap_04a_hold_max_must_be_120_s() {
    assert_eq!(
        fixed_error("holdMaxSec", 121),
        SessionPolicyError::NotFixed {
            field: "holdMaxSec",
            expected: 120,
            actual: 121
        }
    );
}

#[test]
fn cap_04b_hands_free_max_must_be_5_min() {
    assert_eq!(
        fixed_error("handsFreeMaxMin", 6),
        SessionPolicyError::NotFixed {
            field: "handsFreeMaxMin",
            expected: 5,
            actual: 6
        }
    );
}

#[test]
fn cap_74b_soft_cap_is_not_above_hard_cap_for_every_accepted_policy() {
    for hard in [60, 61, 240, 480] {
        let p = parse(with("hardCapMin", hard)).unwrap();
        assert!(p.check_in_at_min < p.soft_cap_min && p.soft_cap_min <= p.hard_cap_min);
    }
}

#[test]
fn cap_74b_wrong_types_and_unknown_fields_are_errors_not_panics() {
    for bad in [
        json!(-1),
        json!(1.5),
        json!("60"),
        json!(null),
        json!(u64::MAX),
    ] {
        let mut v = spec();
        v["hardCapMin"] = bad;
        assert!(parse(v).is_err());
    }
    let mut v = spec();
    v["extra"] = json!(1);
    assert!(parse(v).is_err());
}
