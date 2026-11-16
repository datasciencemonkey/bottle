//! SP-04 (#44) tests. Names carry the requirement ID so the coverage tools pick them up.

use sp04_bench::{
    p_core_count, pad_to_secs, parse_wav, peak_rss_bytes, percentile, read_wav, split_tail, stats,
    wer, Engine, WavError, SAMPLE_RATE,
};
use std::path::PathBuf;

/// Build a WAV image. `extra` chunks go between `fmt ` and `data` (like the JUNK/FLLR chunks
/// that macOS `say` writes).
fn wav(
    format: u16,
    channels: u16,
    rate: u32,
    bits: u16,
    samples: &[i16],
    extra: &[(&[u8; 4], &[u8])],
) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    for (id, data) in extra {
        body.extend_from_slice(*id);
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
        if data.len() % 2 == 1 {
            body.push(0);
        }
    }
    let block_align = channels * bits / 8;
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&16u32.to_le_bytes());
    body.extend_from_slice(&format.to_le_bytes());
    body.extend_from_slice(&channels.to_le_bytes());
    body.extend_from_slice(&rate.to_le_bytes());
    body.extend_from_slice(&(rate * block_align as u32).to_le_bytes());
    body.extend_from_slice(&block_align.to_le_bytes());
    body.extend_from_slice(&bits.to_le_bytes());
    body.extend_from_slice(b"data");
    body.extend_from_slice(&((samples.len() * 2) as u32).to_le_bytes());
    for s in samples {
        body.extend_from_slice(&s.to_le_bytes());
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

#[test]
fn sp_04_percentiles_correct() {
    // 1..=20 shuffled: nearest-rank p50 = 10, p95 = 19, max = 20, mean = 10.5.
    let mut v: Vec<f64> = (1..=20).map(f64::from).collect();
    v.reverse();
    v.swap(3, 11);
    let s = stats(&v).expect("non-empty");
    assert_eq!(s.n, 20);
    assert_eq!(s.p50, 10.0);
    assert_eq!(s.p95, 19.0);
    assert_eq!(s.max, 20.0);
    assert!((s.mean - 10.5).abs() < 1e-9);

    let sorted = [5.0, 7.0, 9.0];
    assert_eq!(percentile(&sorted, 0.0), 5.0);
    assert_eq!(percentile(&sorted, 50.0), 7.0);
    assert_eq!(percentile(&sorted, 95.0), 9.0);
    assert_eq!(percentile(&sorted, 100.0), 9.0);
}

#[test]
fn sp_04_stats_of_empty_is_none() {
    assert!(stats(&[]).is_none());
}

#[test]
fn sp_04_wav_reader_rejects_wrong_rate() {
    let img = wav(1, 1, 44_100, 16, &[0, 1, 2], &[]);
    assert_eq!(parse_wav(&img), Err(WavError::WrongRate(44_100)));
}

#[test]
fn sp_04_wav_reader_rejects_stereo_and_non_pcm16() {
    assert_eq!(
        parse_wav(&wav(1, 2, SAMPLE_RATE, 16, &[0, 0], &[])),
        Err(WavError::WrongChannels(2))
    );
    assert_eq!(
        parse_wav(&wav(3, 1, SAMPLE_RATE, 16, &[0], &[])),
        Err(WavError::NotPcm16 {
            format: 3,
            bits: 16
        })
    );
    assert_eq!(parse_wav(b"RIFX\0\0\0\0WAVE"), Err(WavError::NotRiffWave));
}

#[test]
fn sp_04_wav_reader_reads_pcm16_mono_with_extra_chunks() {
    let img = wav(
        1,
        1,
        SAMPLE_RATE,
        16,
        &[0, 16_384, -32_768, 32_767],
        &[(b"JUNK", &[0u8; 28]), (b"FLLR", &[1, 2, 3])],
    );
    let s = parse_wav(&img).expect("valid wav");
    assert_eq!(s.len(), 4);
    assert_eq!(s[0], 0.0);
    assert_eq!(s[1], 0.5);
    assert_eq!(s[2], -1.0);
    assert!((s[3] - 32_767.0 / 32_768.0).abs() < 1e-6);
}

#[test]
fn sp_04_wav_reader_reports_missing_file() {
    let r = read_wav(&PathBuf::from("/nonexistent/sp04/none.wav"));
    assert!(matches!(r, Err(WavError::Io(_))));
}

#[test]
fn sp_04_wer_counts_edits() {
    assert_eq!(
        wer("Open the sales dashboard.", "open the sales dashboard"),
        0.0
    );
    // one substitution, one deletion over 4 reference words
    assert!((wer("open the sales dashboard", "open a sales") - 0.5).abs() < 1e-9);
    // one insertion over 2 words
    assert!((wer("hello world", "hello big world") - 0.5).abs() < 1e-9);
    assert_eq!(wer("", ""), 0.0);
}

#[test]
fn sp_04_pad_to_secs_adds_trailing_silence_only() {
    let s = vec![0.25f32; 8_000];
    let p = pad_to_secs(&s, 2.0);
    assert_eq!(p.len(), 32_000);
    assert_eq!(p[7_999], 0.25);
    assert_eq!(p[8_000], 0.0);
    let long = vec![0.1f32; 40_000];
    assert_eq!(pad_to_secs(&long, 2.0).len(), 40_000);
}

#[test]
fn sp_04_tail_split_takes_last_segment() {
    let s: Vec<f32> = (0..160_000).map(|i| i as f32).collect(); // 10 s
    let (head, tail) = split_tail(&s, 2.0);
    assert_eq!(head.len(), 128_000);
    assert_eq!(tail.len(), 32_000);
    assert_eq!(tail[0], 128_000.0);
    let (h2, t2) = split_tail(&s[..16_000], 2.0);
    assert!(h2.is_empty());
    assert_eq!(t2.len(), 16_000);
}

#[test]
fn sp_04_peak_rss_is_reported() {
    let rss = peak_rss_bytes().expect("peak RSS on this OS");
    assert!(
        rss > 1_000_000,
        "peak RSS {rss} bytes is too small to be real"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn sp_04_p_core_count_on_macos() {
    let n = p_core_count().expect("hw.perflevel0.physicalcpu");
    assert!(n >= 1);
}

/// Needs the model. Set SP04_MODEL_DIR to the extracted
/// `sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8` folder. Optional: SP04_CLIP and SP04_CLIP_REF
/// (a reference text file) for a WER check; default clip is `test_wavs/en.wav` in the model folder.
#[test]
fn sp_04_loads_model_and_transcribes_known_clip() {
    let Some(dir) = std::env::var_os("SP04_MODEL_DIR").map(PathBuf::from) else {
        println!("TODO(#44): SP04_MODEL_DIR is not set; skipped the model test (see docs/spikes/SP-04.md)");
        return;
    };
    let threads = p_core_count().unwrap_or(4) as i32;
    let engine = Engine::load(&dir, threads).expect("load Parakeet");
    let clip = std::env::var_os("SP04_CLIP")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("test_wavs").join("en.wav"));
    let samples = read_wav(&clip).expect("read clip");
    let text = engine.transcribe(&samples);
    println!("transcript: {text}");
    assert!(
        text.split_whitespace().count() >= 3,
        "transcript too short: {text:?}"
    );
    if let Some(r) = std::env::var_os("SP04_CLIP_REF") {
        let reference = std::fs::read_to_string(r).expect("read reference");
        let e = wer(&reference, &text);
        assert!(e <= 0.25, "WER {e:.3} > 0.25 for {text:?}");
    }
}
