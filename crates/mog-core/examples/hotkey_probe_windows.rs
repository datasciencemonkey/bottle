//! Device probe for R1 HotkeyService on Windows (#7, AT-30, device steps in #296).
//!
//! Run: `cargo run -p mog-core --example hotkey_probe_windows`. It prints only the hook
//! signals (talk key, Esc, "another key", elevated window, hook health) and the callback p99
//! every 10 s. It never prints, stores or logs key contents, and it records no audio.
//! Press Ctrl+C to stop.

#[cfg(windows)]
fn main() {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::{Duration, Instant};

    use mog_core::hotkey::HotkeyService;
    use mog_core::hotkey_windows::win32::WindowsHotkeyService;
    use mog_core::hotkey_windows::TalkKey;

    let (service, signals) = WindowsHotkeyService::new(TalkKey::RightAlt);
    if let Err(e) = service.start() {
        eprintln!("start failed: {}", e.code());
        std::process::exit(1);
    }
    println!("hook running; talk key Right-Alt; Ctrl+C to stop");
    let mut last_report = Instant::now();
    loop {
        match signals.recv_timeout(Duration::from_secs(1)) {
            Ok(signal) => println!("{signal:?}"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if last_report.elapsed() >= Duration::from_secs(10) {
            println!("callback p99: {:?}", service.callback_p99());
            last_report = Instant::now();
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("hotkey_probe_windows runs on Windows only");
}
