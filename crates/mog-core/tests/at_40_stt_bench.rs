//! AT-40 "STT benchmark gate" (#231), section 15.
//!
//! The AT script: run the golden set (240 or more utterances, 10 or more speakers, 4 conditions,
//! 20 non-speech clips) on both providers and the Opus path. Pass: WER 10 % or less (7 % or less
//! for quiet English), numbers 97 % or more, entities 85 % or more, 0 of 20 hallucinations, STT
//! p95 1.0 s or less on 10 s clips, Opus delta 0.5 pp or less.
//!
//! The scorer `mog_core::stt_bench` runs here on synthetic data (the `at_22_*` harness
//! self-tests, ADR 0034). The golden set audio needs people (needs:human) and the result files
//! need the STT engine (#46), the Databricks ASR client (#49) and the benchmark runner (#53), so
//! the device run skips until a person sets the variables. The capture parts are pending tests.
//! docs/tests/AT-DICTATION-STT-OFFLINE.md has the steps.

use std::time::Duration;

use mog_core::latency_bench::SampleSource;
use mog_core::stt_bench::{
    gate, normalize, opus_delta_pp, score, word_errors, GoldenSet, Hypothesis, Kind, RunResults,
    SttRoute, Utterance, MIN_CONDITIONS, MIN_NON_SPEECH, MIN_SPEAKERS, MIN_SPEECH, MIN_TEN_S_CLIPS,
};
use mog_core::testkit::{self, Fixture, ReferenceDevice};
use mog_core::types::TranscriptProvider;

const CONDITIONS: [&str; 4] = ["quiet", "noise", "far_field", "accent"];

/// A synthetic golden set that meets every rule: 240 speech items over 10 speakers and 4
/// conditions (the first 40 are 10 s clips), and 20 non-speech clips. No real speech.
fn golden() -> GoldenSet {
    let mut utterances = Vec::new();
    for i in 0..MIN_SPEECH {
        utterances.push(Utterance {
            id: format!("u{i:03}"),
            speaker: format!("s{}", i % MIN_SPEAKERS),
            condition: CONDITIONS[i % 4].to_owned(),
            lang: "en".to_owned(),
            kind: Kind::Speech,
            reference: format!(
                "Put {i}.5 percent of the budget on main.sales.orders for Acme Corp"
            ),
            numbers: vec![format!("{i}.5")],
            entities: vec!["main.sales.orders".to_owned(), "Acme Corp".to_owned()],
            audio_ms: if i < 40 { 10_000 } else { 4_000 },
        });
    }
    for i in 0..MIN_NON_SPEECH {
        utterances.push(Utterance {
            id: format!("n{i:02}"),
            speaker: "none".to_owned(),
            condition: "noise".to_owned(),
            lang: String::new(),
            kind: Kind::NonSpeech,
            reference: String::new(),
            numbers: Vec::new(),
            entities: Vec::new(),
            audio_ms: 3_000,
        });
    }
    GoldenSet {
        version: 1,
        utterances,
    }
}

/// A run that copies every reference exactly in 400 ms (a test double, not a measurement).
fn perfect(g: &GoldenSet, provider: TranscriptProvider, route: SttRoute) -> RunResults {
    RunResults {
        provider,
        route,
        items: g
            .utterances
            .iter()
            .map(|u| Hypothesis {
                id: u.id.clone(),
                text: u.reference.clone(),
                latency_ms: 400,
            })
            .collect(),
    }
}

fn all_runs(g: &GoldenSet) -> Vec<RunResults> {
    vec![
        perfect(g, TranscriptProvider::LocalParakeet, SttRoute::Direct),
        perfect(g, TranscriptProvider::DbxGemini, SttRoute::Direct),
        perfect(g, TranscriptProvider::LocalParakeet, SttRoute::Opus),
    ]
}

fn device() -> SampleSource {
    SampleSource::Device {
        device: ReferenceDevice::MacM1_8Gb14_4,
        date_run: "2026-11-16".to_owned(),
        run_by: "tester".to_owned(),
    }
}

fn edit(run: &mut RunResults, id: &str, f: impl FnOnce(&mut Hypothesis)) {
    f(run.items.iter_mut().find(|h| h.id == id).expect("item"));
}

// ---------- Harness self-tests (AT-22): the scorer ----------

#[test]
fn at_22_stt_bench_normalize_keeps_numbers_and_table_names() {
    assert_eq!(
        normalize("Put 3.5% on main.sales_orders, then 1,000 rows."),
        [
            "put",
            "3.5%",
            "on",
            "main.sales_orders",
            "then",
            "1000",
            "rows"
        ]
    );
    assert!(normalize("  ...  ").is_empty());
    assert_eq!(
        normalize("down -3, a-b, 5-6"),
        ["down", "-3", "a", "b", "5", "6"]
    );
    assert_ne!(normalize("-3"), normalize("3"));
}

#[test]
fn at_22_stt_bench_word_errors_count_substitutions_insertions_and_deletions() {
    let r = normalize("show me the sales for march");
    assert_eq!(word_errors(&r, &r), 0);
    assert_eq!(word_errors(&r, &normalize("show me the sale for march")), 1);
    assert_eq!(
        word_errors(&r, &normalize("show me all the sales for march")),
        1
    );
    assert_eq!(word_errors(&r, &normalize("show the sales march")), 2);
    assert_eq!(word_errors(&r, &[]), r.len());
}

#[test]
fn at_22_stt_bench_the_synthetic_golden_set_meets_every_rule() {
    assert_eq!(golden().problems(), Vec::<String>::new());
}

#[test]
fn at_22_stt_bench_golden_set_rules_name_each_gap() {
    let speech = |g: &GoldenSet| {
        g.utterances
            .iter()
            .filter(|u| u.kind == Kind::Speech)
            .count()
    };

    let mut g = golden();
    let pos = g.utterances.iter().position(|u| u.id == "u239").unwrap();
    g.utterances.remove(pos);
    assert_eq!(speech(&g), MIN_SPEECH - 1);
    assert!(
        g.problems().iter().any(|p| p.contains("240")),
        "{:?}",
        g.problems()
    );

    let mut g = golden();
    for u in &mut g.utterances {
        if u.speaker == "s9" {
            u.speaker = "s8".into();
        }
    }
    assert!(
        g.problems().iter().any(|p| p.contains("speakers")),
        "{:?}",
        g.problems()
    );

    let mut g = golden();
    for u in &mut g.utterances {
        if u.condition == "accent" {
            u.condition = "noise".into();
        }
    }
    assert!(
        g.problems().iter().any(|p| p.contains("conditions")),
        "{:?}",
        g.problems()
    );
    assert_eq!(MIN_CONDITIONS, 4);

    let mut g = golden();
    for u in &mut g.utterances {
        if u.condition == "quiet" {
            u.condition = "studio".into();
        }
    }
    assert!(
        g.problems().iter().any(|p| p.contains("quiet")),
        "{:?}",
        g.problems()
    );

    let mut g = golden();
    g.utterances.retain(|u| u.id != "n19");
    assert!(
        g.problems().iter().any(|p| p.contains("non-speech")),
        "{:?}",
        g.problems()
    );

    let mut g = golden();
    for u in &mut g.utterances {
        u.audio_ms = 4_000;
    }
    assert!(
        g.problems().iter().any(|p| p.contains("10 s")),
        "{:?}",
        g.problems()
    );
    assert_eq!(MIN_TEN_S_CLIPS, 20);

    let mut g = golden();
    let dup = g.utterances[0].clone();
    g.utterances.push(dup);
    assert!(
        g.problems().iter().any(|p| p.contains("duplicate")),
        "{:?}",
        g.problems()
    );

    let mut g = golden();
    g.utterances[5].reference = " . ".into();
    g.utterances[5].numbers.clear();
    g.utterances[5].entities.clear();
    assert!(
        g.problems().iter().any(|p| p.contains("no reference")),
        "{:?}",
        g.problems()
    );

    let mut g = golden();
    for u in &mut g.utterances {
        u.lang = u.lang.to_uppercase();
    }
    assert_eq!(g.problems(), Vec::<String>::new(), "EN is English");

    let mut g = golden();
    g.utterances[0].numbers.push("42".into());
    assert!(
        g.problems()
            .iter()
            .any(|p| p.contains("not in its reference")),
        "{:?}",
        g.problems()
    );
}

#[test]
fn at_22_stt_bench_a_perfect_run_scores_zero_wer_and_full_accuracy() {
    let g = golden();
    let s = score(
        &g,
        &perfect(&g, TranscriptProvider::LocalParakeet, SttRoute::Direct),
    );
    assert_eq!(s.speech_n, 240);
    assert_eq!(s.wer_pct, 0.0);
    assert_eq!(s.wer_quiet_en_pct, Some(0.0));
    assert_eq!(s.numbers_pct, Some(100.0));
    assert_eq!(s.entities_pct, Some(100.0));
    assert_eq!(s.hallucinations, 0);
    assert_eq!(s.non_speech_n, 20);
    assert_eq!(s.ten_s_n, 40);
    assert_eq!(s.p95_ten_s, Some(Duration::from_millis(400)));
    assert_eq!(s.missing, 0);
}

#[test]
fn at_22_stt_bench_numbers_and_entities_count_only_exact_tokens() {
    let g = golden();
    let mut run = perfect(&g, TranscriptProvider::LocalParakeet, SttRoute::Direct);
    edit(&mut run, "u001", |h| h.text = h.text.replace("1.5", "15"));
    edit(&mut run, "u002", |h| {
        h.text = h.text.replace("main.sales.orders", "main sales orders")
    });
    let s = score(&g, &run);
    // 240 numbers, one changed; 480 entities, one changed.
    assert!((s.numbers_pct.unwrap() - 239.0 / 240.0 * 100.0).abs() < 1e-9);
    assert!((s.entities_pct.unwrap() - 479.0 / 480.0 * 100.0).abs() < 1e-9);
    assert!(s.wer_pct > 0.0);
}

#[test]
fn at_22_stt_bench_text_on_a_non_speech_clip_is_a_hallucination() {
    let g = golden();
    let mut run = perfect(&g, TranscriptProvider::DbxGemini, SttRoute::Direct);
    edit(&mut run, "n03", |h| {
        h.text = "Thank you for watching.".into()
    });
    edit(&mut run, "n04", |h| h.text = " . ".into());
    let s = score(&g, &run);
    assert_eq!(s.hallucinations, 1, "punctuation only is no text");
}

#[test]
fn at_22_stt_bench_a_missing_result_counts_as_deleted_and_over_the_limit() {
    let g = golden();
    let mut run = perfect(&g, TranscriptProvider::LocalParakeet, SttRoute::Direct);
    run.items
        .retain(|h| !matches!(h.id.as_str(), "u000" | "u001" | "u002"));
    let s = score(&g, &run);
    assert_eq!(s.missing, 3);
    assert!(s.wer_pct > 0.0);
    assert!(s.numbers_pct.unwrap() < 100.0);
    // 3 of 40 clips of 10 s have no time: the p95 is over any limit.
    assert_eq!(s.p95_ten_s, None);
}

#[test]
fn at_22_stt_bench_p95_uses_only_the_10_s_clips() {
    let g = golden();
    let mut run = perfect(&g, TranscriptProvider::LocalParakeet, SttRoute::Direct);
    // Slow short clips do not count; 3 slow 10 s clips of 40 put the p95 over 1 s.
    for h in run.items.iter_mut().skip(40).take(100) {
        h.latency_ms = 5_000;
    }
    assert_eq!(score(&g, &run).p95_ten_s, Some(Duration::from_millis(400)));
    for id in ["u000", "u001", "u002"] {
        edit(&mut run, id, |h| h.latency_ms = 1_200);
    }
    assert_eq!(
        score(&g, &run).p95_ten_s,
        Some(Duration::from_millis(1_200))
    );
}

#[test]
fn at_22_stt_bench_opus_delta_is_the_wer_difference_in_points() {
    let g = golden();
    let direct = score(
        &g,
        &perfect(&g, TranscriptProvider::LocalParakeet, SttRoute::Direct),
    );
    let mut opus_run = perfect(&g, TranscriptProvider::LocalParakeet, SttRoute::Opus);
    edit(&mut opus_run, "u010", |h| {
        h.text = h.text.replace("budget", "budgie")
    });
    let opus = score(&g, &opus_run);
    let words: usize = g
        .utterances
        .iter()
        .map(|u| normalize(&u.reference).len())
        .sum();
    let want = 100.0 / words as f64;
    assert!((opus_delta_pp(&direct, &opus) - want).abs() < 1e-9);
}

#[test]
fn at_22_stt_bench_doubles_never_pass_the_gate() {
    let g = golden();
    let r = gate(&g, &all_runs(&g), &SampleSource::Doubles);
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    assert!(!r.passed(), "a run of test doubles is never evidence");
    assert_eq!(r.result_text(), "not evidence");
    let r = gate(&g, &all_runs(&g), &device());
    assert!(r.passed(), "{:?}", r.failures);
    assert_eq!(r.result_text(), "pass");
}

#[test]
fn at_22_stt_bench_the_gate_fails_each_threshold_with_its_reason() {
    let g = golden();
    type Breaker = Box<dyn Fn(&mut Vec<RunResults>)>;
    let cases: Vec<(&str, Breaker)> = vec![
        (
            "WER",
            Box::new(|runs| {
                for h in runs[0]
                    .items
                    .iter_mut()
                    .filter(|h| h.id.starts_with('u'))
                    .take(60)
                {
                    h.text = "something else entirely and wrong".into();
                }
            }),
        ),
        (
            "quiet English",
            Box::new(|runs| {
                // Only quiet items (every 4th): total WER stays under 10 %, quiet WER goes over 7 %.
                for h in runs[0]
                    .items
                    .iter_mut()
                    .filter(|h| h.id.starts_with('u'))
                    .step_by(4)
                    .take(10)
                {
                    h.text = "nothing".into();
                }
            }),
        ),
        (
            "numbers",
            Box::new(|runs| {
                for h in runs[1].items.iter_mut().take(10) {
                    h.text = h.text.replace(".5", ".6");
                }
            }),
        ),
        (
            "entities",
            Box::new(|runs| {
                for h in runs[1].items.iter_mut().take(80) {
                    h.text = h.text.replace("Acme Corp", "Acne Court");
                }
            }),
        ),
        (
            "hallucination",
            Box::new(|runs| {
                let h = runs[1].items.iter_mut().find(|h| h.id == "n00").unwrap();
                h.text = "you".into();
            }),
        ),
        (
            "p95",
            Box::new(|runs| {
                for h in runs[0].items.iter_mut().take(3) {
                    h.latency_ms = 1_001;
                }
            }),
        ),
        (
            "Opus",
            Box::new(|runs| {
                for h in runs[2]
                    .items
                    .iter_mut()
                    .filter(|h| h.id.starts_with('u'))
                    .take(20)
                {
                    h.text = h.text.replace("budget", "budgie");
                }
            }),
        ),
        (
            "no run",
            Box::new(|runs| {
                runs.remove(1);
            }),
        ),
    ];
    for (want, f) in cases {
        let mut runs = all_runs(&g);
        f(&mut runs);
        let r = gate(&g, &runs, &device());
        assert!(!r.passed(), "{want}: the gate must fail");
        assert!(
            r.failures.iter().any(|m| m.contains(want)),
            "{want}: {:?}",
            r.failures
        );
    }
}

#[test]
fn at_22_stt_bench_duplicate_results_and_a_bad_run_date_do_not_pass() {
    let g = golden();
    let mut runs = all_runs(&g);
    let dup = runs[1].items[0].clone();
    runs[1].items.push(dup);
    let r = gate(&g, &runs, &device());
    assert!(
        r.failures.iter().any(|m| m.contains("duplicate")),
        "{:?}",
        r.failures
    );

    let bad_date = SampleSource::Device {
        device: ReferenceDevice::MacM1_8Gb14_4,
        date_run: "yesterday".to_owned(),
        run_by: "tester".to_owned(),
    };
    let r = gate(&g, &all_runs(&g), &bad_date);
    assert!(r.failures.is_empty(), "{:?}", r.failures);
    assert!(!r.passed());
    assert_eq!(r.result_text(), "not evidence");
}

#[test]
fn at_22_stt_bench_an_invalid_golden_set_fails_the_gate() {
    let mut g = golden();
    g.utterances.truncate(100);
    let runs = all_runs(&golden());
    let r = gate(&g, &runs, &device());
    assert!(!r.passed());
    assert!(
        r.failures.iter().any(|m| m.contains("golden set")),
        "{:?}",
        r.failures
    );
}

#[test]
fn at_22_stt_bench_files_parse_and_reject_unknown_fields() {
    let g = GoldenSet::from_json(
        r#"{"version":1,"utterances":[{"id":"u1","speaker":"s1","condition":"quiet","lang":"en",
        "kind":"speech","reference":"two rows","numbers":["two"],"entities":[],"audio_ms":2000}]}"#,
    )
    .expect("a valid manifest");
    assert_eq!(g.utterances[0].kind, Kind::Speech);
    assert!(GoldenSet::from_json(r#"{"version":1,"utterances":[],"extra":1}"#).is_err());
    let r = RunResults::from_json(
        r#"{"provider":"local_parakeet","route":"opus","items":[{"id":"u1","text":"two rows","latency_ms":300}]}"#,
    )
    .expect("a valid result file");
    assert_eq!(r.route, SttRoute::Opus);
    assert!(
        RunResults::from_json(r#"{"provider":"whisper_x","route":"direct","items":[]}"#).is_err()
    );
}

#[test]
fn at_22_stt_bench_the_summary_holds_numbers_and_no_text() {
    let g = golden();
    let mut runs = all_runs(&g);
    edit(&mut runs[0], "u005", |h| {
        h.text = "a private sentence that must not print".into()
    });
    let r = gate(&g, &runs, &device());
    let text = r.summary();
    assert!(text.contains("local_parakeet"), "{text}");
    assert!(!text.contains("private sentence"), "{text}");
    assert!(!text.contains("main.sales.orders"), "{text}");
}

// ---------- AT-40 device run (needs:human) ----------

const ENV_GOLDEN: &str = "BOTTLE_STT_GOLDEN";
const ENV_RESULTS: &str = "BOTTLE_STT_RESULTS";

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

#[test]
fn at_40_device_golden_set_meets_the_gate() {
    // TODO(#53): the runner that writes the result files; it needs #46 (local) and #49 (ASR).
    let device = match testkit::device_from(env).expect("valid BOTTLE_TEST_DEVICE") {
        Fixture::Ready(d) => d,
        Fixture::Skip(reason) => {
            eprintln!("{reason}");
            return;
        }
    };
    let (Some(golden_path), Some(results)) = (env(ENV_GOLDEN), env(ENV_RESULTS)) else {
        eprintln!(
            "skipped: {ENV_GOLDEN} or {ENV_RESULTS} is not set, so the AT-40 STT benchmark does \
             not run on {} (see docs/tests/AT-DICTATION-STT-OFFLINE.md)",
            device.id()
        );
        return;
    };
    let g =
        GoldenSet::from_json(&std::fs::read_to_string(&golden_path).expect("read the golden set"))
            .expect("valid golden set");
    let runs: Vec<RunResults> = results
        .split(',')
        .map(|p| {
            RunResults::from_json(&std::fs::read_to_string(p.trim()).expect("read a result file"))
                .expect("valid result file")
        })
        .collect();
    let source = SampleSource::Device {
        device,
        date_run: env("BOTTLE_STT_DATE").unwrap_or_default(),
        run_by: env("BOTTLE_STT_RUN_BY").unwrap_or_default(),
    };
    let r = gate(&g, &runs, &source);
    // Numbers only: the summary never holds a reference or a hypothesis.
    eprintln!("{}", r.summary());
    assert!(
        r.passed(),
        "AT-40 gate is {}: {:?} (set BOTTLE_STT_DATE as YYYY-MM-DD and BOTTLE_STT_RUN_BY)",
        r.result_text(),
        r.failures
    );
}

// ---------- Pending: capture and Opus parts ----------

#[test]
fn at_40_cap_20_quiet_input_reaches_minus_3_dbfs_before_stt() {
    use mog_core::audio_device::normalize::{normalize_peak, peak_dbfs, TARGET_PEAK_DBFS};
    // A quiet synthetic tone at -30 dBFS: after the pre-STT normalisation it peaks at
    // -3 dBFS +- 0.5 dB (CAP-20, ADR 0067).
    let amp = 10f32.powf(-30.0 / 20.0);
    let mut segment: Vec<f32> = (0..16_000)
        .map(|i| amp * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 16_000.0).sin())
        .collect();
    assert!((peak_dbfs(&segment) - -30.0).abs() < 0.5);
    normalize_peak(&mut segment);
    assert!((peak_dbfs(&segment) - TARGET_PEAK_DBFS).abs() < 0.5);
}

#[test]
#[ignore = "pending TODO(#46): R2 does not hand a segment to R4 yet"]
fn at_40_cap_20_segment_handed_to_r4_peaks_at_minus_3_dbfs() {
    // TODO(#46): R2 with a quiet synthetic tone at -30 dBFS through the mic sink and the PTT
    // buffer (#21): the segment that R4 reads peaks at -3 dBFS +- 0.5 dB. The buffer calls
    // `audio_device::normalize::normalize_peak` on the segment before R4 (ADR 0067).
    todo!("TODO(#46): read the peak of the segment that R2 hands to R4 and assert -3 dBFS");
}

#[test]
#[ignore = "pending TODO(#126): the Opus round trip and its 32 kbps fallback are not built"]
fn at_40_cap_33a_cap_33b_opus_penalty_is_listed_and_over_0_5_pp_selects_32_kbps() {
    // The scorer part (the delta) runs in at_22_stt_bench_opus_delta_is_the_wer_difference_in_points.
    // TODO(#126): run the golden set through the journal Opus encoder at the default bitrate, list
    // the delta in the report (CAP-33a), and when it is over 0.5 pp read the journal config and
    // assert 32 kbps (CAP-33b).
    let g = golden();
    let direct = score(
        &g,
        &perfect(&g, TranscriptProvider::LocalParakeet, SttRoute::Direct),
    );
    assert_eq!(direct.wer_pct, 0.0);
    todo!("TODO(#126): encode with the journal Opus path and read the journal bitrate config");
}
