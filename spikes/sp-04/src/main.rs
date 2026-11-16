//! SP-04 (#44) benchmark binary. See docs/spikes/SP-04.md for the method.
//!
//! sp04-bench load  --model-dir DIR [--threads N]
//! sp04-bench bench --model-dir DIR --audio-dir DIR [--threads N] [--runs 20] [--warmup 3]
//!                  [--tails 1,2,3,5] [--tail-primary 3] [--long-secs 240] [--segment-secs 20]
//!                  [--platform mac|windows]

use sp04_bench::{
    excess_secs, p_core_count, pad_to_secs, peak_rss_bytes, read_wav, split_tail, stats, wer,
    Engine, Stats, SAMPLE_RATE,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const CLIP_SECS: [u32; 4] = [2, 5, 10, 30];
/// Post-roll after key-up (requirements 8.1 step 1): 120 ms of audio after the last word.
const POST_ROLL_SECS: f64 = 0.120;
/// Length of the closed VAD segment decoded just before the tail.
const PREV_SEGMENT_SECS: f64 = 5.0;
/// A clip may be this much longer than its nominal length (the 10 s say(1) clip is 10.02 s).
const LENGTH_TOLERANCE_SECS: f64 = 0.2;
const LOAD_LIMIT_MS: f64 = 3_000.0;
const TAIL_LIMIT_MS: f64 = 400.0;
const RSS_LIMIT_BYTES: u64 = 1_000_000_000;

fn usage() -> ExitCode {
    eprintln!(
        "usage: sp04-bench load --model-dir DIR [--threads N]\n       \
         sp04-bench bench --model-dir DIR --audio-dir DIR [--threads N] [--runs 20] [--warmup 3] \
         [--tails 1,2,3,5] [--tail-primary 3] [--long-secs 240] [--segment-secs 20] [--platform mac|windows]"
    );
    ExitCode::from(2)
}

struct Args {
    mode: String,
    opts: HashMap<String, String>,
}

impl Args {
    fn parse() -> Option<Args> {
        let mut it = std::env::args().skip(1);
        let mode = it.next()?;
        let mut opts = HashMap::new();
        while let Some(k) = it.next() {
            let key = k.strip_prefix("--")?.to_owned();
            opts.insert(key, it.next()?);
        }
        Some(Args { mode, opts })
    }
    fn get(&self, k: &str) -> Option<&str> {
        self.opts.get(k).map(String::as_str)
    }
    fn num<T: std::str::FromStr>(&self, k: &str, default: T) -> Result<T, String> {
        match self.get(k) {
            None => Ok(default),
            Some(v) => v.parse().map_err(|_| format!("--{k}: bad number {v:?}")),
        }
    }
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_000_000.0
}

fn verdict(pass: bool) -> &'static str {
    if pass {
        "PASS"
    } else {
        "FAIL"
    }
}

fn threads(args: &Args) -> Result<i32, String> {
    match args.get("threads") {
        Some(v) => v
            .parse()
            .map_err(|_| format!("--threads: bad number {v:?}")),
        None => p_core_count()
            .map(|n| n as i32)
            .ok_or_else(|| "--threads is required on this OS (P-core count unknown)".to_owned()),
    }
}

fn load(args: &Args) -> Result<(Engine, f64, i32), String> {
    let dir = PathBuf::from(args.get("model-dir").ok_or("--model-dir is required")?);
    let n = threads(args)?;
    let t0 = Instant::now();
    let engine = Engine::load(&dir, n)?;
    Ok((engine, t0.elapsed().as_secs_f64() * 1e3, n))
}

fn time_ms<F: FnMut() -> String>(mut f: F) -> (f64, String) {
    let t0 = Instant::now();
    let text = f();
    (t0.elapsed().as_secs_f64() * 1e3, text)
}

fn row(name: &str, secs: f64, s: &Stats) -> String {
    format!(
        "| {name} | {secs:.2} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.4} |",
        s.n,
        s.p50,
        s.p95,
        s.max,
        s.mean,
        s.p50 / (secs * 1e3)
    )
}

fn bench(args: &Args) -> Result<bool, String> {
    let audio = PathBuf::from(args.get("audio-dir").ok_or("--audio-dir is required")?);
    let runs: usize = args.num("runs", 20)?;
    let warmup: usize = args.num("warmup", 3)?;
    let tail_primary: f64 = args.num("tail-primary", 3.0)?;
    let long_secs: f64 = args.num("long-secs", 240.0)?;
    let segment_secs: f64 = args.num("segment-secs", 20.0)?;
    let tails: Vec<f64> = args
        .get("tails")
        .unwrap_or("1,2,3,5")
        .split(',')
        .map(|t| {
            t.trim()
                .parse()
                .map_err(|_| format!("--tails: bad value {t:?}"))
        })
        .collect::<Result<_, _>>()?;
    let platform = args
        .get("platform")
        .unwrap_or(if cfg!(windows) { "windows" } else { "mac" });
    let limit_10s = if platform == "windows" { 900.0 } else { 600.0 };

    let (engine, load_ms, n_threads) = load(args)?;
    let rss_after_load = peak_rss_bytes().unwrap_or(0);
    println!("## SP-04 run");
    println!();
    println!(
        "threads={n_threads} runs={runs} warmup={warmup} platform={platform} os={} arch={}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!(
        "load_ms={load_ms:.1} peak_rss_after_load_mb={:.1}",
        mb(rss_after_load)
    );
    println!();

    // Full-utterance warm latency per clip length.
    println!("### Warm latency, full clip (create stream + decode + result)");
    println!();
    println!("| clip | audio s | n | p50 ms | p95 ms | max ms | mean ms | RTF (p50) |");
    println!("|---|---|---|---|---|---|---|---|");
    let mut p95_10s = f64::NAN;
    let mut texts = Vec::new();
    let mut speech: HashMap<u32, Vec<f32>> = HashMap::new();
    let mut length_ok = true;
    for secs in CLIP_SECS {
        let path = audio.join(format!("clip_{secs}s.wav"));
        let raw = read_wav(&path).map_err(|e| e.to_string())?;
        let clip = pad_to_secs(&raw, f64::from(secs));
        if let Some(e) = excess_secs(&raw, f64::from(secs), LENGTH_TOLERANCE_SECS) {
            println!(
                "WARNING: clip_{secs}s.wav is {e:.2} s longer than {secs} s; its row is not a {secs} s measurement. \
                 Make the clip shorter (faster speech rate) and run again."
            );
            length_ok = false;
        }
        let dur = clip.len() as f64 / f64::from(SAMPLE_RATE);
        for _ in 0..warmup {
            engine.transcribe(&clip);
        }
        let mut ms = Vec::with_capacity(runs);
        let mut text = String::new();
        for _ in 0..runs {
            let (t, s) = time_ms(|| engine.transcribe(&clip));
            ms.push(t);
            text = s;
        }
        let s = stats(&ms).ok_or("--runs must be at least 1")?;
        if secs == 10 {
            p95_10s = s.p95;
        }
        println!("{}", row(&format!("{secs} s"), dur, &s));
        let reference = std::fs::read_to_string(audio.join(format!("ref_{secs}.txt"))).ok();
        texts.push((secs, reference.map(|r| wer(&r, &text)), text));
        speech.insert(secs, raw);
    }
    println!();

    println!(
        "peak_rss_after_clips_mb={:.1}",
        mb(peak_rss_bytes().unwrap_or(0))
    );
    println!();

    println!("### Transcripts (rough WER against the say(1) script)");
    println!();
    println!("| clip | WER | text |");
    println!("|---|---|---|");
    for (secs, w, text) in &texts {
        let w = w.map_or("n/a".to_owned(), |w| format!("{w:.3}"));
        println!("| {secs} s | {w} | {text} |");
    }
    println!();

    // Tail latency (requirements 8.1 step 3, CAP-52): earlier VAD segments were decoded while the
    // user spoke; after key-up only the last segment (last T s of speech + 120 ms post-roll) decodes.
    println!("### Tail latency (previous segment of up to 5 s decoded first, untimed; only the tail + 120 ms post-roll is timed)");
    println!();
    println!("| source | tail s | n | p50 ms | p95 ms | max ms | mean ms | RTF (p50) |");
    println!("|---|---|---|---|---|---|---|---|");
    let mut primary_tail_p95 = f64::NAN;
    for src in [10u32, 30] {
        let raw = &speech[&src];
        for &t in &tails {
            let (head, tail) = split_tail(raw, t);
            let mut seg = tail.to_vec();
            seg.resize(
                seg.len() + (POST_ROLL_SECS * f64::from(SAMPLE_RATE)) as usize,
                0.0,
            );
            let dur = seg.len() as f64 / f64::from(SAMPLE_RATE);
            for _ in 0..warmup {
                engine.transcribe(&seg);
            }
            let mut ms = Vec::with_capacity(runs);
            // The segment just before the tail (up to PREV_SEGMENT_SECS) decodes right before
            // key-up in the app, so decode it untimed before each timed tail.
            let (_, prev) = split_tail(head, PREV_SEGMENT_SECS);
            for _ in 0..runs {
                if !prev.is_empty() {
                    engine.transcribe(prev);
                }
                ms.push(time_ms(|| engine.transcribe(&seg)).0);
            }
            let s = stats(&ms).ok_or("--runs must be at least 1")?;
            if (t - tail_primary).abs() < 1e-9 {
                // f64::max ignores NaN, so the first value replaces the NaN start.
                primary_tail_p95 = primary_tail_p95.max(s.p95);
            }
            println!("{}", row(&format!("{src} s clip"), dur, &s));
        }
    }
    println!();

    println!(
        "peak_rss_after_tails_mb={:.1}",
        mb(peak_rss_bytes().unwrap_or(0))
    );
    println!();

    // Long stream for NFR-05: decode `long_secs` of audio in session-style VAD segments.
    if long_secs > 0.0 {
        let base = pad_to_secs(&speech[&30], 30.0);
        let total = (long_secs * f64::from(SAMPLE_RATE)) as usize;
        let long: Vec<f32> = base.iter().copied().cycle().take(total).collect();
        let seg_len = (segment_secs * f64::from(SAMPLE_RATE)) as usize;
        let t0 = Instant::now();
        for seg in long.chunks(seg_len.max(1)) {
            engine.transcribe(seg);
        }
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        println!(
            "### Long stream\n\nlong_secs={long_secs} segment_secs={segment_secs} decode_ms={ms:.1} rtf={:.4}\n",
            ms / (long_secs * 1e3)
        );
    }

    let peak = peak_rss_bytes().unwrap_or(0);
    println!(
        "peak_rss_mb={:.1} (in-process, getrusage / GetProcessMemoryInfo)",
        mb(peak)
    );
    println!();
    println!("### Criteria (SP-04)");
    println!();
    println!("| criterion | value | limit | result |");
    println!("|---|---|---|---|");
    let checks = [
        ("10 s p95", p95_10s, limit_10s, "ms"),
        (
            &*format!("tail p95 ({tail_primary} s tail; CONDITIONAL on the PTT VAD, #283)"),
            primary_tail_p95,
            TAIL_LIMIT_MS,
            "ms",
        ),
        ("peak RSS", mb(peak), mb(RSS_LIMIT_BYTES), "MB"),
        ("load (this process)", load_ms, LOAD_LIMIT_MS, "ms"),
    ];
    let mut all = true;
    for (name, v, limit, unit) in checks {
        let ok = v.is_finite() && v <= limit;
        all &= ok;
        println!(
            "| {name} | {v:.1} {unit} | <= {limit:.0} {unit} | {} |",
            verdict(ok)
        );
    }
    // If the PTT VAD closes no segment before key-up (for example the R2 15-20 s target), the
    // tail is the whole utterance: this row shows that case. It is not part of SP04_RESULT.
    println!(
        "| tail p95 if the tail is the whole 10 s utterance (informational, #283) | {p95_10s:.1} ms | <= {TAIL_LIMIT_MS:.0} ms | {} |",
        verdict(p95_10s <= TAIL_LIMIT_MS)
    );
    println!();
    if !length_ok {
        println!("SP04_RESULT INVALID (a clip is longer than its nominal length)");
        return Err("a clip is longer than its nominal length; see the WARNING lines".into());
    }
    println!("SP04_RESULT {}", verdict(all));
    Ok(all)
}

fn main() -> ExitCode {
    let Some(args) = Args::parse() else {
        return usage();
    };
    let result = match args.mode.as_str() {
        "load" => load(&args).map(|(engine, ms, n)| {
            // One short decode proves the session works; it is not part of the load time.
            let text = engine.transcribe(&vec![0.0; SAMPLE_RATE as usize]);
            println!(
                "load_ms={ms:.1} threads={n} peak_rss_mb={:.1} silence_text={text:?}",
                mb(peak_rss_bytes().unwrap_or(0))
            );
            ms <= LOAD_LIMIT_MS
        }),
        "bench" => bench(&args),
        _ => return usage(),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}
