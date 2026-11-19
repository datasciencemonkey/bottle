//! The Bottle Tauri app (ARC-1). One OS process: this crate links `mog-core`.
//!
//! The app only hosts windows. Domain logic runs in the hidden `brain` window (ARC-2);
//! secrets and I/O stay in `mog-core` (section 4.4). The IPC command surface is in `ipc` (#223).

pub mod agent_host;
pub mod capture_host;
pub mod cli_host;
pub mod consent_host;
pub mod hotkey_host;
pub mod hotkey_settings;
pub mod ipc;
#[cfg(target_os = "macos")]
pub mod macos_panel;
pub mod mcp_host;
pub mod mog_config_host;
pub mod mog_drag;
pub mod profiles_host;
pub mod rate_bucket_host;
pub mod recording_host;
pub mod recording_cloud_host;
pub mod webview_media;
pub mod window_host;
pub mod windows;

use tauri::Manager;
use window_host::TauriWindowHost;
use windows::WindowKind;

/// The id of the demo MOG and card that a debug build opens when `BOTTLE_DEMO_WINDOWS=1`.
pub const DEMO_ID: &str = "demo";

/// MOG-01b: macOS `activateIgnoringOtherApps` at launch. The tao default (`true`) activates Bottle
/// and takes the frontmost app away from the user (SP-01 section 4.2).
pub const ACTIVATE_IGNORING_OTHER_APPS: bool = false;

/// The windows Bottle opens at start: the hidden brain and the hub.
/// MOG and card windows open on demand (ARC-3).
pub fn startup_windows(demo: bool) -> Vec<WindowKind> {
    let mut kinds = vec![WindowKind::Brain, WindowKind::Hub];
    if demo {
        kinds.push(WindowKind::Mog(DEMO_ID.to_owned()));
        kinds.push(WindowKind::Card(DEMO_ID.to_owned()));
    }
    kinds
}

/// Debug demo only: hide and show the demo card `DEMO_CARD_OPENS` times, 1 s apart, then leave
/// it hidden, so the Mac check can count opens and watch the reaper (MOG-03b, MOG-18b).
pub const DEMO_CARD_OPENS: usize = 20;

fn demo_card_cycle(app: tauri::AppHandle<tauri::Wry>) {
    let _ = std::thread::Builder::new()
        .name("bottle-demo-card-cycle".into())
        .spawn(move || {
            let step = std::time::Duration::from_secs(1);
            let Some(host) = app.try_state::<TauriWindowHost<tauri::Wry>>() else {
                return;
            };
            // Time to start `swift tools/mac/card-check.swift` in a terminal.
            std::thread::sleep(step * 20);
            for _ in 0..DEMO_CARD_OPENS {
                let _ = host.hide_card(DEMO_ID);
                std::thread::sleep(step);
                let _ = host.show_card(DEMO_ID);
                std::thread::sleep(step);
            }
            let _ = host.hide_card(DEMO_ID);
        });
}

/// R2: build the one [`capture_host::CaptureHost`] of the process and manage it, so the
/// `capture_*` commands reach it (#425, ADR 0055). Its events go to the brain through
/// [`ipc::BrainChannel::send`], which scrubs each payload.
fn install_capture(handle: &tauri::AppHandle<tauri::Wry>) -> std::io::Result<()> {
    let app = handle.clone();
    let relay: capture_host::BrainRelay = std::sync::Arc::new(move |event, payload| {
        app.state::<ipc::BrainChannel>().send(event, payload)
    });
    // #476 (CAP-16a): `capture_subscribe` sends the levels on a Channel of the brain webview.
    let levels = capture_host::HostLevels::new(capture_host::brain_level_resolver(handle.clone()));
    // #46 (ADR 0082): R4 SttEngine, with the #413 loader (`stt_loader`). `stt_transcribe`
    // takes the results from the store (#514); `BrainSegments` sends `capture.segment` (#514).
    let segments = capture_host::BrainSegments::new(std::sync::Arc::clone(&relay));
    let stt =
        mog_core::stt_engine::LocalStt::new(stt_loader(), std::sync::Arc::new(segments), |_| {});
    // #21 (CAP-12c, CAP-34, ADR 0078): the RAM-only PTT buffer. R4 is its consumer (#46).
    let ptt = mog_core::audio_device::ptt_buffer::PttBuffer::new(std::sync::Arc::new(
        stt.engine().clone(),
    ));
    // ADR 0103: the call recording tap. The mic and the system audio device feed it.
    let clock: std::sync::Arc<dyn mog_core::clock::MonotonicClock> =
        std::sync::Arc::new(mog_core::clock::SystemClock::new());
    let tap = mog_core::recording_capture::tap::RecordingTap::new(std::sync::Arc::clone(&clock));
    // #388 (ADR 0038): the cpal mic on its own audio thread. Its frames go to the level
    // stream (#476), to the PTT buffer (#21), to R4 (#46, CAP-52) and to the call recording
    // tap (ADR 0103; it drops frames while no recording runs).
    // CAP-22 (#453, ADR 0063): `app.capture.preferBuiltInMic` reaches this mic. No mic thread:
    // the app still runs, and every start answers `host_failed`.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let (device, link) = {
        use mog_core::audio_device::cpal_backend::CpalMicHost;
        let settings = handle.try_state::<std::sync::Arc<mog_core::app_settings::AppSettings>>();
        capture_host::mic_capture_device(
            capture_host::spawn_level_mic(
                CpalMicHost::new,
                mog_core::audio_device::level::TeeSink(
                    mog_core::audio_device::level::TeeSink(ptt.sink(), stt.engine().frame_sink()),
                    tap.mic_sink(),
                ),
                levels.gate(),
            ),
            settings.as_ref().map(|s| s.inner().as_ref()),
        )
    };
    // No mic backend on other platforms (the Linux CI build).
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let (device, link): (Box<dyn mog_core::capture::AudioDevice>, Option<()>) =
        (Box::new(capture_host::UnwiredMic), None);
    // ADR 0103: R2 owns the mic and the system audio (process tap / WASAPI loopback) as one
    // device; system audio opens only for the call recording's lease.
    let (system, system_status) = recording_host::system_audio_device(&tap);
    let device: Box<dyn mog_core::capture::AudioDevice> = Box::new(
        mog_core::audio_device::system::RecordingDevice::new(device, system),
    );
    let host = capture_host::CaptureHost::with_stt(
        device,
        std::sync::Arc::clone(&clock),
        std::sync::Arc::clone(&relay),
        levels,
        ptt,
        stt.engine().clone(),
    )?;
    // ADR 0103: a muted mic must not end a call recording, so the session holds the mic's
    // zero-buffer watch while it records.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let mic_hold = link.as_ref().map(|l| l.silence_hold());
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if let Some(link) = link {
        // A stream that fails after it opened stops its leases (`device_failed`).
        host.connect_failures(&link);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = link;
    // ADR 0103: the call recording session over this core. The knob is OFF at launch. Parts
    // go to the hand-off slot that the cloud side connects; until then they stay in the
    // user-only temp dir under the app cache dir.
    let handoff = std::sync::Arc::new(recording_host::HandoffSlot::default());
    let recording_temp_root = handle
        .path()
        .app_cache_dir()
        .map(|d| d.join("recordings"))
        .unwrap_or_else(|_| std::env::temp_dir().join("bottle-recordings"));
    let recording = recording_host::RecordingHost::new(
        &host,
        tap,
        system_status,
        clock,
        std::sync::Arc::new(mog_core::clock::SystemWallClock),
        std::sync::Arc::clone(&handoff) as _,
        mog_core::recording_capture::session::SessionConfig {
            temp_root: recording_temp_root.clone(),
            ..Default::default()
        },
    )?;
    let recording = std::sync::Arc::new(recording);
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if let Some(hold) = mic_hold {
        recording
            .session()
            .set_active_hook(std::sync::Arc::new(move |on| hold.set(on)));
    }
    // Sleep stops a recording and finalizes it. Without the OS watch the app still runs.
    let _ = recording.watch_sleep(&*mog_core::token_events::os_session_source());
    // `recording.state` goes to the brain at each start, stop and system audio change.
    let state_relay = std::sync::Arc::clone(&relay);
    recording
        .session()
        .set_observer(std::sync::Arc::new(move |snap| {
            if let Ok(payload) = serde_json::to_value(snap.to_wire()) {
                let _ = state_relay(mog_core::ipc::events::RECORDING_STATE, payload);
            }
        }));
    // The `recording_*` component reaches the session; its knob writes reach the host.
    if let Some(component) =
        handle.try_state::<std::sync::Arc<mog_core::recording::RecordingComponent>>()
    {
        let weak = std::sync::Arc::downgrade(&recording);
        let asker = recording_host::os_permission_asker();
        component.connect_capture(mog_core::recording::CaptureLink {
            capture: std::sync::Arc::clone(recording.session()) as _,
            temp_root: recording_temp_root.clone(),
            on_knob: std::sync::Arc::new(move |on| {
                if let Some(r) = weak.upgrade() {
                    r.set_enabled(on, &*asker);
                }
            }),
            on_start: {
                let session = std::sync::Arc::downgrade(recording.session());
                std::sync::Arc::new(move |id, profile, instructions| {
                    if let Some(s) = session.upgrade() {
                        s.set_job(
                            id.as_str(),
                            Some(profile.clone()),
                            instructions.map(|i| i.as_str().to_owned()),
                        );
                    }
                })
            },
        });
    }
    handle.manage(host);
    handle.manage(recording);
    handle.manage(handoff);
    // CAP-51c: the hotkey host preloads R4 at each talk-key down through this state.
    handle.manage(stt);
    Ok(())
}

/// #413 (ADR 0049 amendment 1): R4 loads the Parakeet model from the W-17e folder when the app
/// is built with `--features local-stt` and a model is installed; else `stt_model_missing`.
fn stt_loader() -> std::sync::Arc<dyn mog_core::stt_engine::RecognizerLoader> {
    std::sync::Arc::new(mog_core::stt_setup::AppModelLoader::for_this_device())
}

/// The core version that this app links (ARC-1).
pub fn linked_core_version() -> &'static str {
    mog_core::core_version()
}

/// Start the app.
pub fn run() {
    let demo =
        cfg!(debug_assertions) && std::env::var("BOTTLE_DEMO_WINDOWS").is_ok_and(|v| v == "1");
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.activate_ignoring_other_apps(ACTIVATE_IGNORING_OTHER_APPS);
    builder
        .manage(ipc::BrainChannel::default())
        .manage(ipc::HubChannel::default())
        // #453 (CAP-22): the app-wide settings for `app_setting_*`, in memory until #77.
        // `install_capture` connects them to the mic. Managed before `setup` runs it.
        .manage(std::sync::Arc::new(
            mog_core::app_settings::AppSettings::default(),
        ))
        // ADR 0103: the `recording_*` component. The settings stay in memory, knob off, until
        // PR C keeps them in a JSON file in the app config dir. TODO(#65).
        .manage(std::sync::Arc::new(
            mog_core::recording::RecordingComponent::new(std::sync::Arc::new(
                mog_core::recording::MemorySettingsStore::default(),
            )),
        ))
        // G-09a, ARC-F3 (#467, #138): the ONE rate bucket of the process.
        // `rate_bucket.rs:4`: "R7 and R8 share one per process". Every R7 and R8 (and the
        // future `model_invoke` and `dbx_http` callers, #511) takes it from app state.
        .manage(rate_bucket_host::SharedRateBucket(std::sync::Arc::new(
            mog_core::http_transport::rate_bucket::AimdBucket::new(std::sync::Arc::new(
                mog_core::clock::SystemClock::new(),
            )),
        )))
        .invoke_handler(ipc::handler)
        // CAP-02b: every WebView denies mic, camera and other permission requests (#387).
        .on_permission_request(webview_media::on_permission_request)
        // R1: an opened MOG or card view moves the talk-key target (#305).
        .on_page_load(hotkey_host::on_page_load)
        .on_window_event(|window, event| {
            // R1: a focused card or a closed MOG window moves the talk-key target (#305).
            hotkey_host::on_window_event(window, event);
            // #573: a close of the hub only hides it, so a double-click on a MOG can show it
            // again (`src/brain/hub-window.ts` finds it by its label). Quit still ends the app.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if windows::hides_on_close(window.label()) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            // R11: a window that closes outside the host frees its ledger slot.
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(host) = window.try_state::<TauriWindowHost<tauri::Wry>>() {
                    host.window_destroyed(window.label());
                }
            }
        })
        .setup(move |app| {
            let handle = app.handle();
            // MOG-02 / MOG-01c: MOG windows are non-activating NSPanels that a native drag moves.
            #[cfg(target_os = "macos")]
            {
                // `setup` runs on the main thread; without it the native drag (MOG-01c) is lost.
                let mtm = objc2::MainThreadMarker::new()
                    .ok_or("Bottle setup is not on the main thread")?;
                macos_panel::install_drag_monitor(mtm);
                app.manage(TauriWindowHost::with_style(
                    handle.clone(),
                    Box::new(macos_panel::MacPanelStyle),
                ));
            }
            #[cfg(not(target_os = "macos"))]
            app.manage(TauriWindowHost::new(handle.clone()));
            // R1: start the talk key. This never prompts (CAP-36, #305). If the forwarder
            // thread cannot start, the app still runs without the talk key.
            if hotkey_host::install(handle).is_err() {
                // The brain subscribes later; `hotkey:status` is sticky, so it still gets this.
                hotkey_host::report_unavailable(handle);
            }
            // ADR 0101 (user decision 2026-11-19): once per launch, ask for each OS permission
            // that was never asked (macOS Input Monitoring, then Accessibility), on a worker.
            #[cfg(target_os = "macos")]
            hotkey_host::prompt_at_launch(handle);
            // R2: one CaptureService per process; its events go to the brain Channel (#425).
            install_capture(handle)?;
            // MB-02: the MOG config store in the app config dir (#25). If the OS gives no app
            // config dir, the app still runs; `config_*` then fail with a code (#333).
            let _ = mog_config_host::install(handle);
            // SEC-32a: the T12 consent records in the local store (#431, ADR 0071). If the OS
            // key store or the data dir is not there, the app still runs; `consent_*` then fail
            // with `consent_storage_failed`, and the notice gate stays closed (MB-08a).
            let _ = consent_host::install(handle);
            // R5: the one CliRunner of the process (#16), for T7 and R6. It runs no CLI yet.
            cli_host::install(handle);
            // T7 for `profiles_list` and the R7/R8 profile hosts (#17). Before R8, which uses it.
            profiles_host::install(handle);
            // R5 launch check on a worker (W-02d) and `cli_status`. A missing CLI never fails the
            // start.
            cli_host::start(handle);
            // R8 for `mcp_call` (#467). If its HTTP client cannot start, the app still runs;
            // `mcp_call` then answers `host_failed`.
            let _ = mcp_host::install(handle);
            // ADR 0103 (PR C): the recording cloud host, on the R6 of mcp_host.
            let _ = recording_cloud_host::install(handle);
            // "Connect a workspace" (#162): `profile_login` over R5 and the W-06a check of R6.
            profiles_host::install_login(handle);
            // R16 for `agent_*` (#307): the one shared AgentHost. If its loopback model proxy
            // cannot bind, the app still runs; `agent_*` then answer `host_failed`.
            let _ = agent_host::install(handle);
            let host = app.state::<TauriWindowHost<tauri::Wry>>();
            for kind in startup_windows(demo) {
                // MOG and card windows go through R11, so the cap and the ledger hold.
                match &kind {
                    WindowKind::Mog(id) => host.open_mog_window(id)?,
                    WindowKind::Card(id) => host.show_card(id)?,
                    _ => {
                        windows::build(handle, &kind.spec())?;
                    }
                }
            }
            // After setup the host refuses main-thread calls, which could deadlock (ADR 0022).
            host.seal_main_thread();
            // MOG-18b: destroy cards that are hidden for 5 min.
            window_host::spawn_card_reaper(handle.clone())?;
            if demo && std::env::var("BOTTLE_DEMO_CARD_CYCLE").is_ok_and(|v| v == "1") {
                // The Mac check of #78 (tools/mac/card-check.swift): 20 card opens.
                demo_card_cycle(handle.clone());
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the Bottle app")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // R1: remove the key tap on quit (#305).
                hotkey_host::shutdown(app);
                // ADR 0103: a running call recording stops with `quit` and hands off its
                // parts, before R2 stops every lease.
                if let Some(recording) =
                    app.try_state::<std::sync::Arc<recording_host::RecordingHost>>()
                {
                    recording.shutdown();
                }
                // R2: stop every capture, so the mic closes, and end the cap timer (#425).
                if let Some(capture) = app.try_state::<capture_host::CaptureHost>() {
                    capture.shutdown();
                }
            }
        });
}
