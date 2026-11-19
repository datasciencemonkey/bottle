//! The host side of the hub's Local speech step (#413, #45, #529, ADR 0104).
//!
//! One [`SttModelHost`] per process wraps the one [`SttSetup`] (the models folder, the stored
//! consent, the `.partial` install and the hash verify, all in `mog_core::stt_setup`). It
//! answers `stt_model_status`, `stt_model_consent`, `stt_model_install`, `stt_model_verify` and
//! `stt_model_cancel`, the five commands the hardened gateway lets only the hub call.
//!
//! `stt_model_install` sends progress on a Tauri Channel. The gateway lets only the hub call
//! these commands, so the Channel always resolves on the hub webview, like
//! `view_notice_subscribe` (ADR 0091) and the mic level stream (ADR 0067, `brain_level_resolver`).
//!
//! Without the `local-stt` feature (off by default: CI has no vendored sherpa-onnx, #534, #414)
//! `stt_model_status` and `stt_model_verify` still answer from the files on disk (pure hashing,
//! no network), but `stt_model_install` answers `stt_setup_unavailable` at once: this build has
//! no HTTP client for the model download (`mog_core::stt_local` is itself gated on the feature).

#[cfg(not(feature = "local-stt"))]
use std::io;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc::{
    ChannelRef, SttConsentWire, SttModelConsentArgs, SttModelInstallArgs, SttSourceWire,
};
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::stt_model::{ModelConsent, ModelSource, ModelStatus, Progress};
use mog_core::stt_setup::{Extractor, SetupError, SetupStatus, SharedFetch, SttSetup};
use serde_json::{json, Value};
use tauri::ipc::{Channel, JavaScriptChannelId};
use tauri::Manager;

/// The component ID for the content-free error codes of this step (matches `#514`'s R4).
const COMPONENT: &str = "R4";

/// One chunk of install progress, sent to whatever resolved the hub's `onProgress` Channel.
pub trait ProgressOut: Send + Sync {
    fn progress(&self, file: &str, done: u64, total: u64);
}

/// A [`ProgressOut`] over a Tauri Channel: each chunk crosses as one JSON object. After a
/// failed send it is dead and sends nothing more (the hub window closed mid-install); the
/// install itself keeps running and still resolves with the final status.
pub struct ChannelProgressOut {
    channel: Channel<Value>,
    dead: AtomicBool,
}

impl ChannelProgressOut {
    pub fn new(channel: Channel<Value>) -> Self {
        Self {
            channel,
            dead: AtomicBool::new(false),
        }
    }
}

impl ProgressOut for ChannelProgressOut {
    fn progress(&self, file: &str, done: u64, total: u64) {
        if self.dead.load(Ordering::Relaxed) {
            return;
        }
        let msg = json!({ "file": file, "done": done, "total": total });
        if self.channel.send(msg).is_err() {
            self.dead.store(true, Ordering::Relaxed);
        }
    }
}

/// Turns a `stt_model_install` `onProgress` reference into the sink of the chunks. `None`: the
/// reference does not resolve (for example the hub webview is gone).
pub type ProgressResolver = Arc<dyn Fn(&ChannelRef) -> Option<Arc<dyn ProgressOut>> + Send + Sync>;

/// The app resolver: the reference is a Channel of the `hub` webview, because the gateway lets
/// only the hub call `stt_model_install` (like `brain_level_resolver` for the brain, #476).
pub fn hub_progress_resolver<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> ProgressResolver {
    let hub = crate::windows::WindowKind::Hub.label();
    Arc::new(move |c: &ChannelRef| {
        let id = JavaScriptChannelId::from_str(c.as_str()).ok()?;
        let webview = app.get_webview_window(&hub)?;
        let channel = id.channel_on(webview.as_ref().clone());
        Some(Arc::new(ChannelProgressOut::new(channel)) as Arc<dyn ProgressOut>)
    })
}

#[cfg(feature = "local-stt")]
fn real_net() -> SharedFetch {
    Arc::new(mog_core::stt_local::HttpRangeFetch::default())
}

#[cfg(feature = "local-stt")]
fn real_extract() -> Arc<Extractor> {
    Arc::new(|archive: &Path, dest: &Path| {
        mog_core::stt_local::extract_model_archive(
            &mog_core::stt_model::ModelSpec::parakeet_tdt_v3_int8(),
            archive,
            dest,
        )
    })
}

/// Without `local-stt` (#534, #414) `mog_core::stt_local` does not compile (no vendored
/// sherpa-onnx, no `ureq`/`tar`/`bzip2`). `stt_model_install` is refused before either of these
/// runs (see `SttModelHost::call`); they exist only so `SttSetup::for_this_device` can build.
#[cfg(not(feature = "local-stt"))]
struct UnavailableFetch;

#[cfg(not(feature = "local-stt"))]
impl mog_core::stt_model::RangeFetch for UnavailableFetch {
    fn open(
        &self,
        _url: &str,
        _offset: u64,
    ) -> Result<(u64, Box<dyn io::Read + '_>), mog_core::stt_model::FetchError> {
        Err(mog_core::stt_model::FetchError::Transport)
    }
}

#[cfg(not(feature = "local-stt"))]
fn real_net() -> SharedFetch {
    Arc::new(UnavailableFetch)
}

#[cfg(not(feature = "local-stt"))]
fn real_extract() -> Arc<Extractor> {
    Arc::new(|_: &Path, _: &Path| {
        Err(io::Error::other(
            "stt_setup_unavailable: local-stt is off in this build",
        ))
    })
}

/// `true` only in a build with the `local-stt` feature (CI builds it off, #534, #414).
fn local_stt_enabled() -> bool {
    cfg!(feature = "local-stt")
}

/// The `stt_model_*` component of the app: the one managed [`SttSetup`] (#413, #45).
pub struct SttModelHost {
    setup: SttSetup,
    resolve: ProgressResolver,
}

impl SttModelHost {
    pub fn new(setup: SttSetup, resolve: ProgressResolver) -> Self {
        SttModelHost { setup, resolve }
    }

    /// The Parakeet model (D78) in this device's W-17e folder. `None` only when the OS gives no
    /// user folder for the models (for example `HOME` is unset): then `stt_model_*` answer
    /// `host_failed`, the same as every other missing host part.
    pub fn for_this_device(resolve: ProgressResolver) -> Option<Self> {
        let setup = SttSetup::for_this_device(real_net(), real_extract()).ok()?;
        Some(SttModelHost::new(setup, resolve))
    }
}

fn status_json(s: &SetupStatus) -> Value {
    let (model, partial_bytes) = match s.model {
        ModelStatus::Installed => ("installed", 0),
        ModelStatus::Partial { bytes } => ("partial", bytes),
        ModelStatus::Missing => ("missing", 0),
    };
    let consent = match s.consent {
        ModelConsent::Granted => "granted",
        ModelConsent::Declined => "declined",
        ModelConsent::NotAsked => "not_asked",
    };
    json!({
        "model": model,
        "partialBytes": partial_bytes,
        "totalBytes": s.total_bytes,
        "consent": consent,
        "installing": s.installing,
    })
}

/// `SetupError` (and the `InstallError` it wraps) cross as their own content-free code
/// (`ERROR_CODES`, ADR 0104), never as `host_failed`.
fn setup_err(e: SetupError) -> CoreError {
    CoreError::Unavailable {
        component: COMPONENT,
        reason: e.code(),
    }
}

fn unavailable() -> CoreError {
    CoreError::Unavailable {
        component: COMPONENT,
        reason: "stt_setup_unavailable",
    }
}

fn args_of<T: for<'de> serde::Deserialize<'de>>(args: Value) -> CoreResult<T> {
    serde_json::from_value(args).map_err(|_| CoreError::Unavailable {
        component: COMPONENT,
        reason: "host_failed",
    })
}

impl GatewayComponents for SttModelHost {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        match command {
            "stt_model_status" => Ok(status_json(&self.setup.status())),
            "stt_model_consent" => {
                let a: SttModelConsentArgs = args_of(args)?;
                let consent = match a.consent {
                    SttConsentWire::Granted => ModelConsent::Granted,
                    SttConsentWire::Declined => ModelConsent::Declined,
                };
                self.setup
                    .set_consent(consent)
                    .map(|s| status_json(&s))
                    .map_err(setup_err)
            }
            "stt_model_install" => {
                // #534, #414: CI has no vendored sherpa-onnx, so the default build has no HTTP
                // client for the download at all (`mog_core::stt_local` does not compile in).
                if !local_stt_enabled() {
                    return Err(unavailable());
                }
                let a: SttModelInstallArgs = args_of(args)?;
                let source = match a.source {
                    SttSourceWire::Github => ModelSource::GithubRelease,
                    SttSourceWire::HfMirror => ModelSource::HuggingFaceMirror,
                };
                let sink = (self.resolve)(&a.on_progress);
                let mut report = move |p: Progress| {
                    if let Some(s) = &sink {
                        s.progress(p.file, p.done, p.total);
                    }
                };
                self.setup
                    .install(source, &mut report)
                    .map(|_dir| status_json(&self.setup.status()))
                    .map_err(setup_err)
            }
            "stt_model_verify" => self
                .setup
                .verify()
                .map(|()| status_json(&self.setup.status()))
                .map_err(setup_err),
            "stt_model_cancel" => Ok(json!({ "cancelled": self.setup.cancel() })),
            _ => Err(CoreError::Unavailable {
                component: COMPONENT,
                reason: "not_implemented",
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mog_core::stt_model::{FetchError, ModelSpec, PinnedFile, RangeFetch};
    use std::io::Read;
    #[cfg(feature = "local-stt")]
    use std::sync::Mutex;

    /// 21 bytes, never the real 487 MB model (#413 test rule: fake small model, fake SHA).
    const FAKE_BYTES: &[u8] = b"hello stt fake model\n";
    const FAKE_SHA: &str = "79f5799a2abfa65d47158331c9ce59b9441e01aeff50903d7c5c75a1a9fe2be1";

    fn fake_spec() -> ModelSpec {
        let file = PinnedFile {
            name: "fake.bin",
            size: FAKE_BYTES.len() as u64,
            sha256: FAKE_SHA,
        };
        ModelSpec {
            name: "fake-model",
            archive: PinnedFile {
                name: "fake-model.tar.bz2",
                size: FAKE_BYTES.len() as u64,
                sha256: FAKE_SHA,
            },
            files: vec![file],
        }
    }

    /// Serves `FAKE_BYTES` for any allowed URL, ignoring `offset` (always a fresh body).
    struct FakeNet;
    impl RangeFetch for FakeNet {
        fn open(&self, _url: &str, _offset: u64) -> Result<(u64, Box<dyn Read + '_>), FetchError> {
            Ok((0, Box::new(std::io::Cursor::new(FAKE_BYTES))))
        }
    }

    struct FailNet;
    impl RangeFetch for FailNet {
        fn open(&self, _url: &str, _offset: u64) -> Result<(u64, Box<dyn Read + '_>), FetchError> {
            Err(FetchError::Transport)
        }
    }

    fn no_extract() -> Arc<Extractor> {
        Arc::new(|_: &Path, _: &Path| Ok(()))
    }

    #[cfg(feature = "local-stt")]
    #[derive(Default)]
    struct RecordingProgress(Mutex<Vec<(String, u64, u64)>>);
    #[cfg(feature = "local-stt")]
    impl ProgressOut for RecordingProgress {
        fn progress(&self, file: &str, done: u64, total: u64) {
            self.0.lock().unwrap().push((file.to_owned(), done, total));
        }
    }

    /// A fresh scratch folder under the OS temp dir, named for `tag` and this process (so
    /// parallel test runs never collide), like `crates/mog-core/tests/stt_setup.rs::tmp`.
    fn tmp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bottle-stt-model-host-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn host(
        tag: &str,
        net: Arc<dyn RangeFetch + Send + Sync>,
        resolve: ProgressResolver,
    ) -> SttModelHost {
        let setup = SttSetup::new(tmp(tag), fake_spec(), net, no_extract());
        SttModelHost::new(setup, resolve)
    }

    fn no_progress() -> ProgressResolver {
        Arc::new(|_: &ChannelRef| None)
    }

    #[test]
    fn stt_model_status_reports_missing_with_the_pinned_total() {
        let host = host("status", Arc::new(FailNet), no_progress());
        let v = host.call("stt_model_status", json!({})).unwrap();
        assert_eq!(v["model"], "missing");
        assert_eq!(v["consent"], "not_asked");
        assert_eq!(v["installing"], false);
        assert_eq!(v["totalBytes"], FAKE_BYTES.len() as u64);
    }

    #[test]
    fn stt_model_consent_stores_the_answer_and_returns_the_new_status() {
        let host = host("consent", Arc::new(FailNet), no_progress());
        let v = host
            .call("stt_model_consent", json!({ "consent": "granted" }))
            .unwrap();
        assert_eq!(v["consent"], "granted");
        let v = host
            .call("stt_model_consent", json!({ "consent": "declined" }))
            .unwrap();
        assert_eq!(v["consent"], "declined");
    }

    // Without `local-stt`, every install answers `stt_setup_unavailable` before the consent
    // check even runs (see `stt_model_install_is_unavailable_without_the_local_stt_feature`).
    #[cfg(feature = "local-stt")]
    #[test]
    fn stt_model_install_refuses_without_consent() {
        let host = host("no-consent", Arc::new(FailNet), no_progress());
        let err = host
            .call(
                "stt_model_install",
                json!({ "source": "hf_mirror", "onProgress": "__CHANNEL__:1" }),
            )
            .unwrap_err();
        assert_eq!(err.code(), "stt_consent_required");
    }

    #[cfg(feature = "local-stt")]
    #[test]
    fn stt_model_install_downloads_checks_and_reports_progress() {
        let seen: Arc<RecordingProgress> = Arc::new(RecordingProgress::default());
        let sink = Arc::clone(&seen);
        let resolve: ProgressResolver = Arc::new(move |c: &ChannelRef| {
            assert_eq!(c.as_str(), "__CHANNEL__:1");
            Some(Arc::clone(&sink) as Arc<dyn ProgressOut>)
        });
        let host = host("install-progress", Arc::new(FakeNet), resolve);
        host.call("stt_model_consent", json!({ "consent": "granted" }))
            .unwrap();
        let v = host
            .call(
                "stt_model_install",
                json!({ "source": "hf_mirror", "onProgress": "__CHANNEL__:1" }),
            )
            .unwrap();
        assert_eq!(v["model"], "installed");
        assert_eq!(v["consent"], "granted");
        let chunks = seen.0.lock().unwrap();
        assert!(
            !chunks.is_empty(),
            "the download reports at least one chunk"
        );
        let (file, done, total) = chunks.last().unwrap();
        assert_eq!(file, "fake.bin");
        assert_eq!(*done, FAKE_BYTES.len() as u64);
        assert_eq!(*total, FAKE_BYTES.len() as u64);

        // Installed already, and it still verifies: a second install is a local no-op (no
        // network), and never re-downloads what is on disk and matches its pin.
        let before = chunks.len();
        drop(chunks);
        let v2 = host
            .call(
                "stt_model_install",
                json!({ "source": "hf_mirror", "onProgress": "__CHANNEL__:1" }),
            )
            .unwrap();
        assert_eq!(v2["model"], "installed");
        assert_eq!(
            seen.0.lock().unwrap().len(),
            before,
            "no new chunks: no re-download"
        );
    }

    #[cfg(not(feature = "local-stt"))]
    #[test]
    fn stt_model_install_is_unavailable_without_the_local_stt_feature() {
        let host = host("no-feature", Arc::new(FakeNet), no_progress());
        host.call("stt_model_consent", json!({ "consent": "granted" }))
            .unwrap();
        let err = host
            .call(
                "stt_model_install",
                json!({ "source": "hf_mirror", "onProgress": "__CHANNEL__:1" }),
            )
            .unwrap_err();
        assert_eq!(err.code(), "stt_setup_unavailable");
    }

    #[test]
    fn stt_model_verify_never_sends_a_request() {
        let host = host("verify", Arc::new(FailNet), no_progress());
        // Missing on disk: verify fails with a hash mismatch (no network was even tried, or
        // `FailNet` would have surfaced a different code).
        let err = host.call("stt_model_verify", json!({})).unwrap_err();
        assert_eq!(err.code(), "stt_hash_mismatch");
        let v = host.call("stt_model_status", json!({})).unwrap();
        assert_eq!(v["model"], "missing");
    }

    #[test]
    fn stt_model_cancel_is_false_with_nothing_running() {
        let host = host("cancel", Arc::new(FailNet), no_progress());
        let v = host.call("stt_model_cancel", json!({})).unwrap();
        assert_eq!(v["cancelled"], false);
    }

    #[test]
    fn hub_progress_resolver_only_resolves_a_well_formed_channel_reference() {
        // `ChannelRef` itself rejects anything outside `__CHANNEL__:<digits>` at parse time
        // (ARC-4), so the resolver only ever sees a well-formed reference.
        assert!(ChannelRef::try_from("not-a-channel".to_owned()).is_err());
        assert!(ChannelRef::try_from("__CHANNEL__:42".to_owned()).is_ok());
    }
}
