//! Latency benchmark harness (#171): the mark API, the stats and budgets per NFR row, and the
//! AT-21 / AT-30 / AT-43 report. Every test here runs on the #229 simulated clock and test doubles.
//! The device tests at the end skip with a reason unless a person sets the device fixture
//! (docs/tests/LATENCY-BENCH.md).

use std::sync::Arc;
use std::time::Duration;

use mog_core::clock::SimClock;
use mog_core::latency_bench::doubles::{ScriptedPipeline, ScriptedTiming};
use mog_core::latency_bench::import::{parse_csv, parse_sp03_log, to_csv, CSV_HEADER, CSV_MAGIC};
use mog_core::latency_bench::{
    budget, budgets, evaluate, percentile, BudgetConfig, Gate, GateResult, HotkeySamples,
    LatencyRecorder, Mark, MarkError, Outcome, Report, ResourceUse, Route, RunInfo, SampleSource,
    SttPath, Summary, Trial,
};
use mog_core::testkit::{self, Fixture, ReferenceDevice};

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

/// A timing that meets every budget of section 8.1.
fn good() -> ScriptedTiming {
    ScriptedTiming {
        mic_live: ms(40),
        hold: ms(1_200),
        ack: ms(30),
        chip: Some(ms(900)),
        insert: Some(ms(950)),
    }
}

fn presses(p: &ScriptedPipeline, n: usize, path: SttPath, route: Route, t: ScriptedTiming) {
    for _ in 0..n {
        p.press(path, route, t).expect("valid press");
    }
}

fn outcome(id: &str, trials: &[Trial], hk: &HotkeySamples, cfg: BudgetConfig) -> Outcome {
    let b = budget(id, cfg).expect("known budget");
    evaluate(&b, trials, hk).outcome
}

fn full_hotkey(cycles: u32, seen: u32, latency: Duration) -> HotkeySamples {
    let mut hk = HotkeySamples::default();
    for i in 0..cycles {
        hk.record_cycle(i < seen, &[latency, latency]);
    }
    hk
}

fn device_info() -> RunInfo {
    RunInfo {
        build: "abc1234".into(),
        hardware: "Apple M1 8 GB".into(),
        os: "macOS 14.4".into(),
        models: vec!["parakeet-tdt-0.6b-v3 int8".into()],
        resources: ResourceUse {
            peak_rss_mb: Some(900),
            cpu_percent: Some(35),
        },
        config: BudgetConfig::default(),
    }
}

fn device() -> SampleSource {
    SampleSource::Device {
        device: ReferenceDevice::MacM1_8Gb14_4,
        date_run: "2026-11-20".into(),
        run_by: "a person".into(),
    }
}

/// Trials that meet every AT-21 / AT-43 budget, from the scripted pipeline double.
fn good_trials() -> Vec<Trial> {
    let p = ScriptedPipeline::new();
    presses(&p, 20, SttPath::Local, Route::Question, good());
    let mut cloud = good();
    cloud.chip = Some(ms(1_900));
    presses(&p, 20, SttPath::Cloud, Route::Question, cloud);
    presses(&p, 20, SttPath::Local, Route::Dictation, good());
    p.recorder().trials()
}

// ---- Mark API --------------------------------------------------------------------------------

#[test]
fn nfr_02_mark_api_measures_key_down_to_mic_live_on_the_sim_clock() {
    let clock = SimClock::new();
    let rec = LatencyRecorder::new(Arc::new(clock.clone()));
    let t = rec.begin(SttPath::Local, Route::Question);
    rec.mark(t, Mark::KeyDown).unwrap();
    clock.advance(ms(40));
    rec.mark(t, Mark::MicLive).unwrap();
    clock.advance(ms(500));
    rec.mark(t, Mark::KeyUp).unwrap();
    clock.advance(ms(60));
    rec.mark(t, Mark::LocalAck).unwrap();
    let trials = rec.trials();
    assert_eq!(trials.len(), 1);
    assert_eq!(trials[0].span(Mark::KeyDown, Mark::MicLive), Some(ms(40)));
    assert_eq!(trials[0].span(Mark::KeyUp, Mark::LocalAck), Some(ms(60)));
    assert_eq!(trials[0].span(Mark::KeyUp, Mark::PreviewChip), None);
    assert_eq!(trials[0].path(), SttPath::Local);
    assert_eq!(trials[0].route(), Route::Question);
}

#[test]
fn at_21_mark_api_rejects_duplicate_missing_and_unknown_marks() {
    let clock = SimClock::new();
    let rec = LatencyRecorder::new(Arc::new(clock.clone()));
    let t = rec.begin(SttPath::Local, Route::Dictation);
    assert_eq!(
        rec.mark(t, Mark::KeyUp),
        Err(MarkError::MissingPrior {
            mark: Mark::KeyUp,
            needs: Mark::KeyDown
        })
    );
    rec.mark(t, Mark::KeyDown).unwrap();
    assert_eq!(
        rec.mark(t, Mark::KeyDown),
        Err(MarkError::Duplicate(Mark::KeyDown))
    );
    assert_eq!(
        rec.mark(t, Mark::TextInserted),
        Err(MarkError::MissingPrior {
            mark: Mark::TextInserted,
            needs: Mark::KeyUp
        })
    );
    clock.advance(ms(100));
    rec.mark(t, Mark::KeyUp).unwrap();
    // An imported OS time before key-up is out of order.
    assert_eq!(
        rec.mark_at(t, Mark::LocalAck, ms(50)),
        Err(MarkError::OutOfOrder {
            mark: Mark::LocalAck,
            after: Mark::KeyUp
        })
    );
    let rec2 = LatencyRecorder::new(Arc::new(SimClock::new()));
    rec2.begin(SttPath::Cloud, Route::Question);
    let unknown = rec2.begin(SttPath::Cloud, Route::Question);
    assert_eq!(
        rec.mark(unknown, Mark::KeyDown),
        Err(MarkError::UnknownTrial(unknown))
    );
}

#[test]
fn at_21_mark_api_is_safe_from_many_threads() {
    let clock = SimClock::new();
    let rec = Arc::new(LatencyRecorder::new(Arc::new(clock.clone())));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let rec = Arc::clone(&rec);
            std::thread::spawn(move || {
                for _ in 0..50 {
                    let t = rec.begin(SttPath::Local, Route::Question);
                    rec.mark(t, Mark::KeyDown).unwrap();
                    rec.mark(t, Mark::KeyUp).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let trials = rec.trials();
    assert_eq!(trials.len(), 400);
    assert!(trials.iter().all(|t| t.at(Mark::KeyUp).is_some()));
}

#[test]
fn sec_16b_bench_records_only_event_names_and_times() {
    let p = ScriptedPipeline::new();
    p.press(SttPath::Local, Route::Dictation, good()).unwrap();
    let csv = p.recorder().to_csv();
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some(CSV_MAGIC));
    assert_eq!(lines.next(), Some(CSV_HEADER));
    assert_eq!(CSV_HEADER, "trial,path,route,mark,t_us");
    let names: Vec<&str> = Mark::ALL.iter().map(|m| m.name()).collect();
    for line in lines {
        let cells: Vec<&str> = line.split(',').collect();
        assert_eq!(cells.len(), 5, "{line}");
        assert!(cells[0].parse::<u64>().is_ok(), "{line}");
        assert!(["local", "cloud"].contains(&cells[1]), "{line}");
        assert!(["dictation", "question"].contains(&cells[2]), "{line}");
        assert!(names.contains(&cells[3]), "{line}");
        assert!(cells[4].parse::<u64>().is_ok(), "{line}");
    }
    assert_eq!(
        names,
        [
            "key_down",
            "mic_live",
            "key_up",
            "local_ack",
            "preview_chip",
            "text_inserted"
        ]
    );
}

#[test]
fn at_21_sample_csv_round_trips_and_rejects_bad_rows() {
    let trials = good_trials();
    let back = parse_csv(&to_csv(&trials)).expect("round trip");
    assert_eq!(back, trials);
    assert!(
        parse_csv("trial,path,route,mark,t_us\n").is_err(),
        "needs the magic line"
    );
    let bad_mark = format!("{CSV_MAGIC}\n{CSV_HEADER}\n1,local,question,transcript,5\n");
    assert!(parse_csv(&bad_mark).is_err());
    let bad_order = format!("{CSV_MAGIC}\n{CSV_HEADER}\n1,local,question,key_up,5\n");
    assert!(parse_csv(&bad_order).is_err());
    let mixed = format!(
        "{CSV_MAGIC}\n{CSV_HEADER}\n1,local,question,key_down,0\n1,cloud,question,key_up,9\n"
    );
    assert!(parse_csv(&mixed).is_err(), "one trial has one path");
    let extra = format!("{CSV_MAGIC}\n{CSV_HEADER}\n1,local,question,key_down,0,hello\n");
    assert!(parse_csv(&extra).is_err(), "no extra column can carry text");
}

// ---- Stats -----------------------------------------------------------------------------------

#[test]
fn at_21_percentiles_use_the_nearest_rank() {
    let s: Vec<Duration> = (1..=100).rev().map(ms).collect();
    assert_eq!(percentile(&s, 50), Some(ms(50)));
    assert_eq!(percentile(&s, 95), Some(ms(95)));
    assert_eq!(percentile(&s, 100), Some(ms(100)));
    assert_eq!(percentile(&s, 0), Some(ms(1)));
    assert_eq!(percentile(&[], 95), None);
    assert_eq!(percentile(&[ms(7)], 95), Some(ms(7)));
    assert_eq!(percentile(&s, 101), None);
    let twenty: Vec<Duration> = (1..=20).map(ms).collect();
    let sum = Summary::of(&twenty).unwrap();
    assert_eq!(
        (sum.n, sum.p50, sum.p95, sum.max),
        (20, ms(10), ms(19), ms(20))
    );
}

#[test]
fn at_21_budget_table_matches_the_nfr_rows() {
    let ids: Vec<&str> = budgets(BudgetConfig::default())
        .iter()
        .map(|b| b.id)
        .collect();
    assert_eq!(
        ids,
        ["NFR-01a", "NFR-01b", "NFR-01c", "NFR-01d", "NFR-02", "NFR-08a", "NFR-08b"]
    );
    assert!(budget("NFR-99", BudgetConfig::default()).is_none());
}

#[test]
fn nfr_01a_preview_chip_p95_within_1_5_s_with_local_stt() {
    let cfg = BudgetConfig::default();
    let hk = HotkeySamples::default();
    let p = ScriptedPipeline::new();
    let mut t = good();
    t.chip = Some(ms(1_400));
    presses(&p, 19, SttPath::Local, Route::Question, t);
    t.chip = Some(ms(1_600));
    presses(&p, 1, SttPath::Local, Route::Question, t);
    // Cloud trials do not count for the local row.
    t.chip = Some(ms(2_100));
    presses(&p, 20, SttPath::Cloud, Route::Question, t);
    assert_eq!(
        outcome("NFR-01a", &p.recorder().trials(), &hk, cfg),
        Outcome::Within
    );
    // A second slow press moves the nearest-rank p95 (21st of 22) over 1.5 s.
    let mut slow = good();
    slow.chip = Some(ms(1_600));
    p.press(SttPath::Local, Route::Question, slow).unwrap();
    p.press(SttPath::Local, Route::Question, slow).unwrap();
    assert_eq!(
        outcome("NFR-01a", &p.recorder().trials(), &hk, cfg),
        Outcome::Over
    );
}

#[test]
fn nfr_01b_preview_chip_p95_within_2_2_s_with_cloud_stt() {
    let cfg = BudgetConfig::default();
    let hk = HotkeySamples::default();
    let p = ScriptedPipeline::new();
    let mut t = good();
    t.chip = Some(ms(2_100));
    presses(&p, 20, SttPath::Cloud, Route::Question, t);
    assert_eq!(
        outcome("NFR-01b", &p.recorder().trials(), &hk, cfg),
        Outcome::Within
    );
    assert_eq!(
        outcome("NFR-01a", &p.recorder().trials(), &hk, cfg),
        Outcome::NoData
    );
    t.chip = Some(ms(2_300));
    presses(&p, 2, SttPath::Cloud, Route::Question, t);
    assert_eq!(
        outcome("NFR-01b", &p.recorder().trials(), &hk, cfg),
        Outcome::Over
    );
}

#[test]
fn nfr_01c_every_local_ack_within_100_ms() {
    let cfg = BudgetConfig::default();
    let hk = HotkeySamples::default();
    let p = ScriptedPipeline::new();
    let mut t = good();
    t.ack = ms(100);
    presses(&p, 30, SttPath::Local, Route::Dictation, t);
    assert_eq!(
        outcome("NFR-01c", &p.recorder().trials(), &hk, cfg),
        Outcome::Within
    );
    // The row has no percentile: one ack over 100 ms fails it.
    t.ack = ms(101);
    presses(&p, 1, SttPath::Cloud, Route::Question, t);
    assert_eq!(
        outcome("NFR-01c", &p.recorder().trials(), &hk, cfg),
        Outcome::Over
    );
}

#[test]
fn nfr_01d_dictation_insert_p95_within_1_5_s_with_local_stt() {
    let cfg = BudgetConfig::default();
    let hk = HotkeySamples::default();
    let p = ScriptedPipeline::new();
    let mut t = good();
    t.insert = Some(ms(1_450));
    presses(&p, 20, SttPath::Local, Route::Dictation, t);
    // A question press with a slow insert time does not count for UC1.
    t.insert = Some(ms(3_000));
    presses(&p, 5, SttPath::Local, Route::Question, t);
    assert_eq!(
        outcome("NFR-01d", &p.recorder().trials(), &hk, cfg),
        Outcome::Within
    );
    t.insert = Some(ms(1_550));
    presses(&p, 2, SttPath::Local, Route::Dictation, t);
    assert_eq!(
        outcome("NFR-01d", &p.recorder().trials(), &hk, cfg),
        Outcome::Over
    );
}

#[test]
fn nfr_02_mic_live_p95_150_ms_or_60_ms_when_sp08_passes() {
    let hk = HotkeySamples::default();
    let p = ScriptedPipeline::new();
    let mut t = good();
    t.mic_live = ms(100);
    presses(&p, 20, SttPath::Local, Route::Question, t);
    let trials = p.recorder().trials();
    assert_eq!(
        outcome("NFR-02", &trials, &hk, BudgetConfig::default()),
        Outcome::Within
    );
    let sp08 = BudgetConfig { sp08_passed: true };
    assert_eq!(outcome("NFR-02", &trials, &hk, sp08), Outcome::Over);
    let b = budget("NFR-02", sp08).unwrap();
    assert!(b.limit_text().contains("60 ms"), "{}", b.limit_text());
}

#[test]
fn nfr_08a_every_key_up_in_at_least_1000_cycles() {
    let cfg = BudgetConfig::default();
    let none: Vec<Trial> = Vec::new();
    assert_eq!(
        outcome("NFR-08a", &none, &HotkeySamples::default(), cfg),
        Outcome::NoData
    );
    assert_eq!(
        outcome("NFR-08a", &none, &full_hotkey(999, 999, ms(2)), cfg),
        Outcome::TooFewSamples {
            have: 999,
            need: 1_000
        }
    );
    assert_eq!(
        outcome("NFR-08a", &none, &full_hotkey(1_000, 999, ms(2)), cfg),
        Outcome::Over
    );
    assert_eq!(
        outcome("NFR-08a", &none, &full_hotkey(1_000, 1_000, ms(2)), cfg),
        Outcome::Within
    );
}

#[test]
fn nfr_08b_key_event_delivery_p95_within_15_ms() {
    let cfg = BudgetConfig::default();
    let none: Vec<Trial> = Vec::new();
    assert_eq!(
        outcome("NFR-08b", &none, &full_hotkey(1_000, 1_000, ms(15)), cfg),
        Outcome::Within
    );
    assert_eq!(
        outcome("NFR-08b", &none, &full_hotkey(1_000, 1_000, ms(16)), cfg),
        Outcome::Over
    );
}

#[test]
fn at_21_too_few_samples_or_none_is_never_within() {
    let cfg = BudgetConfig::default();
    let hk = HotkeySamples::default();
    assert_eq!(outcome("NFR-01a", &[], &hk, cfg), Outcome::NoData);
    let p = ScriptedPipeline::new();
    presses(&p, 19, SttPath::Local, Route::Question, good());
    assert_eq!(
        outcome("NFR-01a", &p.recorder().trials(), &hk, cfg),
        Outcome::TooFewSamples { have: 19, need: 20 }
    );
}

// ---- Import of the SP-03 probe log (#5) -------------------------------------------------------

#[test]
fn nfr_08a_sp03_log_import_counts_missed_key_ups() {
    let log = "\
[t=1.0 ms] src=tap event=created type=listen_only
EV src=tap kind=down t_ns=1000 lat_ms=0.5
EV src=tap kind=up t_ns=2000 lat_ms=1.5
EV src=carbon kind=down t_ns=2500 lat_ms=3
EV src=tap kind=down t_ns=3000 lat_ms=none
EV src=tap kind=down t_ns=4000 lat_ms=2
EV src=tap kind=up t_ns=5000 lat_ms=4
EV src=tap kind=up t_ns=5500 lat_ms=1
EV src=tap kind=down t_ns=6000 lat_ms=-1
SUMMARY src=tap downs=4 ups=3
";
    let hk = parse_sp03_log(log);
    // Four key-downs: two got a key-up, one was followed by another key-down, one was the last.
    assert_eq!(hk.cycles, 4);
    assert_eq!(hk.key_ups_seen, 2);
    // Numeric, non-negative tap latencies only; the orphan key-up still has a delivery time.
    let lat: Vec<f64> = hk
        .event_latencies
        .iter()
        .map(|d| d.as_secs_f64() * 1e3)
        .collect();
    assert_eq!(lat.len(), 5, "{lat:?}");
}

// ---- Report ----------------------------------------------------------------------------------

#[test]
fn at_21_report_refuses_pass_when_samples_come_from_doubles() {
    let trials = good_trials();
    let hk = full_hotkey(1_000, 1_000, ms(3));
    for gate in [Gate::At21, Gate::At30, Gate::At43] {
        let r = Report::build(gate, SampleSource::Doubles, device_info(), &trials, &hk);
        assert!(
            r.results.iter().all(|b| b.outcome == Outcome::Within),
            "{gate:?}"
        );
        assert_eq!(r.result(), GateResult::NotRun, "{gate:?}");
        let md = r.to_markdown();
        assert!(md.contains("| Result | not run |"), "{md}");
        assert!(!md.contains("| Result | pass |"), "{md}");
        assert!(md.contains("test doubles"), "{md}");
    }
}

#[test]
fn at_21_report_device_pass_needs_every_row_models_and_resource_use() {
    let trials = good_trials();
    let hk = HotkeySamples::default();
    let r = Report::build(Gate::At21, device(), device_info(), &trials, &hk);
    assert_eq!(r.result(), GateResult::Pass, "{:?}", r.missing());
    assert!(r.to_markdown().contains("| Result | pass |"));

    let mut no_models = device_info();
    no_models.models.clear();
    let r = Report::build(Gate::At21, device(), no_models, &trials, &hk);
    assert_eq!(r.result(), GateResult::Partial);
    assert!(
        r.missing().iter().any(|m| m.contains("models")),
        "{:?}",
        r.missing()
    );

    let mut no_rss = device_info();
    no_rss.resources.peak_rss_mb = None;
    let r = Report::build(Gate::At21, device(), no_rss, &trials, &hk);
    assert_eq!(r.result(), GateResult::Partial);

    let local_only: Vec<Trial> = trials
        .iter()
        .filter(|t| t.path() == SttPath::Local)
        .cloned()
        .collect();
    let r = Report::build(Gate::At21, device(), device_info(), &local_only, &hk);
    assert_eq!(
        r.result(),
        GateResult::Partial,
        "NFR-01b has no cloud samples"
    );

    let r = Report::build(Gate::At21, device(), device_info(), &[], &hk);
    assert_eq!(r.result(), GateResult::NotRun);

    let p = ScriptedPipeline::new();
    let mut slow = good();
    slow.ack = ms(150);
    presses(&p, 1, SttPath::Local, Route::Question, slow);
    let mut with_fail = trials.clone();
    with_fail.extend(p.recorder().trials());
    let r = Report::build(Gate::At21, device(), device_info(), &with_fail, &hk);
    assert_eq!(r.result(), GateResult::Fail);
    assert!(r.to_markdown().contains("| Result | fail |"));
}

#[test]
fn at_21_report_device_pass_needs_a_date_and_a_runner() {
    let trials = good_trials();
    let hk = HotkeySamples::default();
    let src = SampleSource::Device {
        device: ReferenceDevice::Win11I5_16Gb,
        date_run: "yesterday".into(),
        run_by: " ".into(),
    };
    let r = Report::build(Gate::At21, src, device_info(), &trials, &hk);
    assert_ne!(r.result(), GateResult::Pass);
    let missing = r.missing().join("; ");
    assert!(missing.contains("date"), "{missing}");
    assert!(missing.contains("run by"), "{missing}");
}

#[test]
fn at_21_report_fills_the_evidence_template() {
    let trials = good_trials();
    let r = Report::build(
        Gate::At21,
        device(),
        device_info(),
        &trials,
        &HotkeySamples::default(),
    );
    let md = r.to_markdown();
    for needle in [
        "# Evidence: AT-21",
        "| Gate | AT-21 |",
        "| Gate type | device |",
        "| Date run | 2026-11-20 |",
        "| Run by | a person |",
        "| Build | abc1234 |",
        "| Hardware | Apple M1 8 GB (mac-m1-8gb-14.4) |",
        "| Profile | none |",
        "| Requirements | NFR-01a, NFR-01b, NFR-01c, NFR-02 |",
        "## Method",
        "## Raw evidence",
        "## Result against the criterion",
        "## Follow-ups",
        "macOS 14.4",
        "parakeet-tdt-0.6b-v3 int8",
        "900 MB",
        "35 %",
    ] {
        assert!(md.contains(needle), "missing {needle:?} in\n{md}");
    }
}

#[test]
fn at_21_report_shows_p50_and_p95_for_the_local_and_cloud_stt_paths() {
    let trials = good_trials();
    let r = Report::build(
        Gate::At21,
        device(),
        device_info(),
        &trials,
        &HotkeySamples::default(),
    );
    let md = r.to_markdown();
    let chip_rows: Vec<&str> = md
        .lines()
        .filter(|l| l.contains("key_up -> preview_chip"))
        .collect();
    assert!(
        chip_rows
            .iter()
            .any(|l| l.contains("| local |") && l.contains("900.0 ms")),
        "{md}"
    );
    assert!(
        chip_rows
            .iter()
            .any(|l| l.contains("| cloud |") && l.contains("1900.0 ms")),
        "{md}"
    );
    assert!(md.contains("| p50 | p95 |"), "{md}");
}

#[test]
fn at_30_and_at_43_reports_cover_their_own_rows() {
    let trials = good_trials();
    let hk = full_hotkey(1_000, 1_000, ms(3));
    let at30 = Report::build(Gate::At30, device(), device_info(), &trials, &hk);
    let ids: Vec<&str> = at30.results.iter().map(|b| b.budget.id).collect();
    assert_eq!(ids, ["NFR-08a", "NFR-08b"]);
    assert_eq!(at30.result(), GateResult::Pass, "{:?}", at30.missing());
    assert!(at30.to_markdown().contains("1000 of 1000"));
    let at43 = Report::build(Gate::At43, device(), device_info(), &trials, &hk);
    let ids: Vec<&str> = at43.results.iter().map(|b| b.budget.id).collect();
    assert_eq!(ids, ["NFR-01d"]);
    assert_eq!(at43.result(), GateResult::Pass, "{:?}", at43.missing());
    assert_eq!(Gate::At30.file_name(), "AT-30-latency.md");
    assert_eq!(Gate::parse("AT-43"), Some(Gate::At43));
    assert_eq!(Gate::parse("AT-07"), None);
}

#[test]
fn at_21_scripted_pipeline_double_drives_the_sim_clock() {
    let p = ScriptedPipeline::new();
    let t = good();
    p.press(SttPath::Local, Route::Dictation, t).unwrap();
    let tr = &p.recorder().trials()[0];
    assert_eq!(tr.at(Mark::KeyDown), Some(Duration::ZERO));
    assert_eq!(tr.span(Mark::KeyDown, Mark::KeyUp), Some(ms(1_200)));
    assert_eq!(tr.span(Mark::KeyUp, Mark::PreviewChip), Some(ms(900)));
    assert_eq!(tr.span(Mark::KeyUp, Mark::TextInserted), Some(ms(950)));
    // The next press starts after the last mark of the first one.
    p.press(SttPath::Local, Route::Dictation, t).unwrap();
    let second = p.recorder().trials()[1].at(Mark::KeyDown).unwrap();
    assert!(second >= ms(1_200 + 950), "{second:?}");
}

// ---- Device runs (need a person: docs/tests/LATENCY-BENCH.md) ---------------------------------

const ENV_SAMPLES: &str = "BOTTLE_BENCH_SAMPLES";
const ENV_SP03_LOG: &str = "BOTTLE_BENCH_SP03_LOG";

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

#[test]
fn at_21_at_43_device_samples_meet_the_latency_budgets() {
    // TODO(#14, #46, #142, #58): the real PTT pipeline does not call the mark API yet, so no device
    // sample file exists. When it does, a person sets these variables and this test checks it.
    let device = match testkit::device_from(env).expect("valid BOTTLE_TEST_DEVICE") {
        Fixture::Ready(d) => d,
        Fixture::Skip(reason) => {
            eprintln!("{reason}");
            return;
        }
    };
    let Some(path) = env(ENV_SAMPLES) else {
        eprintln!(
            "skipped: {ENV_SAMPLES} is not set, so the AT-21/AT-43 latency benchmark does not run \
             on {} (see docs/tests/LATENCY-BENCH.md)",
            device.id()
        );
        return;
    };
    let text = std::fs::read_to_string(&path).expect("read the sample file");
    let trials = parse_csv(&text).expect("valid sample file");
    let hk = HotkeySamples::default();
    let cfg = BudgetConfig {
        sp08_passed: env("BOTTLE_BENCH_SP08_PASSED").as_deref() == Some("1"),
    };
    for id in ["NFR-01a", "NFR-01b", "NFR-01c", "NFR-01d", "NFR-02"] {
        let r = evaluate(&budget(id, cfg).unwrap(), &trials, &hk);
        assert_eq!(r.outcome, Outcome::Within, "{id}: {:?}", r.summary);
    }
}

#[test]
fn at_30_nfr_08a_nfr_08b_hid_rig_1000_cycles() {
    // TODO(#5, #7): the macOS log comes from the SP-03 probe; the Windows hook has no log yet.
    let rig = match testkit::hid_rig_from(env).expect("valid HID rig fixture") {
        Fixture::Ready(r) => r,
        Fixture::Skip(reason) => {
            eprintln!("{reason}");
            return;
        }
    };
    let Some(path) = env(ENV_SP03_LOG) else {
        eprintln!(
            "skipped: {ENV_SP03_LOG} is not set, so the AT-30 HID rig benchmark does not run on \
             {} (see docs/tests/LATENCY-BENCH.md)",
            rig.device.id()
        );
        return;
    };
    let text = std::fs::read_to_string(&path).expect("read the SP-03 log");
    let hk = parse_sp03_log(&text);
    for id in ["NFR-08a", "NFR-08b"] {
        let r = evaluate(&budget(id, BudgetConfig::default()).unwrap(), &[], &hk);
        assert_eq!(r.outcome, Outcome::Within, "{id}: {:?}", r.summary);
    }
}

#[test]
fn at_21_demo_doubles_are_within_budget_but_never_a_pass() {
    use mog_core::latency_bench::doubles::{demo_hotkey, demo_pipeline};
    let trials = demo_pipeline(40).recorder().trials();
    let hk = demo_hotkey(1_000);
    for gate in [Gate::At21, Gate::At30, Gate::At43] {
        let r = Report::build(gate, SampleSource::Doubles, device_info(), &trials, &hk);
        assert!(
            r.results.iter().all(|b| b.outcome == Outcome::Within),
            "{gate:?}: {:?}",
            r.results
        );
        assert_eq!(r.result(), GateResult::NotRun);
    }
}
