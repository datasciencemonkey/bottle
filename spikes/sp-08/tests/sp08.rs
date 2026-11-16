//! SP-08 (#15) unit tests. They need no audio device.

use sp08::*;

fn probe(args: &[&str]) -> ProbeArgs {
    match parse_args(args.iter().copied()) {
        Ok(Command::Probe(p)) => p,
        other => panic!("expected a probe command, got {other:?}"),
    }
}

#[test]
fn sp_08_mode_names_round_trip() {
    for m in Mode::ALL {
        assert_eq!(parse_mode(m.name()), Ok(ModeArg::One(m)));
    }
    assert_eq!(Mode::BuildOnly.name(), "build-only");
    assert_eq!(Mode::BuildThenPlay.name(), "build-then-play");
    assert_eq!(Mode::BuildPauseThenPlay.name(), "build-pause-then-play");
    assert_eq!(Mode::Cold.name(), "cold");
    assert_eq!(parse_mode("all"), Ok(ModeArg::All));
}

#[test]
fn sp_08_mode_parser_rejects_unknown() {
    assert!(parse_mode("play").is_err());
    assert!(parse_mode("").is_err());
    assert!(parse_mode("Cold").is_err());
}

#[test]
fn sp_08_args_defaults() {
    assert_eq!(probe(&[]), ProbeArgs::default());
    let d = ProbeArgs::default();
    assert_eq!(d.mode, ModeArg::All);
    assert_eq!(d.runs, 5);
}

#[test]
fn sp_08_args_parse_all_flags() {
    let p = probe(&[
        "--mode",
        "build-then-play",
        "--runs",
        "20",
        "--hold",
        "10",
        "--wait",
        "2.5",
        "--gap",
        "0",
    ]);
    assert_eq!(p.mode, ModeArg::One(Mode::BuildThenPlay));
    assert_eq!(p.runs, 20);
    assert_eq!(p.hold_s, 10.0);
    assert_eq!(p.wait_s, 2.5);
    assert_eq!(p.gap_s, 0.0);
    assert_eq!(probe(&["--mode=cold"]).mode, ModeArg::One(Mode::Cold));
}

#[test]
fn sp_08_args_reject_bad_values() {
    for bad in [
        &["--runs", "0"][..],
        &["--runs", "x"],
        &["--runs"],
        &["--hold", "-1"],
        &["--wait", "nan"],
        &["--mode", "loud"],
        &["--frobnicate"],
    ] {
        assert!(
            parse_args(bad.iter().copied()).is_err(),
            "{bad:?} must be rejected"
        );
    }
}

#[test]
fn sp_08_args_subcommands() {
    assert_eq!(parse_args(["device-state"]), Ok(Command::DeviceState));
    assert_eq!(parse_args(["--help"]), Ok(Command::Help));
    assert_eq!(parse_args(["-h"]), Ok(Command::Help));
}

#[test]
fn sp_08_percentile_nearest_rank() {
    let v: Vec<f64> = (1..=20).rev().map(f64::from).collect();
    assert_eq!(percentile(&v, 50.0), Some(10.0));
    assert_eq!(percentile(&v, 95.0), Some(19.0));
    assert_eq!(percentile(&v, 100.0), Some(20.0));
    assert_eq!(percentile(&v, 0.0), Some(1.0));
    assert_eq!(percentile(&[7.0], 95.0), Some(7.0));
    // 5 samples: p95 rank = ceil(4.75) = 5, the max.
    assert_eq!(percentile(&[5.0, 1.0, 4.0, 2.0, 3.0], 95.0), Some(5.0));
    assert_eq!(percentile(&[5.0, 1.0, 4.0, 2.0, 3.0], 50.0), Some(3.0));
}

#[test]
fn sp_08_percentile_empty_is_none() {
    assert_eq!(percentile(&[], 50.0), None);
    assert_eq!(summarize(&[]), None);
}

#[test]
fn sp_08_summary() {
    let s = summarize(&[3.0, 1.0, 2.0, 10.0]).unwrap();
    assert_eq!(s.n, 4);
    assert_eq!(s.min, 1.0);
    assert_eq!(s.max, 10.0);
    assert_eq!(s.p50, 2.0);
    assert_eq!(s.p95, 10.0);
}

#[test]
fn sp_08_latency_from_marks() {
    assert_eq!(latency_ms(1_000_000, 13_500_000), Some(12.5));
    assert_eq!(latency_ms(1_000_000, 0), None, "no callback");
    assert_eq!(
        latency_ms(5_000_000, 4_000_000),
        None,
        "callback before key-down"
    );
}

#[test]
fn sp_08_fourcc_matches_coreaudio_constants() {
    assert_eq!(K_DEVICE_IS_RUNNING_SOMEWHERE, 0x676F_6E65);
    assert_eq!(K_DEVICE_IS_RUNNING, 0x676F_696E);
    assert_eq!(K_DEFAULT_INPUT_DEVICE, 0x6449_6E20);
    assert_eq!(K_SCOPE_GLOBAL, 0x676C_6F62);
}

#[test]
fn sp_08_evidence_verdict() {
    let pass = RunFlags {
        baseline: Some(false),
        before_play: Some(false),
        after_play: Some(true),
        callbacks_before_play: 0,
    };
    assert_eq!(evidence(pass), Evidence::IdleUntilPlay);
    assert_eq!(
        evidence(RunFlags {
            before_play: Some(true),
            ..pass
        }),
        Evidence::LiveBeforePlay
    );
    assert_eq!(
        evidence(RunFlags {
            callbacks_before_play: 3,
            ..pass
        }),
        Evidence::LiveBeforePlay
    );
    // build-only: no play, idle the whole time is still "idle until play".
    assert_eq!(
        evidence(RunFlags {
            after_play: None,
            ..pass
        }),
        Evidence::IdleUntilPlay
    );
    // A baseline that already runs (another app uses the mic) proves nothing.
    assert_eq!(
        evidence(RunFlags {
            baseline: Some(true),
            before_play: Some(true),
            ..pass
        }),
        Evidence::Inconclusive
    );
    // No flag off macOS and no callbacks: no evidence.
    assert_eq!(evidence(RunFlags::default()), Evidence::Inconclusive);
    // Played but the device never ran: the probe did not reach the device.
    assert_eq!(
        evidence(RunFlags {
            after_play: Some(false),
            ..pass
        }),
        Evidence::Inconclusive
    );
}
