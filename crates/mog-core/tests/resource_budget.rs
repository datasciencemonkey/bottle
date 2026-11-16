//! Memory and idle CPU budget harness (#173): the sample model, the budget of each NFR row
//! (NFR-04a, NFR-04b, NFR-05, NFR-06a, NFR-06b, NFR-09), the SP-02 / D73 window-plan rule, the
//! tool imports and the AT-21 report. Every test runs on test doubles, fixture files and the #229
//! simulated clock. The device test at the end skips with a reason unless a person sets the
//! device fixture (docs/tests/RESOURCE-BENCH.md).

use std::time::Duration;

use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::latency_bench::GateResult;
use mog_core::resource_budget::doubles::{demo_samples, ScriptedProbe};
use mog_core::resource_budget::import::{
    parse_csv, parse_powermetrics, parse_presentmon_csv, parse_ps, parse_sp04_bench,
    parse_tasklist_csv, parse_time_l, parse_typeperf_csv, to_csv, ProcessFilter, CSV_HEADER,
    CSV_MAGIC,
};
use mog_core::resource_budget::{
    collect_rss, evaluate, evaluate_all, sp02, window_plan, CpuRun, CpuScenario, Outcome,
    ParakeetRun, Report, RssRun, RunInfo, Samples, Source, Sp02Result, WindowPlan, IDS, MB,
    MIN_CPU_SAMPLES, MIN_RSS_SAMPLES,
};
use mog_core::testkit::{self, Fixture, ReferenceDevice};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/resource_budget"
);

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{FIXTURES}/{name}")).expect("fixture file")
}

fn rss(mogs: u8, peak_mb: u64) -> RssRun {
    let peak = peak_mb * MB;
    RssRun {
        mogs,
        bytes: vec![peak - MB, peak, peak - 2 * MB, peak - MB, peak - MB],
    }
}

fn cpu(scenario: CpuScenario, mogs: u8, mean: f64) -> CpuRun {
    CpuRun {
        scenario,
        mogs,
        percent: vec![mean; MIN_CPU_SAMPLES],
        presents_per_s: None,
    }
}

fn outcome(id: &str, s: &Samples) -> Outcome {
    evaluate(id, s).expect("known row").outcome
}

fn device_info() -> RunInfo {
    RunInfo {
        build: "abc1234".into(),
        hardware: "Apple M1 8 GB".into(),
        os: "macOS 14.4".into(),
        stt_model: "parakeet-tdt-0.6b-v3 int8".into(),
    }
}

fn device() -> Source {
    Source::Device {
        device: ReferenceDevice::MacM1_8Gb14_4,
        date_run: "2026-11-20".into(),
        run_by: "a person".into(),
    }
}

// ---- Budgets, one row at a time ------------------------------------------------------------

#[test]
fn at_21_budget_ids_match_the_nfr_rows() {
    assert_eq!(
        IDS,
        ["NFR-04a", "NFR-04b", "NFR-05", "NFR-06a", "NFR-06b", "NFR-09"]
    );
    let results = evaluate_all(&Samples::default());
    let limits: Vec<_> = results.iter().map(|r| r.limit.as_str()).collect();
    assert_eq!(
        limits,
        [
            "peak <= 250 MB",
            "(N=8 - N=1) / 7 <= 40 MB",
            "peak <= 1000 MB over >= 240 s",
            "mean <= 1 %",
            "mean <= 3 %",
            "hidden_panels - baseline < 0.2 %",
        ]
    );
    assert!(results.iter().all(|r| r.outcome == Outcome::NoData));
    assert!(evaluate("NFR-99", &Samples::default()).is_none());
}

#[test]
fn nfr_04a_shell_brain_and_one_mog_within_250_mb() {
    let mut s = Samples {
        rss: vec![rss(1, 250)],
        ..Samples::default()
    };
    assert_eq!(outcome("NFR-04a", &s), Outcome::Within);
    s.rss[0].bytes.push(250 * MB + 1);
    assert_eq!(outcome("NFR-04a", &s), Outcome::Over);
    s.rss[0].bytes = vec![100 * MB; MIN_RSS_SAMPLES - 1];
    assert_eq!(outcome("NFR-04a", &s), Outcome::TooFew);
    // Only N=1 counts: a run with 2 MOGs is not the NFR-04a run.
    let s = Samples {
        rss: vec![rss(2, 200)],
        ..Samples::default()
    };
    assert_eq!(outcome("NFR-04a", &s), Outcome::NoData);
}

#[test]
fn nfr_04b_each_extra_mog_within_40_mb_measured_with_8() {
    let s = Samples {
        rss: vec![rss(1, 180), rss(8, 180 + 7 * 40)],
        ..Samples::default()
    };
    let r = evaluate("NFR-04b", &s).unwrap();
    assert_eq!(r.outcome, Outcome::Within);
    assert_eq!(r.measured.as_deref(), Some("40.0 MB per extra MOG"));
    let s = Samples {
        rss: vec![rss(1, 180), rss(8, 180 + 7 * 40 + 1)],
        ..Samples::default()
    };
    assert_eq!(outcome("NFR-04b", &s), Outcome::Over);
    // The row is measured with 8 MOGs: N=2 and N=4 alone are no data.
    let s = Samples {
        rss: vec![rss(1, 180), rss(2, 200), rss(4, 260)],
        ..Samples::default()
    };
    let r = evaluate("NFR-04b", &s).unwrap();
    assert_eq!(r.outcome, Outcome::NoData);
    assert!(r.note.unwrap().contains("N=8"));
}

#[test]
fn nfr_05_parakeet_peak_within_1_gb_over_a_4_min_clip() {
    let mut s = Samples {
        parakeet: Some(ParakeetRun {
            clip_secs: 240.0,
            peak_bytes: 1_000 * MB,
        }),
        ..Samples::default()
    };
    assert_eq!(outcome("NFR-05", &s), Outcome::Within);
    s.parakeet.as_mut().unwrap().peak_bytes += 1;
    assert_eq!(outcome("NFR-05", &s), Outcome::Over);
    // A clip shorter than 4 min is not the NFR-05 run, even when the peak is low.
    s.parakeet = Some(ParakeetRun {
        clip_secs: 239.0,
        peak_bytes: 500 * MB,
    });
    let r = evaluate("NFR-05", &s).unwrap();
    assert_eq!(r.outcome, Outcome::Invalid);
    assert!(r.note.unwrap().contains("4 min"));
}

#[test]
fn nfr_05_sp04_m3_max_run_is_reported_as_a_fail() {
    // SP-04 (#44, PR #281) measured 1.91 GB on an M3 Max. The harness must show it as a fail.
    let run = parse_sp04_bench(&fixture("sp04-bench-m3max.txt")).expect("SP-04 bench output");
    assert_eq!(run.clip_secs, 240.0);
    assert_eq!(run.peak_bytes, 1_910_500_000);
    let s = Samples {
        parakeet: Some(run),
        ..Samples::default()
    };
    assert_eq!(outcome("NFR-05", &s), Outcome::Over);
    let info = RunInfo {
        build: "PR #281 spikes/sp-04".into(),
        hardware: "Apple M3 Max 64 GB".into(),
        os: "macOS".into(),
        stt_model: "parakeet-tdt-0.6b-v3 int8".into(),
    };
    let source = Source::OtherHardware {
        date_run: "2026-11-16".into(),
        run_by: "impl-44".into(),
    };
    let r = Report::build(source, info, &s);
    assert_eq!(r.result(), GateResult::Fail);
    let md = r.to_markdown();
    assert!(md.contains("| Result | fail |"), "{md}");
    assert!(md.contains("1910.5 MB"), "{md}");
    assert!(md.contains("not a reference device"), "{md}");
}

#[test]
fn nfr_05_time_l_gives_the_peak_rss_in_bytes() {
    assert_eq!(
        parse_time_l(&fixture("time-l-m3max.txt")),
        Some(1_910_472_704)
    );
    assert_eq!(parse_time_l("no such line"), None);
}

#[test]
fn nfr_06a_idle_cpu_within_1_percent_with_rive_paused() {
    let mut s = Samples {
        cpu: vec![cpu(CpuScenario::RivePaused, 8, 1.0)],
        ..Samples::default()
    };
    assert_eq!(outcome("NFR-06a", &s), Outcome::Within);
    s.cpu[0].percent = vec![1.01; MIN_CPU_SAMPLES];
    assert_eq!(outcome("NFR-06a", &s), Outcome::Over);
    s.cpu[0].percent = vec![0.1; MIN_CPU_SAMPLES - 1];
    assert_eq!(outcome("NFR-06a", &s), Outcome::TooFew);
    // PresentMon shows frames, so Rive is not paused: the run does not count.
    s.cpu[0].percent = vec![0.1; MIN_CPU_SAMPLES];
    s.cpu[0].presents_per_s = Some(30.0);
    let r = evaluate("NFR-06a", &s).unwrap();
    assert_eq!(r.outcome, Outcome::Invalid);
    assert!(r.note.unwrap().contains("not paused"));
}

#[test]
fn nfr_06b_idle_cpu_within_3_percent_with_8_animating_mogs() {
    let mut s = Samples {
        cpu: vec![cpu(CpuScenario::RiveAnimating, 8, 3.0)],
        ..Samples::default()
    };
    assert_eq!(outcome("NFR-06b", &s), Outcome::Within);
    s.cpu[0].percent = vec![3.1; MIN_CPU_SAMPLES];
    assert_eq!(outcome("NFR-06b", &s), Outcome::Over);
    // The row names 8 MOGs.
    let s4 = Samples {
        cpu: vec![cpu(CpuScenario::RiveAnimating, 4, 1.0)],
        ..Samples::default()
    };
    let r = evaluate("NFR-06b", &s4).unwrap();
    assert_eq!(r.outcome, Outcome::Invalid);
    assert!(r.note.unwrap().contains("8 MOGs"));
    // PresentMon shows no frames, so Rive does not animate.
    s.cpu[0].percent = vec![1.0; MIN_CPU_SAMPLES];
    s.cpu[0].presents_per_s = Some(0.0);
    let r = evaluate("NFR-06b", &s).unwrap();
    assert_eq!(r.outcome, Outcome::Invalid);
    assert!(r.note.unwrap().contains("not animating"));
}

#[test]
fn nfr_09_hidden_panels_and_hit_poll_cost_less_than_0_2_percent() {
    let s = Samples {
        cpu: vec![
            cpu(CpuScenario::Baseline, 0, 0.30),
            cpu(CpuScenario::HiddenPanels, 8, 0.49),
        ],
        ..Samples::default()
    };
    let r = evaluate("NFR-09", &s).unwrap();
    assert_eq!(r.outcome, Outcome::Within);
    assert_eq!(r.measured.as_deref(), Some("0.19 % over baseline"));
    // "Less than": 0.2 % exactly is over.
    let s = Samples {
        cpu: vec![
            cpu(CpuScenario::Baseline, 0, 0.30),
            cpu(CpuScenario::HiddenPanels, 8, 0.50),
        ],
        ..Samples::default()
    };
    assert_eq!(outcome("NFR-09", &s), Outcome::Over);
    // The cost is a difference, so the baseline run is needed.
    let s = Samples {
        cpu: vec![cpu(CpuScenario::HiddenPanels, 8, 0.1)],
        ..Samples::default()
    };
    assert_eq!(outcome("NFR-09", &s), Outcome::NoData);
    // Hidden panels with 0 MOGs measure nothing.
    let s = Samples {
        cpu: vec![
            cpu(CpuScenario::Baseline, 0, 0.30),
            cpu(CpuScenario::HiddenPanels, 0, 0.30),
        ],
        ..Samples::default()
    };
    assert_eq!(outcome("NFR-09", &s), Outcome::Invalid);
}

// ---- SP-02 and the D73 window plan ---------------------------------------------------------

#[test]
fn sp_02_passes_only_when_every_criterion_holds() {
    let s = demo_samples();
    assert_eq!(sp02(&s), Sp02Result::Pass);
    let mut no_steals = s.clone();
    no_steals.focus_steals = None;
    assert!(
        matches!(sp02(&no_steals), Sp02Result::NotRun(r) if r.iter().any(|m| m.contains("focus steals")))
    );
    let mut steals = s.clone();
    steals.focus_steals = Some(1);
    assert!(matches!(sp02(&steals), Sp02Result::Fail(_)));
    // 8 MOGs must stay at 550 MB or less without STT.
    let mut big = s.clone();
    big.rss = vec![rss(1, 250), rss(8, 551)];
    assert!(matches!(sp02(&big), Sp02Result::Fail(r) if r.iter().any(|m| m.contains("550 MB"))));
}

#[test]
fn d73_window_plan_is_cap_8_cap_4_or_one_overlay_per_display() {
    let s = demo_samples();
    assert_eq!(window_plan(&s).0, WindowPlan::Cap8);

    // SP-02 fails at 8 MOGs, but 4 MOGs fit: default cap 4, then the D73 overlay above 4.
    let mut cap4 = s.clone();
    cap4.rss = vec![rss(1, 200), rss(4, 200 + 3 * 40), rss(8, 200 + 7 * 50)];
    assert!(matches!(sp02(&cap4), Sp02Result::Fail(_)));
    assert_eq!(window_plan(&cap4).0, WindowPlan::Cap4ThenOverlay);

    // 4 MOGs do not fit either: one overlay window per display hosts all mascots.
    let mut overlay = cap4.clone();
    overlay.rss = vec![rss(1, 200), rss(4, 200 + 3 * 45), rss(8, 200 + 7 * 50)];
    assert_eq!(window_plan(&overlay).0, WindowPlan::OverlayPerDisplay);

    // No N=4 run: the cap cannot be checked, so the overlay applies.
    let mut no4 = cap4.clone();
    no4.rss.retain(|r| r.mogs != 4);
    assert_eq!(window_plan(&no4).0, WindowPlan::OverlayPerDisplay);

    // A cap does not stop focus steals.
    let mut steals = s.clone();
    steals.focus_steals = Some(2);
    assert_eq!(window_plan(&steals).0, WindowPlan::OverlayPerDisplay);

    // SP-02 has not run: no decision (TODO(#79)).
    let (plan, why) = window_plan(&Samples::default());
    assert_eq!(plan, WindowPlan::Undecided);
    assert!(why.contains("#79"), "{why}");
}

// ---- The AT-21 report ----------------------------------------------------------------------

#[test]
fn at_21_report_refuses_pass_when_samples_come_from_doubles() {
    let s = demo_samples();
    assert!(evaluate_all(&s)
        .iter()
        .all(|r| r.outcome == Outcome::Within));
    let r = Report::build(Source::Doubles, device_info(), &s);
    assert_eq!(r.result(), GateResult::NotRun);
    let md = r.to_markdown();
    assert!(md.contains("| Result | not run |"), "{md}");
    assert!(md.contains("test doubles"), "{md}");
}

#[test]
fn at_21_report_device_pass_needs_every_row_and_the_run_record() {
    let s = demo_samples();
    let r = Report::build(device(), device_info(), &s);
    assert_eq!(r.result(), GateResult::Pass, "{:?}", r.missing());
    assert!(r.missing().is_empty());

    for (field, info) in [
        (
            "build",
            RunInfo {
                build: " ".into(),
                ..device_info()
            },
        ),
        (
            "hardware",
            RunInfo {
                hardware: String::new(),
                ..device_info()
            },
        ),
        (
            "OS",
            RunInfo {
                os: String::new(),
                ..device_info()
            },
        ),
        (
            "STT model",
            RunInfo {
                stt_model: String::new(),
                ..device_info()
            },
        ),
    ] {
        let r = Report::build(device(), info, &s);
        assert_eq!(r.result(), GateResult::Partial, "{field}");
        assert!(
            r.missing().iter().any(|m| m.contains(field)),
            "{field}: {:?}",
            r.missing()
        );
    }

    let bad_date = Source::Device {
        device: ReferenceDevice::Win11I5_16Gb,
        date_run: "20 Nov".into(),
        run_by: String::new(),
    };
    let r = Report::build(bad_date, device_info(), &s);
    assert_eq!(r.result(), GateResult::Partial);
    assert!(r.missing().iter().any(|m| m.contains("date")));
    assert!(r.missing().iter().any(|m| m.contains("run by")));

    // A missing row is not a pass.
    let mut no_hidden = s.clone();
    no_hidden
        .cpu
        .retain(|c| c.scenario != CpuScenario::HiddenPanels);
    let r = Report::build(device(), device_info(), &no_hidden);
    assert_eq!(r.result(), GateResult::Partial);
    assert!(r.missing().iter().any(|m| m.starts_with("NFR-09")));

    // No samples at all.
    let r = Report::build(device(), device_info(), &Samples::default());
    assert_eq!(r.result(), GateResult::NotRun);

    // A row over its limit is a fail.
    let mut over = s.clone();
    over.rss = vec![rss(1, 300), rss(8, 400)];
    let r = Report::build(device(), device_info(), &over);
    assert_eq!(r.result(), GateResult::Fail);
}

#[test]
fn at_21_report_from_other_hardware_is_never_a_pass() {
    let source = Source::OtherHardware {
        date_run: "2026-11-20".into(),
        run_by: "a person".into(),
    };
    let r = Report::build(source, device_info(), &demo_samples());
    assert_eq!(r.result(), GateResult::Partial);
    assert!(r
        .missing()
        .iter()
        .any(|m| m.contains("not a reference device")));
}

#[test]
fn at_21_report_fills_the_evidence_template() {
    let md = Report::build(device(), device_info(), &demo_samples()).to_markdown();
    for needle in [
        "# Evidence: AT-21",
        "| Gate | AT-21 |",
        "| Gate type | device |",
        "| Result | pass |",
        "| Date run | 2026-11-20 |",
        "| Run by | a person |",
        "| Build | abc1234 |",
        "| Hardware | Apple M1 8 GB (mac-m1-8gb-14.4) |",
        "| Profile | none |",
        "| Requirements | NFR-04a, NFR-04b, NFR-05, NFR-06a, NFR-06b, NFR-09 |",
        "| Scope | part: NFR-04a, NFR-04b, NFR-05, NFR-06a, NFR-06b, NFR-09 only |",
        "## Method",
        "## Raw evidence",
        "## Result against the criterion",
        "## Follow-ups",
        "macOS 14.4",
        "parakeet-tdt-0.6b-v3 int8",
        "SP-02 from these samples: pass",
        "Window plan (D73): cap 8",
    ] {
        assert!(md.contains(needle), "missing {needle:?} in\n{md}");
    }
}

// ---- Sample files and tool imports ---------------------------------------------------------

#[test]
fn at_21_sample_csv_round_trips_and_rejects_bad_rows() {
    let s = demo_samples();
    let text = to_csv(&s);
    assert!(text.starts_with(&format!("{CSV_MAGIC}\n{CSV_HEADER}\n")));
    assert_eq!(parse_csv(&text).unwrap(), s);

    assert!(
        parse_csv("kind,scenario,mogs,value\n").is_err(),
        "no magic line"
    );
    let head = format!("{CSV_MAGIC}\n{CSV_HEADER}\n");
    for (bad, line) in [
        ("rss,shell,1,abc\n", 3),
        ("rss,shell,9,100\n", 3),
        ("cpu,dancing,8,1.0\n", 3),
        ("note,shell,1,1\n", 3),
        ("rss,shell,1,100,extra\n", 3),
        ("cpu,rive_paused,8,-1\n", 3),
        ("cpu,rive_paused,8,NaN\n", 3),
    ] {
        let e = parse_csv(&format!("{head}{bad}")).unwrap_err();
        assert_eq!(e.line, line, "{bad}: {e}");
    }
}

#[test]
fn sec_16b_samples_and_imports_hold_only_numbers() {
    // The sample file has no free-text column: a fifth column is rejected (above), and the
    // SP-04 import reads only numeric keys, never the transcript table.
    let run = parse_sp04_bench(&fixture("sp04-bench-m3max.txt")).unwrap();
    let s = Samples {
        parakeet: Some(run),
        ..Samples::default()
    };
    let csv = to_csv(&s);
    let md = Report::build(Source::Doubles, device_info(), &s).to_markdown();
    for out in [&csv, &md] {
        assert!(!out.contains("sales dashboard"), "{out}");
        assert!(!out.contains("Open the"), "{out}");
    }
    for line in csv.lines().skip(2) {
        let cols: Vec<_> = line.split(',').collect();
        assert_eq!(cols.len(), 4, "{line}");
        assert!(cols[3].parse::<f64>().is_ok(), "{line}");
    }
}

#[test]
fn at_21_ps_and_tasklist_snapshots_sum_the_rss_of_bottle_processes() {
    let mac = ProcessFilter::names(&[
        "Bottle",
        "com.apple.WebKit.WebContent",
        "com.apple.WebKit.GPU",
    ]);
    assert_eq!(
        parse_ps(&fixture("ps-snapshot.txt"), &mac),
        Ok((120_000 + 40_000 + 20_000) * 1024)
    );
    let win = ProcessFilter::names(&["Bottle.exe", "msedgewebview2"]);
    assert_eq!(
        parse_tasklist_csv(&fixture("tasklist.csv"), &win),
        Ok((120_000 + 40_000 + 20_000) * 1024)
    );
    // No Bottle process is an error, not 0 MB.
    let none = ProcessFilter::names(&["Nothing"]);
    assert!(parse_ps(&fixture("ps-snapshot.txt"), &none).is_err());
    assert!(parse_tasklist_csv(&fixture("tasklist.csv"), &none).is_err());
}

#[test]
fn nfr_06a_powermetrics_import_gives_percent_of_one_core() {
    let f = ProcessFilter::names(&["Bottle", "com.apple.WebKit.WebContent"]);
    let v = parse_powermetrics(&fixture("powermetrics-tasks.txt"), &f).unwrap();
    assert_eq!(v.len(), 2);
    assert!(
        (v[0] - 0.6).abs() < 1e-9 && (v[1] - 1.2).abs() < 1e-9,
        "{v:?}"
    );
    assert!(parse_powermetrics("nothing here", &f).is_err());
}

#[test]
fn nfr_06b_typeperf_import_sums_the_bottle_instances() {
    let f = ProcessFilter::names(&["Bottle", "msedgewebview2"]);
    let v = parse_typeperf_csv(&fixture("typeperf.csv"), &f).unwrap();
    // The third row has no value for Bottle, so it is dropped (not counted as 0).
    assert_eq!(v.len(), 2, "{v:?}");
    assert!(
        (v[0] - 0.8).abs() < 1e-9 && (v[1] - 1.6).abs() < 1e-9,
        "{v:?}"
    );
    let none = ProcessFilter::names(&["Nothing"]);
    assert!(parse_typeperf_csv(&fixture("typeperf.csv"), &none).is_err());
}

#[test]
fn nfr_06a_presentmon_import_gives_presents_per_second() {
    let f = ProcessFilter::names(&["Bottle.exe", "msedgewebview2.exe"]);
    let v = parse_presentmon_csv(&fixture("presentmon.csv"), &f, 2.0).unwrap();
    assert!((v - 1.5).abs() < 1e-9, "{v}");
    assert!(
        parse_presentmon_csv("a,b\n1,2\n", &f, 2.0).is_err(),
        "no Application column"
    );
    assert!(parse_presentmon_csv(&fixture("presentmon.csv"), &f, 0.0).is_err());
}

#[test]
fn nfr_04a_sampler_collects_rss_on_the_sim_clock() {
    let clock = SimClock::new();
    let mut probe = ScriptedProbe::new(vec![Ok(180 * MB), Ok(182 * MB), Ok(181 * MB)]);
    let c = clock.clone();
    let run = collect_rss(&mut probe, 1, 3, Duration::from_secs(2), &mut |d| {
        c.advance(d)
    })
    .unwrap();
    assert_eq!(run.mogs, 1);
    assert_eq!(run.bytes, vec![180 * MB, 182 * MB, 181 * MB]);
    // It waits between samples, not after the last one.
    assert_eq!(clock.now(), Duration::from_secs(4));
    // A probe error stops the run.
    let mut bad = ScriptedProbe::new(vec![Ok(1), Err("no Bottle process".into())]);
    assert!(collect_rss(&mut bad, 1, 2, Duration::ZERO, &mut |_| {}).is_err());
}

// ---- Device run (needs a person: docs/tests/RESOURCE-BENCH.md) -----------------------------

const ENV_SAMPLES: &str = "BOTTLE_RES_SAMPLES";
const ENV_SP04_BENCH: &str = "BOTTLE_RES_SP04_BENCH";

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

#[test]
fn at_21_device_resource_samples_meet_the_budgets() {
    // TODO(#173): a person records the samples on a reference device (needs:human).
    // TODO(#79): the SP-02 run gives the N = 0, 1, 2, 4, 8 rows and the focus steals.
    // TODO(#81): the 30 Hz hit-poll must be on for the hidden_panels run.
    let device = match testkit::device_from(env).expect("valid BOTTLE_TEST_DEVICE") {
        Fixture::Ready(d) => d,
        Fixture::Skip(reason) => {
            eprintln!("{reason}");
            return;
        }
    };
    let Some(path) = env(ENV_SAMPLES) else {
        eprintln!(
            "skipped: {ENV_SAMPLES} is not set, so the AT-21 resource benchmark does not run on \
             {} (see docs/tests/RESOURCE-BENCH.md)",
            device.id()
        );
        return;
    };
    let mut s = parse_csv(&std::fs::read_to_string(&path).expect("read the sample file"))
        .expect("valid sample file");
    if let Some(bench) = env(ENV_SP04_BENCH) {
        let text = std::fs::read_to_string(&bench).expect("read the SP-04 bench output");
        s.parakeet = Some(parse_sp04_bench(&text).expect("SP-04 bench output"));
    }
    for r in evaluate_all(&s) {
        assert_eq!(
            r.outcome,
            Outcome::Within,
            "{}: {:?} {:?}",
            r.id,
            r.measured,
            r.note
        );
    }
}
