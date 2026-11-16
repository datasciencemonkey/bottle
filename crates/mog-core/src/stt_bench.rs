//! The STT benchmark scorer for AT-40 (#231, ADR 0034).
//!
//! It reads a golden-set manifest and one result file for each provider and route, and checks the
//! AT-40 pass rules: WER 10 % or less (7 % or less for quiet English), numbers 97 % or more,
//! entities 85 % or more, 0 hallucinations on the non-speech clips, STT p95 1.0 s or less on the
//! 10 s clips, and an Opus delta of 0.5 pp or less. The golden set itself (audio of real speakers)
//! needs people, and the result files need the engine (#46), the Databricks ASR client (#49) and
//! the benchmark runner (#53).
//!
//! Rules (ADR 0034):
//! - Text is compared as [`normalize`] tokens: lower case, punctuation removed, decimals,
//!   percents and dotted table names kept, thousands commas dropped.
//! - WER is corpus level: word edits over reference words, for the speech utterances.
//! - A number or an entity of the manifest counts as kept when its tokens appear in order and
//!   next to each other in the hypothesis.
//! - A non-speech clip with any token in its hypothesis is a hallucination.
//! - The p95 is the nearest rank ([`crate::latency_bench::percentile`]) over the speech clips of
//!   9 to 11 s. A missing result counts as deleted words and as a time over any limit.
//! - [`gate`] gives `pass` only for a run on a reference device; test doubles are never evidence
//!   ([`SampleSource`]).
//!
//! Privacy (SEC rules): [`GateReport::summary`] and the failure texts hold provider names, counts
//! and rates only, never a reference or a hypothesis.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::Deserialize;

use crate::latency_bench::{percentile, SampleSource, MIN_TIME_SAMPLES};
use crate::types::TranscriptProvider;

/// At least this many speech utterances.
pub const MIN_SPEECH: usize = 240;
/// At least this many speakers.
pub const MIN_SPEAKERS: usize = 10;
/// At least this many recording conditions; one of them is [`QUIET`].
pub const MIN_CONDITIONS: usize = 4;
/// At least this many non-speech clips.
pub const MIN_NON_SPEECH: usize = 20;
/// At least this many speech clips of 9 to 11 s for the latency row.
pub const MIN_TEN_S_CLIPS: usize = MIN_TIME_SAMPLES;
/// The condition of the quiet English WER row.
pub const QUIET: &str = "quiet";

/// WER limit, percent.
pub const MAX_WER_PCT: f64 = 10.0;
/// Quiet English WER limit, percent.
pub const MAX_WER_QUIET_EN_PCT: f64 = 7.0;
/// Number accuracy floor, percent.
pub const MIN_NUMBERS_PCT: f64 = 97.0;
/// Entity accuracy floor, percent.
pub const MIN_ENTITIES_PCT: f64 = 85.0;
/// STT p95 limit on 10 s clips.
pub const MAX_P95_TEN_S: Duration = Duration::from_millis(1_000);
/// Opus round-trip WER penalty limit, percentage points.
pub const MAX_OPUS_DELTA_PP: f64 = 0.5;

const TEN_S_MIN_MS: u64 = 9_000;
const TEN_S_MAX_MS: u64 = 11_000;

/// Speech or a non-speech clip (noise, music, silence).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Speech,
    NonSpeech,
}

/// One golden-set item. The audio file stays outside the repository.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Utterance {
    pub id: String,
    pub speaker: String,
    pub condition: String,
    /// BCP 47 language tag, for example `en` or `en-GB`. Empty for non-speech.
    #[serde(default)]
    pub lang: String,
    pub kind: Kind,
    /// The reference text. Empty for non-speech.
    #[serde(default)]
    pub reference: String,
    /// The numbers of the reference, as written in it.
    #[serde(default)]
    pub numbers: Vec<String>,
    /// The entities of the reference (names, table names), as written in it.
    #[serde(default)]
    pub entities: Vec<String>,
    pub audio_ms: u64,
}

/// The golden-set manifest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenSet {
    pub version: u32,
    pub utterances: Vec<Utterance>,
}

/// The audio path: straight from capture, or through the journal Opus round trip (CAP-33a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SttRoute {
    Direct,
    Opus,
}

impl SttRoute {
    fn name(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Opus => "opus",
        }
    }
}

/// One STT output.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hypothesis {
    pub id: String,
    pub text: String,
    pub latency_ms: u64,
}

/// The output of one provider on one route over the golden set.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunResults {
    pub provider: TranscriptProvider,
    pub route: SttRoute,
    pub items: Vec<Hypothesis>,
}

fn provider_name(p: TranscriptProvider) -> &'static str {
    match p {
        TranscriptProvider::LocalParakeet => "local_parakeet",
        TranscriptProvider::DbxGemini => "dbx_gemini",
        TranscriptProvider::DbxWhisper => "dbx_whisper",
    }
}

impl GoldenSet {
    /// Parse a manifest. Unknown fields are an error.
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("golden set: {e}"))
    }

    /// Every AT-40 rule that the manifest does not meet. Empty when it is valid.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let speech: Vec<&Utterance> = self
            .utterances
            .iter()
            .filter(|u| u.kind == Kind::Speech)
            .collect();
        let non_speech = self.utterances.len() - speech.len();
        if speech.len() < MIN_SPEECH {
            out.push(format!(
                "{} speech utterances; AT-40 needs {MIN_SPEECH} or more",
                speech.len()
            ));
        }
        let speakers: BTreeSet<&str> = speech.iter().map(|u| u.speaker.as_str()).collect();
        if speakers.len() < MIN_SPEAKERS {
            out.push(format!(
                "{} speakers; AT-40 needs {MIN_SPEAKERS} or more",
                speakers.len()
            ));
        }
        let conditions: BTreeSet<&str> = speech.iter().map(|u| u.condition.as_str()).collect();
        if conditions.len() < MIN_CONDITIONS {
            out.push(format!(
                "{} conditions; AT-40 needs {MIN_CONDITIONS} or more",
                conditions.len()
            ));
        }
        if !speech.iter().any(|u| is_quiet_english(u)) {
            out.push(format!(
                "no {QUIET} English speech for the quiet English WER row"
            ));
        }
        if non_speech < MIN_NON_SPEECH {
            out.push(format!(
                "{non_speech} non-speech clips; AT-40 needs {MIN_NON_SPEECH} or more"
            ));
        }
        let ten_s = speech.iter().filter(|u| is_ten_s(u)).count();
        if ten_s < MIN_TEN_S_CLIPS {
            out.push(format!(
                "{ten_s} speech clips of 10 s (9-11 s); the p95 row needs {MIN_TEN_S_CLIPS} or more"
            ));
        }
        let mut seen = BTreeSet::new();
        for u in &self.utterances {
            if !seen.insert(u.id.as_str()) {
                out.push(format!("duplicate id {}", u.id));
            }
            let reference = normalize(&u.reference);
            for t in u.numbers.iter().chain(&u.entities) {
                if !contains_run(&reference, &normalize(t)) {
                    out.push(format!(
                        "{}: an annotated token is not in its reference",
                        u.id
                    ));
                }
            }
            if u.kind == Kind::NonSpeech && !reference.is_empty() {
                out.push(format!("{}: a non-speech clip has a reference text", u.id));
            }
        }
        out
    }
}

impl RunResults {
    /// Parse a result file. Unknown fields and unknown providers are an error.
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("result file: {e}"))
    }
}

fn is_quiet_english(u: &Utterance) -> bool {
    u.condition == QUIET && (u.lang == "en" || u.lang.starts_with("en-"))
}

fn is_ten_s(u: &Utterance) -> bool {
    (TEN_S_MIN_MS..=TEN_S_MAX_MS).contains(&u.audio_ms)
}

/// The comparison tokens of a text (ADR 0034).
pub fn normalize(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    let mut s = String::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        let digit_at = |j: Option<usize>| {
            j.and_then(|j| chars.get(j))
                .is_some_and(|c| c.is_ascii_digit())
        };
        if c == ',' && digit_at(i.checked_sub(1)) && digit_at(Some(i + 1)) {
            continue; // 1,000 -> 1000
        }
        if c.is_alphanumeric() || matches!(c, '.' | '%' | '\'' | '_') {
            s.push(c);
        } else {
            s.push(' ');
        }
    }
    s.split_whitespace()
        .map(|t| t.trim_matches(|c| c == '.' || c == '\''))
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The word edit distance (substitutions + insertions + deletions).
pub fn word_errors(reference: &[String], hypothesis: &[String]) -> usize {
    let mut prev: Vec<usize> = (0..=hypothesis.len()).collect();
    for (i, r) in reference.iter().enumerate() {
        let mut cur = Vec::with_capacity(prev.len());
        cur.push(i + 1);
        for (j, h) in hypothesis.iter().enumerate() {
            let sub = prev[j] + usize::from(r != h);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[hypothesis.len()]
}

fn contains_run(hay: &[String], needle: &[String]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// The scores of one run.
#[derive(Debug, Clone, PartialEq)]
pub struct Score {
    pub provider: TranscriptProvider,
    pub route: SttRoute,
    pub speech_n: usize,
    pub wer_pct: f64,
    /// `None` when the golden set has no quiet English speech.
    pub wer_quiet_en_pct: Option<f64>,
    /// `None` when the golden set has no annotated number.
    pub numbers_pct: Option<f64>,
    /// `None` when the golden set has no annotated entity.
    pub entities_pct: Option<f64>,
    pub hallucinations: usize,
    pub non_speech_n: usize,
    pub ten_s_n: usize,
    /// `None` when there is no 10 s clip, or the p95 sample is a missing result (over any limit).
    pub p95_ten_s: Option<Duration>,
    /// Golden-set items with no result.
    pub missing: usize,
    /// Result items whose id is not in the golden set.
    pub unknown: usize,
}

fn pct(num: usize, den: usize) -> Option<f64> {
    (den > 0).then(|| num as f64 / den as f64 * 100.0)
}

/// Score one run against the golden set.
pub fn score(golden: &GoldenSet, run: &RunResults) -> Score {
    let by_id: BTreeMap<&str, &Hypothesis> = run.items.iter().map(|h| (h.id.as_str(), h)).collect();
    let ids: BTreeSet<&str> = golden.utterances.iter().map(|u| u.id.as_str()).collect();
    let (mut words, mut errors, mut q_words, mut q_errors) = (0, 0, 0, 0);
    let (mut nums, mut nums_ok, mut ents, mut ents_ok) = (0, 0, 0, 0);
    let (mut speech_n, mut non_speech_n, mut hallucinations, mut missing) = (0, 0, 0, 0);
    // A missing result is `None`: over any limit.
    let mut ten_s: Vec<Option<Duration>> = Vec::new();
    for u in &golden.utterances {
        let h = by_id.get(u.id.as_str());
        missing += usize::from(h.is_none());
        let hyp = h.map(|h| normalize(&h.text)).unwrap_or_default();
        match u.kind {
            Kind::NonSpeech => {
                non_speech_n += 1;
                hallucinations += usize::from(!hyp.is_empty());
            }
            Kind::Speech => {
                speech_n += 1;
                let reference = normalize(&u.reference);
                let e = word_errors(&reference, &hyp);
                words += reference.len();
                errors += e;
                if is_quiet_english(u) {
                    q_words += reference.len();
                    q_errors += e;
                }
                for n in &u.numbers {
                    nums += 1;
                    nums_ok += usize::from(contains_run(&hyp, &normalize(n)));
                }
                for n in &u.entities {
                    ents += 1;
                    ents_ok += usize::from(contains_run(&hyp, &normalize(n)));
                }
                if is_ten_s(u) {
                    ten_s.push(h.map(|h| Duration::from_millis(h.latency_ms)));
                }
            }
        }
    }
    let p95_ten_s = p95_with_missing(&ten_s);
    Score {
        provider: run.provider,
        route: run.route,
        speech_n,
        wer_pct: pct(errors, words).unwrap_or(0.0),
        wer_quiet_en_pct: pct(q_errors, q_words),
        numbers_pct: pct(nums_ok, nums),
        entities_pct: pct(ents_ok, ents),
        hallucinations,
        non_speech_n,
        ten_s_n: ten_s.len(),
        p95_ten_s,
        missing,
        unknown: run
            .items
            .iter()
            .filter(|h| !ids.contains(h.id.as_str()))
            .count(),
    }
}

/// Nearest-rank p95 where a missing sample sorts above every time; `None` if it is the p95.
fn p95_with_missing(samples: &[Option<Duration>]) -> Option<Duration> {
    if samples.is_empty() {
        return None;
    }
    let mut known: Vec<Duration> = samples.iter().flatten().copied().collect();
    if known.len() == samples.len() {
        return percentile(&known, 95);
    }
    let rank = (samples.len() * 95).div_ceil(100).max(1);
    if rank > known.len() {
        return None;
    }
    known.sort();
    Some(known[rank - 1])
}

/// The Opus round-trip WER penalty in percentage points (CAP-33a).
pub fn opus_delta_pp(direct: &Score, opus: &Score) -> f64 {
    opus.wer_pct - direct.wer_pct
}

/// The AT-40 gate result.
#[derive(Debug, Clone, PartialEq)]
pub struct GateReport {
    pub scores: Vec<Score>,
    /// Every rule that failed. Counts and rates only.
    pub failures: Vec<String>,
    /// `false` for test doubles: a run on doubles is never evidence.
    pub evidence: bool,
}

impl GateReport {
    /// `true` only when the run is evidence and no rule failed.
    pub fn passed(&self) -> bool {
        self.evidence && self.failures.is_empty()
    }

    /// `pass`, `fail` or `not evidence`.
    pub fn result_text(&self) -> &'static str {
        if !self.failures.is_empty() {
            "fail"
        } else if self.evidence {
            "pass"
        } else {
            "not evidence"
        }
    }

    /// A plain-text table of the scores and the failures. No reference or hypothesis text.
    pub fn summary(&self) -> String {
        let f = |v: Option<f64>| v.map_or("n/a".to_owned(), |v| format!("{v:.2}"));
        let mut s = format!("AT-40 STT benchmark: {}\n", self.result_text());
        s.push_str("provider | route | speech | WER % | quiet EN WER % | numbers % | entities % | hallucinations | 10 s clips | p95 ms | missing\n");
        for x in &self.scores {
            s.push_str(&format!(
                "{} | {} | {} | {:.2} | {} | {} | {} | {}/{} | {} | {} | {}\n",
                provider_name(x.provider),
                x.route.name(),
                x.speech_n,
                x.wer_pct,
                f(x.wer_quiet_en_pct),
                f(x.numbers_pct),
                f(x.entities_pct),
                x.hallucinations,
                x.non_speech_n,
                x.ten_s_n,
                x.p95_ten_s
                    .map_or("over".to_owned(), |d| d.as_millis().to_string()),
                x.missing,
            ));
        }
        for m in &self.failures {
            s.push_str(&format!("- {m}\n"));
        }
        s
    }
}

/// The runs AT-40 needs: both MVP providers direct (CAP-47a) and the local Opus path (CAP-33a).
const REQUIRED: [(TranscriptProvider, SttRoute); 3] = [
    (TranscriptProvider::LocalParakeet, SttRoute::Direct),
    (TranscriptProvider::DbxGemini, SttRoute::Direct),
    (TranscriptProvider::LocalParakeet, SttRoute::Opus),
];

/// Check every AT-40 rule for `runs` on `golden`.
pub fn gate(golden: &GoldenSet, runs: &[RunResults], source: &SampleSource) -> GateReport {
    let mut failures: Vec<String> = golden
        .problems()
        .into_iter()
        .map(|p| format!("golden set: {p}"))
        .collect();
    let scores: Vec<Score> = runs.iter().map(|r| score(golden, r)).collect();
    let find = |p, r| scores.iter().find(|s| s.provider == p && s.route == r);
    for (p, r) in REQUIRED {
        if find(p, r).is_none() {
            failures.push(format!("no run for {} {}", provider_name(p), r.name()));
        }
    }
    for s in &scores {
        let who = format!("{} {}", provider_name(s.provider), s.route.name());
        if s.missing > 0 {
            failures.push(format!("{who}: {} results missing", s.missing));
        }
        if s.unknown > 0 {
            failures.push(format!(
                "{who}: {} results not in the golden set",
                s.unknown
            ));
        }
        if s.route == SttRoute::Opus {
            continue; // The Opus run gates only the delta.
        }
        if s.wer_pct > MAX_WER_PCT {
            failures.push(format!("{who}: WER {:.2} % > {MAX_WER_PCT} %", s.wer_pct));
        }
        match s.wer_quiet_en_pct {
            Some(w) if w > MAX_WER_QUIET_EN_PCT => failures.push(format!(
                "{who}: quiet English WER {w:.2} % > {MAX_WER_QUIET_EN_PCT} %"
            )),
            None => failures.push(format!("{who}: quiet English WER has no samples")),
            _ => {}
        }
        match s.numbers_pct {
            Some(n) if n < MIN_NUMBERS_PCT => {
                failures.push(format!("{who}: numbers {n:.2} % < {MIN_NUMBERS_PCT} %"))
            }
            None => failures.push(format!("{who}: numbers have no samples")),
            _ => {}
        }
        match s.entities_pct {
            Some(n) if n < MIN_ENTITIES_PCT => {
                failures.push(format!("{who}: entities {n:.2} % < {MIN_ENTITIES_PCT} %"))
            }
            None => failures.push(format!("{who}: entities have no samples")),
            _ => {}
        }
        if s.hallucinations > 0 {
            failures.push(format!(
                "{who}: hallucination on {} of {} non-speech clips",
                s.hallucinations, s.non_speech_n
            ));
        }
        match s.p95_ten_s {
            Some(d) if d <= MAX_P95_TEN_S => {}
            Some(d) => failures.push(format!(
                "{who}: p95 {} ms on 10 s clips > {} ms",
                d.as_millis(),
                MAX_P95_TEN_S.as_millis()
            )),
            None => failures.push(format!(
                "{who}: p95 on 10 s clips is over the limit or has no samples"
            )),
        }
    }
    if let (Some(d), Some(o)) = (
        find(TranscriptProvider::LocalParakeet, SttRoute::Direct),
        find(TranscriptProvider::LocalParakeet, SttRoute::Opus),
    ) {
        let delta = opus_delta_pp(d, o);
        if delta > MAX_OPUS_DELTA_PP {
            failures.push(format!(
                "local_parakeet: Opus delta {delta:.2} pp > {MAX_OPUS_DELTA_PP} pp"
            ));
        }
    }
    GateReport {
        scores,
        failures,
        evidence: matches!(source, SampleSource::Device { date_run, run_by, .. }
            if !date_run.trim().is_empty() && !run_by.trim().is_empty()),
    }
}
