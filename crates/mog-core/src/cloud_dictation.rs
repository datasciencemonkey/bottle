//! Cloud dictation (ADR 0123): the talk-key audio goes to a `system.ai` Gemini model on the
//! home profile's AI Gateway, instead of the local Parakeet text.
//!
//! The request is the one audio request of call recording ([`audio_chat_body`]) on the chat
//! completions route, through [`GatewayChatClient`] (host rules, token broker, request tags and
//! the shared rate bucket of R7). The audio is FLAC, 16 kHz mono, in RAM only. The model only
//! transcribes: the filler pre-pass, the cleanup model and the word check stay in the brain.
//!
//! Live facts (DEFAULT, 2026-11-24, synthetic clips): a 3 s clip takes 1.5 to 2.4 s and a 10 s
//! clip 1.8 to 2.5 s (the worst run seen was 2.5 s). `audio_url` data URLs work on the chat
//! route; the Responses route takes the same audio as an `input_file`; `input_audio` is refused.
//!
//! The deadline ([`DEADLINE`]) is 3.5 s, not the ADR 0122 ask-loop budget: the brain's arrival
//! gate (`arrival-gate.ts`, `ARRIVAL_CAP_MS`) holds Ask Genie arrivals for a pasting hold for at
//! most 5 s after key-up. [`DEADLINE`] plus the 1.5 s dictation cleanup budget
//! (`DICTATION_CLEANUP_DEADLINE_MS`) must stay at or under that cap, or a slow cloud call can let
//! an arrival move the card before the paste lands. 3.5 s + 1.5 s = 5 s, with headroom over the
//! 2.5 s worst probed latency.
//!
//! Privacy: the home profile only (Bottle never picks one), no audio, text, prompt or token in a
//! log, a `Debug` or an error. Every error is a content-free code. The caller falls back to the
//! local text on any error.

use std::io::Cursor;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use crate::audio_device::normalize::normalize_peak;
use crate::model_gateway::client::GatewayChatClient;
use crate::recording_capture::flac::{FlacWriter, BLOCK_SIZE};
use crate::recording_cloud::model::{audio_chat_body, reply_text};
use crate::types::{TagApp, TagOp, Tags, RECORDING_DEFAULT_TRANSCRIBE_ENDPOINT};

/// The model of cloud dictation: the recording transcribe model (one Gemini default).
pub const MODEL: &str = RECORDING_DEFAULT_TRANSCRIBE_ENDPOINT;
/// The deadline of one cloud transcription. Past it, Bottle uses the local text. Fits the
/// arrival gate's 5 s cap alongside the 1.5 s dictation cleanup budget (see the module doc).
pub const DEADLINE: Duration = Duration::from_millis(3_500);
/// `max_tokens` of the call (120 s of speech is about 400 words).
pub const MAX_TOKENS: u32 = 2_000;
/// The runtime that `Transcript.runtime` shows for a cloud transcript.
pub const RUNTIME: &str = "ai-gateway";
/// The prompt. It asks for the words as spoken: the cleanup stays in the brain.
pub const PROMPT: &str = "Transcribe this audio exactly as spoken. Keep every word. \
Reply with only the text: no quotes, no notes and no labels. If nobody speaks, reply with nothing.";

/// A home profile source: `None` until the user picks a home. Bottle never picks one.
pub type HomeSource = Arc<dyn Fn() -> Option<String> + Send + Sync>;

/// The cloud half of dictation, as the STT engine sees it.
pub trait CloudStt: Send + Sync {
    /// The user chose "Databricks" as the dictation engine.
    fn enabled(&self) -> bool;
    /// The text of 16 kHz mono `samples` (peak-normalised in place), or a content-free code. It
    /// returns `off` when [`Self::enabled`] is false, `cancelled` soon after `cancel` is set and
    /// `timeout` at [`DEADLINE`].
    fn transcribe(&self, samples: &mut [f32], cancel: &AtomicBool) -> Result<String, &'static str>;
}

/// [`CloudStt`] over the AI Gateway of the home profile.
pub struct GatewayCloudStt {
    client: Arc<GatewayChatClient>,
    home: HomeSource,
    on: Arc<dyn Fn() -> bool + Send + Sync>,
    mog: String,
}

impl std::fmt::Debug for GatewayCloudStt {
    /// Shows nothing: no profile, host or MOG ID.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayCloudStt").finish_non_exhaustive()
    }
}

impl GatewayCloudStt {
    /// `on` reads the Settings choice, `home` the home profile. `mog` is the MOG ID of the
    /// request tags (op `asr`).
    pub fn new(
        client: Arc<GatewayChatClient>,
        home: HomeSource,
        on: Arc<dyn Fn() -> bool + Send + Sync>,
        mog: impl Into<String>,
    ) -> Self {
        Self {
            client,
            home,
            on,
            mog: mog.into(),
        }
    }
}

/// An upper bound on the FLAC size of `len` 16-bit samples. The largest subframe (VERBATIM) is
/// the raw 16-bit samples of the frame, so a bound of 2 bytes a sample plus a small, fixed
/// header on top of every [`BLOCK_SIZE`]-sample frame, and on the file, never undercounts.
/// Pre-sizing the output buffer to this bound means `encode` never reallocates it: the PCM take
/// buffer and the FLAC buffer are zeroed when the caller is done with them (CAP-34), and a
/// reallocation would leave an unwiped copy of either one in freed memory.
fn flac_capacity_bound(len: usize) -> usize {
    const FRAME_HEADER_BYTES: usize = 32;
    const FILE_HEADER_BYTES: usize = 64;
    let frames = len.div_ceil(BLOCK_SIZE).max(1);
    len * 2 + frames * FRAME_HEADER_BYTES + FILE_HEADER_BYTES
}

/// `samples` as FLAC (16 kHz mono 16-bit), peak-normalised in place like the local decode
/// (CAP-20). The caller owns `samples` and the returned buffer, and zeroes both (CAP-34).
fn encode(samples: &mut [f32]) -> Option<Vec<u8>> {
    normalize_peak(samples);
    let out = Vec::with_capacity(flac_capacity_bound(samples.len()));
    let mut w = FlacWriter::new(Cursor::new(out)).ok()?;
    w.write(samples).ok()?;
    w.finish().ok().map(Cursor::into_inner)
}

impl CloudStt for GatewayCloudStt {
    fn enabled(&self) -> bool {
        (self.on)()
    }

    fn transcribe(&self, samples: &mut [f32], cancel: &AtomicBool) -> Result<String, &'static str> {
        // Re-checked here too (ADR 0123): the engine re-checks `enabled()` before it calls in,
        // but a caller that does not is still refused, with no upload, if the choice is off.
        if !(self.on)() {
            return Err("off");
        }
        let profile = (self.home)().ok_or("no_home")?;
        let mut flac = encode(samples).ok_or("bad_audio")?;
        let body = audio_chat_body(MODEL, PROMPT, &flac, MAX_TOKENS).map_err(|_| "bad_audio");
        flac.fill(0);
        let body = body?;
        let tags = Tags {
            app: TagApp::Bottle,
            mog: self.mog.clone(),
            op: TagOp::Asr,
        };
        let reply = self
            .client
            .chat_completions(&profile, body, tags, DEADLINE, Some(cancel))
            .map_err(|e| e.code())?;
        reply_text(&reply)
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .ok_or("bad_reply")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_gateway::client::CHAT_COMPLETIONS_ROUTE;
    use crate::recording_cloud::http::ScriptedHttp;
    use serde_json::json;
    use std::sync::atomic::Ordering;

    fn stt(h: &Arc<ScriptedHttp>, home: Option<&'static str>, on: bool) -> GatewayCloudStt {
        GatewayCloudStt::new(
            Arc::new(GatewayChatClient::new(h.clone(), "dictation")),
            Arc::new(move || home.map(str::to_owned)),
            Arc::new(move || on),
            "dictation",
        )
    }

    fn clip() -> Vec<f32> {
        (0..16_000)
            .map(|i| ((i as f32) * 0.05).sin() * 0.2)
            .collect()
    }

    fn reply(text: &str) -> serde_json::Value {
        json!({"choices": [{"message": {"role": "assistant", "content": [
            {"type": "text", "text": text, "thoughtSignature": "SECRET-SIG"}
        ]}, "index": 0}]})
    }

    #[test]
    fn adr_0123_the_flac_capacity_bound_covers_the_real_encode_for_every_length_tried() {
        // Noisy, non-repeating samples defeat the FIXED predictors, so some frames fall back to
        // VERBATIM (raw 16-bit): the case the bound must cover. CONSTANT (silence) and short,
        // partial-block inputs are covered too.
        for len in [
            0usize,
            1,
            100,
            BLOCK_SIZE - 1,
            BLOCK_SIZE,
            BLOCK_SIZE + 1,
            3 * BLOCK_SIZE + 17,
        ] {
            let mut noisy: Vec<f32> = (0..len)
                .map(|i| ((i as f32 * 12_345.678).sin() * 0.9).fract())
                .collect();
            let encoded = encode(&mut noisy).expect("every length encodes");
            assert!(
                encoded.len() <= flac_capacity_bound(len),
                "len {len}: encoded {} > bound {}",
                encoded.len(),
                flac_capacity_bound(len)
            );
            let mut silent = vec![0.0f32; len];
            let encoded = encode(&mut silent).expect("every length encodes");
            assert!(encoded.len() <= flac_capacity_bound(len));
        }
    }

    #[test]
    fn adr_0123_cloud_dictation_sends_one_flac_audio_request_to_the_home_gateway() {
        let h = Arc::new(ScriptedHttp::new());
        h.json(CHAT_COMPLETIONS_ROUTE, 200, reply("  hello there \n"));
        let no = AtomicBool::new(false);
        let text = stt(&h, Some("HOME"), true)
            .transcribe(&mut clip(), &no)
            .unwrap();
        assert_eq!(text, "hello there");
        let seen = h.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].route, "mlflow.chat_completions");
        let b = seen[0].json.clone().unwrap();
        assert_eq!(b["model"], "system.ai.gemini-3-8-flash");
        let url = b["messages"][0]["content"][1]["audio_url"]["url"]
            .as_str()
            .unwrap();
        assert!(url.starts_with("data:audio/flac;base64,"));
        assert_eq!(b["messages"][0]["content"][0]["text"], PROMPT);
        let tags = seen[0].tags.clone().unwrap();
        assert_eq!((tags.mog.as_str(), tags.op), ("dictation", TagOp::Asr));
        // No `databricks-` serving endpoint, ever.
        assert!(!b.to_string().contains("databricks-"));
    }

    #[test]
    fn adr_0123_cloud_dictation_refuses_to_send_when_the_choice_is_off() {
        // The engine re-checks `enabled()` before it calls in, but `GatewayCloudStt` itself
        // refuses too, so no other caller can upload while the choice is off.
        let h = Arc::new(ScriptedHttp::new());
        let no = AtomicBool::new(false);
        let r = stt(&h, Some("HOME"), false).transcribe(&mut clip(), &no);
        assert_eq!(r, Err("off"));
        assert!(h.seen().is_empty());
    }

    #[test]
    fn adr_0123_cloud_dictation_needs_the_home_profile_and_sends_nothing_without_one() {
        let h = Arc::new(ScriptedHttp::new());
        let no = AtomicBool::new(false);
        let r = stt(&h, None, true).transcribe(&mut clip(), &no);
        assert_eq!(r, Err("no_home"));
        assert!(h.seen().is_empty());
    }

    #[test]
    fn adr_0123_cloud_dictation_a_set_cancel_sends_nothing() {
        let h = Arc::new(ScriptedHttp::new());
        h.json(CHAT_COMPLETIONS_ROUTE, 200, reply("x"));
        let cancel = AtomicBool::new(false);
        cancel.store(true, Ordering::SeqCst);
        let r = stt(&h, Some("HOME"), true).transcribe(&mut clip(), &cancel);
        assert_eq!(r, Err("cancelled"));
        assert!(h.seen().is_empty());
    }

    #[test]
    fn adr_0123_cloud_dictation_errors_are_content_free_codes() {
        let cases: [(u16, &str); 5] = [
            (401, "signed_out"),
            (404, "model_not_found"),
            (429, "rate_limited"),
            (503, "offline"),
            (400, "refused"),
        ];
        for (status, want) in cases {
            let h = Arc::new(ScriptedHttp::new());
            h.json(
                CHAT_COMPLETIONS_ROUTE,
                status,
                json!({"error_code": "X", "message": "SECRET SPEECH"}),
            );
            let no = AtomicBool::new(false);
            let r = stt(&h, Some("HOME"), true).transcribe(&mut clip(), &no);
            assert_eq!(r, Err(want), "status {status}");
        }
    }

    #[test]
    fn adr_0123_cloud_dictation_an_empty_reply_is_an_error_not_a_blank_paste() {
        let h = Arc::new(ScriptedHttp::new());
        h.json(CHAT_COMPLETIONS_ROUTE, 200, reply("   "));
        let no = AtomicBool::new(false);
        let r = stt(&h, Some("HOME"), true).transcribe(&mut clip(), &no);
        assert_eq!(r, Err("bad_reply"));
    }

    #[test]
    fn adr_0123_cloud_dictation_debug_shows_no_profile_and_enabled_follows_the_setting() {
        let h = Arc::new(ScriptedHttp::new());
        let s = stt(&h, Some("SECRET-PROFILE"), true);
        assert!(!format!("{s:?}").contains("SECRET"));
        assert!(s.enabled());
        assert!(!stt(&h, Some("HOME"), false).enabled());
    }
}
