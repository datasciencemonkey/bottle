//! SP-03 (#5) unit tests. They need no keyboard, permission or device.

use sp03::*;

fn ev(source: Source, kind: KeyKind, t_ms: u64, lat: Option<f64>) -> KeyEvent {
    KeyEvent {
        source,
        kind,
        t_ns: t_ms * 1_000_000,
        latency_ms: lat,
    }
}

fn tap(kind: KeyKind, t_ms: u64, lat: f64) -> KeyEvent {
    ev(Source::Tap, kind, t_ms, Some(lat))
}

#[test]
fn sp_03_nfr_08a_pairs_every_key_up() {
    let mut log = Vec::new();
    for i in 0..1000u64 {
        log.push(tap(KeyKind::Down, i * 400, 1.0));
        log.push(tap(KeyKind::Up, i * 400 + 150, 2.0));
    }
    let r = pair_events(&log, Source::Tap);
    assert_eq!(r.downs, 1000);
    assert_eq!(r.ups, 1000);
    assert_eq!(r.pairs, 1000);
    assert_eq!(r.missed_key_ups, 0);
    assert_eq!(r.orphan_key_ups, 0);
    assert_eq!(r.key_up_percent(), 100.0);
    let hold = r.hold.unwrap();
    assert_eq!((hold.min, hold.max), (150.0, 150.0));
}

#[test]
fn sp_03_nfr_08a_detects_missed_key_up() {
    use KeyKind::*;
    let log = [
        tap(Down, 0, 1.0),
        tap(Up, 100, 1.0),
        tap(Down, 400, 1.0),
        // key-up lost here
        tap(Down, 800, 1.0),
        tap(Up, 900, 1.0),
        tap(Up, 950, 1.0),    // orphan
        tap(Down, 1200, 1.0), // trailing down with no up
    ];
    let r = pair_events(&log, Source::Tap);
    assert_eq!(r.downs, 4);
    assert_eq!(r.ups, 3);
    assert_eq!(r.pairs, 2);
    assert_eq!(r.missed_key_ups, 2);
    assert_eq!(r.orphan_key_ups, 1);
    assert_eq!(r.key_up_percent(), 50.0);
}

#[test]
fn sp_03_pairing_filters_by_source() {
    use KeyKind::*;
    let log = [
        tap(Down, 0, 1.0),
        ev(Source::Carbon, Down, 10, None),
        tap(Up, 100, 1.0),
        ev(Source::Carbon, Up, 200, None),
    ];
    let t = pair_events(&log, Source::Tap);
    assert_eq!((t.pairs, t.missed_key_ups), (1, 0));
    let c = pair_events(&log, Source::Carbon);
    assert_eq!((c.pairs, c.missed_key_ups), (1, 0));
    assert_eq!(c.latency, None, "no latency on Carbon events here");
    assert_eq!(
        pair_events(&[], Source::Tap).key_up_percent(),
        100.0,
        "no key-down"
    );
}

#[test]
fn sp_03_nfr_08b_latency_percentiles() {
    let mut log = Vec::new();
    for i in 0..20u64 {
        log.push(tap(KeyKind::Down, i * 10, (i + 1) as f64));
    }
    let r = pair_events(&log, Source::Tap);
    let lat = r.latency.unwrap();
    assert_eq!(lat.n, 20);
    assert_eq!(lat.p50, 10.0);
    assert_eq!(lat.p95, 19.0);
    assert_eq!(lat.max, 20.0);
}

#[test]
fn sp_03_percentile_nearest_rank() {
    assert_eq!(percentile(&[], 50.0), None);
    assert_eq!(percentile(&[3.0], 95.0), Some(3.0));
    assert_eq!(percentile(&[5.0, 1.0, 4.0, 2.0, 3.0], 50.0), Some(3.0));
    assert_eq!(percentile(&[5.0, 1.0, 4.0, 2.0, 3.0], 95.0), Some(5.0));
    assert_eq!(percentile(&[5.0, 1.0, 4.0, 2.0, 3.0], 0.0), Some(1.0));
    assert_eq!(summarize(&[]), None);
}

#[test]
fn sp_03_ev_line_round_trip() {
    let e = tap(KeyKind::Up, 1234, 3.25);
    let line = format_ev_line(&e);
    assert!(line.starts_with("EV "), "{line}");
    assert_eq!(parse_ev_line(&line), Some(e));
    let c = ev(Source::Carbon, KeyKind::Down, 7, None);
    assert_eq!(parse_ev_line(&format_ev_line(&c)), Some(c));
    assert_eq!(parse_ev_line("[t=1.0 ms] src=tap kind=down"), None);
    assert_eq!(
        parse_ev_line("EV src=tap kind=sideways t_ns=1 lat_ms=none"),
        None
    );
    // A log line with a timestamp prefix still parses if it holds the EV part.
    assert_eq!(parse_ev_line(&format!("  {line}  ")), Some(e));
}

#[test]
fn sp_03_cap_35b_right_option_flag_transitions() {
    let down = NX_DEVICE_RIGHT_ALT | CG_FLAG_ALTERNATE | 0x100;
    assert_eq!(
        right_option_transition(KVK_RIGHT_OPTION, down),
        Some(KeyKind::Down)
    );
    assert_eq!(
        right_option_transition(KVK_RIGHT_OPTION, 0x100),
        Some(KeyKind::Up)
    );
    // Left Option still held, Right Option released: the right bit decides.
    assert_eq!(
        right_option_transition(KVK_RIGHT_OPTION, 0x20 | CG_FLAG_ALTERNATE),
        Some(KeyKind::Up)
    );
    // Left Option (kVK 58) is not the talk key.
    assert_eq!(right_option_transition(58, 0x20 | CG_FLAG_ALTERNATE), None);
}

#[test]
fn sp_03_ptt_talk_key_down_up() {
    let mut p = Ptt::new();
    assert_eq!(
        p.on_talk(KeyKind::Down, 1_000_000),
        PttOut::Started {
            by: Source::Tap,
            t_ns: 1_000_000
        }
    );
    assert!(p.is_active());
    assert_eq!(
        p.on_talk(KeyKind::Down, 2_000_000),
        PttOut::Ignored,
        "auto-repeat"
    );
    assert_eq!(
        p.on_talk(KeyKind::Up, 151_000_000),
        PttOut::Ended {
            reason: EndReason::KeyUp,
            t_ns: 151_000_000,
            held_ms: 150.0
        }
    );
    assert!(!p.is_active());
    assert_eq!(p.on_talk(KeyKind::Up, 160_000_000), PttOut::Ignored);
}

#[test]
fn sp_03_cap_39a_secure_input_ends_active_ptt() {
    let mut p = Ptt::new();
    p.on_talk(KeyKind::Down, 0);
    assert_eq!(
        p.on_secure_input(true, 40_000_000),
        PttOut::Ended {
            reason: EndReason::SecureInput,
            t_ns: 40_000_000,
            held_ms: 40.0
        }
    );
    assert!(p.secure_input());
    assert!(!p.is_active());
    // The talk-key up that may still come under SEI does not end or start anything.
    assert_eq!(p.on_talk(KeyKind::Up, 50_000_000), PttOut::Ignored);
    // SEI with no PTT active changes only the mode.
    let mut q = Ptt::new();
    assert_eq!(q.on_secure_input(true, 1), PttOut::Ignored);
    assert!(q.secure_input());
}

#[test]
fn sp_03_cap_39b_chord_drives_ptt_only_under_secure_input() {
    let mut p = Ptt::new();
    assert_eq!(
        p.on_chord(KeyKind::Down, 0),
        PttOut::Ignored,
        "no SEI: tap owns PTT"
    );
    p.on_secure_input(true, 0);
    assert_eq!(
        p.on_talk(KeyKind::Down, 1),
        PttOut::Ignored,
        "SEI: talk key paused"
    );
    assert_eq!(
        p.on_chord(KeyKind::Down, 10_000_000),
        PttOut::Started {
            by: Source::Carbon,
            t_ns: 10_000_000
        }
    );
    assert_eq!(
        p.on_chord(KeyKind::Up, 60_000_000),
        PttOut::Ended {
            reason: EndReason::ChordUp,
            t_ns: 60_000_000,
            held_ms: 50.0
        }
    );
    // SEI goes off: the talk key works again, the chord is ignored.
    assert_eq!(p.on_secure_input(false, 70_000_000), PttOut::Ignored);
    assert_eq!(p.on_chord(KeyKind::Down, 80_000_000), PttOut::Ignored);
    assert!(matches!(
        p.on_talk(KeyKind::Down, 90_000_000),
        PttOut::Started {
            by: Source::Tap,
            ..
        }
    ));
}

#[test]
fn sp_03_secure_input_pid_from_ioreg() {
    let text = r#"
    | |   "IOConsoleUsers" = ({"kCGSSessionOnConsoleKey"=Yes,"kCGSSessionSecureInputPID"=4242,"kCGSessionLoginDoneKey"=Yes})
    "#;
    assert_eq!(parse_secure_input_pid(text), Some(4242));
    assert_eq!(
        parse_secure_input_pid(r#""kCGSSessionOnConsoleKey"=Yes"#),
        None
    );
    assert_eq!(
        parse_secure_input_pid(r#""kCGSSessionSecureInputPID"=x"#),
        None
    );
}

#[test]
fn sp_03_clamshell_state_from_ioreg() {
    assert_eq!(
        parse_clamshell_ioreg(r#"  | "AppleClamshellState" = Yes"#),
        Some(true)
    );
    assert_eq!(
        parse_clamshell_ioreg(r#"  | "AppleClamshellState" = No"#),
        Some(false)
    );
    assert_eq!(
        parse_clamshell_ioreg(r#""AppleClamshellCausesSleep" = Yes"#),
        None
    );
}

#[test]
fn sp_03_cgevent_timestamp_domain() {
    // Apple silicon timebase 125/3: 24 MHz ticks.
    let (numer, denom) = (125, 3);
    let mach_now = 2_400_000_000u64; // 100 s of ticks
    let uptime_ns = 100_000_000_000u64; // 100 s
                                        // A timestamp in ticks, 1 ms (24,000 ticks) old.
    assert_eq!(
        timestamp_domain(mach_now - 24_000, mach_now, uptime_ns, numer, denom),
        TsDomain::MachTicks
    );
    // A timestamp in ns, 1 ms old.
    assert_eq!(
        timestamp_domain(uptime_ns - 1_000_000, mach_now, uptime_ns, numer, denom),
        TsDomain::Nanos
    );
    assert_eq!(
        ts_to_ns(24_000, TsDomain::MachTicks, numer, denom),
        1_000_000
    );
    assert_eq!(ts_to_ns(5, TsDomain::Nanos, numer, denom), 5);
}

#[test]
fn sp_03_carbon_constants() {
    assert_eq!(K_EVENT_CLASS_KEYBOARD, 0x6B65_7962);
    assert_eq!(fourcc(b"S03P"), 0x5330_3350);
    assert_eq!(KVK_SPACE, 49);
    assert_eq!(CARBON_CHORD_MODIFIERS, 0x1200);
    assert_eq!(KVK_RIGHT_OPTION, 61);
}

#[test]
fn sp_03_args() {
    assert_eq!(parse_args(["status"]), Ok(Command::Status));
    assert_eq!(parse_args(["--help"]), Ok(Command::Help));
    assert_eq!(parse_args::<[&str; 0], &str>([]), Ok(Command::Help));
    assert_eq!(
        parse_args(["listen"]),
        Ok(Command::Listen(ListenArgs::default()))
    );
    assert_eq!(
        parse_args([
            "listen",
            "--seconds",
            "30",
            "--request-access",
            "--no-carbon"
        ]),
        Ok(Command::Listen(ListenArgs {
            seconds: 30.0,
            request_access: true,
            carbon: false,
            ..ListenArgs::default()
        }))
    );
    assert_eq!(
        parse_args([
            "self-test",
            "--pairs",
            "50",
            "--hold-ms",
            "20",
            "--gap-ms",
            "40"
        ]),
        Ok(Command::SelfTest(SelfTestArgs {
            pairs: 50,
            hold_ms: 20,
            gap_ms: 40
        }))
    );
    assert_eq!(
        parse_args(["analyze", "run.log"]),
        Ok(Command::Analyze {
            path: "run.log".into()
        })
    );
    for bad in [
        &["frob"][..],
        &["listen", "--seconds", "-1"],
        &["listen", "--loud"],
        &["self-test", "--pairs", "0"],
        &["self-test", "--pairs"],
        &["analyze"],
    ] {
        assert!(parse_args(bad.iter().copied()).is_err(), "{bad:?}");
    }
}
