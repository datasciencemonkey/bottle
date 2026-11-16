//! Device probe for R1 HotkeyService on macOS (#6, AT-30, SP-03 #5).
//!
//! Run: `cargo run -p mog-core --example hotkey_probe`. It prints only PTT phases, pill
//! changes and running counts. It never prints, stores or logs key contents, and it records
//! no audio. Starting it is the user action that may show the Input Monitoring prompt
//! (CAP-36). Steps are in `docs/tests/hotkey-macos.md`.

#[cfg(target_os = "macos")]
fn main() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    use mog_core::hotkey::HotkeyService;
    use mog_core::hotkey_core::{
        request_ptt_permission, HotkeyOutput, HotkeySink, PttPhase, TalkKey, UserAction,
    };
    use mog_core::hotkey_macos::{run_main_loop, MacHotkeyService, MacPermissions};

    struct Print {
        start: Instant,
        downs: AtomicU64,
        ups: AtomicU64,
        cancels: AtomicU64,
        double_taps: AtomicU64,
    }
    impl HotkeySink for Print {
        fn emit(&self, output: HotkeyOutput) {
            let ms = self.start.elapsed().as_millis();
            match output {
                HotkeyOutput::Ptt(phase) => {
                    let n = match phase {
                        PttPhase::Down => &self.downs,
                        PttPhase::Up => &self.ups,
                        PttPhase::Cancel => &self.cancels,
                        PttPhase::DoubleTap => &self.double_taps,
                    };
                    let count = n.fetch_add(1, Ordering::Relaxed) + 1;
                    println!("{ms:>9} ms  ptt {:<6} #{count}", phase.as_str());
                }
                other => println!(
                    "{ms:>9} ms  {:?} {}",
                    other,
                    other.pill_text().unwrap_or("")
                ),
            }
        }
    }

    let sink = Arc::new(Print {
        start: Instant::now(),
        downs: AtomicU64::new(0),
        ups: AtomicU64::new(0),
        cancels: AtomicU64::new(0),
        double_taps: AtomicU64::new(0),
    });
    let service = MacHotkeyService::new(TalkKey::RightOption, sink);
    if let Err(e) = service.start() {
        if e.code() == "input_monitoring_required" {
            let status = request_ptt_permission(&MacPermissions, UserAction::from_user_click());
            println!("Input Monitoring: {status:?}. Grant it in System Settings, then run again.");
        } else {
            println!("start failed: {}", e.code());
        }
        std::process::exit(2);
    }
    println!("Listening. Hold Right-Option. Ctrl+Shift+Space works while Secure Input is on.");
    println!("Press Ctrl+C to quit.");
    run_main_loop();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("hotkey_probe runs on macOS only.");
}
