//! sp03-probe: SP-03 (#5) talk-key probe. CLI only: it opens no window.
//!
//! macOS: listen-only CGEventTap for the talk key, Secure Event Input poller, Carbon chord,
//! clamshell watcher, and a synthetic self-test. `analyze` runs on every OS.

#[cfg(target_os = "macos")]
mod mac;

use sp03::{pair_events, parse_args, parse_ev_line, Command, PairReport, Source, USAGE};

fn print_report(name: &str, r: &PairReport) {
    let lat = r.latency.map_or("n=0".to_string(), |s| {
        format!(
            "n={} min={:.3} p50={:.3} p95={:.3} max={:.3}",
            s.n, s.min, s.p50, s.p95, s.max
        )
    });
    println!(
        "ANALYZE src={name} downs={} ups={} pairs={} missed_key_ups={} orphan_key_ups={} \
         key_up_percent={:.3} latency_ms[{lat}]",
        r.downs,
        r.ups,
        r.pairs,
        r.missed_key_ups,
        r.orphan_key_ups,
        r.key_up_percent()
    );
}

fn analyze(path: &str) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return 1;
        }
    };
    let events: Vec<_> = text.lines().filter_map(parse_ev_line).collect();
    print_report("tap", &pair_events(&events, Source::Tap));
    print_report("carbon", &pair_events(&events, Source::Carbon));
    0
}

#[cfg(target_os = "macos")]
fn run_platform(cmd: Command) -> i32 {
    match cmd {
        Command::Status => {
            mac::status();
            0
        }
        Command::Listen(a) => mac::listen(a),
        Command::SelfTest(a) => mac::self_test(a),
        _ => unreachable!("handled in main"),
    }
}

#[cfg(not(target_os = "macos"))]
fn run_platform(_cmd: Command) -> i32 {
    // TODO(#5): Windows probe (WH_KEYBOARD_LL plus the Raw Input RIDEV_INPUTSINK fallback) is
    // not built. The design note and the elevated-window steps are in docs/spikes/SP-03.md.
    eprintln!("sp03-probe: only `analyze` runs on this OS. See docs/spikes/SP-03.md (Windows).");
    2
}

fn main() {
    let cmd = match parse_args(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let code = match cmd {
        Command::Help => {
            print!("{USAGE}");
            0
        }
        Command::Analyze { path } => analyze(&path),
        other => run_platform(other),
    };
    std::process::exit(code);
}
