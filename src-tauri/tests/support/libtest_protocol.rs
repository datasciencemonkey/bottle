//! #382 NFR-23a: the part of the libtest command-line protocol that cargo-nextest and
//! `cargo test` use, for a `harness = false` test that must run its tests on the main thread.
//!
//! - `--list [--format terse]` prints `<name>: test` for each selected test and exits 0.
//! - `--exact` makes each filter match a whole name; without it a filter matches a substring.
//! - `--ignored` selects only ignored tests and `--include-ignored` selects all tests. No test
//!   here is ignored, so `--ignored` selects nothing.
//! - `--skip <pattern>` drops matching tests. Other libtest flags are accepted and have no effect.
//!
//! Tests run one after another on the calling (main) thread. A test returns `Err(detail)` or
//! panics to fail. The exit code is 0 when all selected tests pass, else 101, as libtest does.

use std::io::{self, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::ExitCode;

/// One test: its name (with the requirement IDs) and its body.
pub struct Test {
    pub name: &'static str,
    pub run: fn() -> Result<(), String>,
}

/// The parsed command line.
#[derive(Debug, Default)]
pub struct Args {
    pub list: bool,
    pub exact: bool,
    pub ignored: bool,
    pub include_ignored: bool,
    pub filters: Vec<String>,
    pub skip: Vec<String>,
}

/// Libtest flags that take a value and do not change which tests run.
const VALUE_FLAGS: &[&str] = &[
    "--format",
    "--test-threads",
    "--color",
    "--logfile",
    "--shuffle-seed",
    "-Z",
];

/// Parses the libtest arguments (without the program name).
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        match flag.as_str() {
            "--list" => out.list = true,
            "--exact" => out.exact = true,
            "--ignored" => out.ignored = true,
            "--include-ignored" => out.include_ignored = true,
            "--skip" => {
                let v = inline.or_else(|| it.next()).ok_or("--skip needs a value")?;
                out.skip.push(v);
            }
            f if VALUE_FLAGS.contains(&f) => {
                let v = inline
                    .or_else(|| it.next())
                    .ok_or_else(|| format!("{f} needs a value"))?;
                if f == "--format" && !["pretty", "terse", "json", "junit"].contains(&v.as_str()) {
                    return Err(format!("unknown --format value {v:?}"));
                }
            }
            f if f.starts_with('-') => {} // --nocapture, --quiet, -q, --show-output and others
            _ => out.filters.push(arg),
        }
    }
    Ok(out)
}

impl Args {
    /// True when the test with this name (and ignored state) is selected.
    pub fn selects(&self, name: &str, ignored: bool) -> bool {
        if self.ignored && !self.include_ignored && !ignored {
            return false;
        }
        let matches = |p: &String| {
            if self.exact {
                name == p
            } else {
                name.contains(p.as_str())
            }
        };
        (self.filters.is_empty() || self.filters.iter().any(matches))
            && !self.skip.iter().any(matches)
    }
}

/// `--list`: one `<name>: test` line per selected test.
pub fn list(tests: &[Test], args: &Args, out: &mut impl Write) -> io::Result<()> {
    for t in tests.iter().filter(|t| args.selects(t.name, false)) {
        writeln!(out, "{}: test", t.name)?;
    }
    Ok(())
}

/// Runs the selected tests in order on this thread. Returns true when none failed.
pub fn run(tests: &[Test], args: &Args, out: &mut impl Write) -> io::Result<bool> {
    let selected: Vec<&Test> = tests
        .iter()
        .filter(|t| args.selects(t.name, false))
        .collect();
    writeln!(out, "\nrunning {} tests", selected.len())?;
    let mut failed = 0;
    for t in &selected {
        let result = match catch_unwind(AssertUnwindSafe(t.run)) {
            Ok(r) => r,
            Err(p) => Err(p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panicked".into())),
        };
        match result {
            Ok(()) => writeln!(out, "test {} ... ok", t.name)?,
            Err(detail) => {
                failed += 1;
                writeln!(out, "test {} ... FAILED\n  {detail}", t.name)?;
            }
        }
    }
    writeln!(
        out,
        "\ntest result: {}. {} passed; {failed} failed; 0 ignored; 0 measured; {} filtered out",
        if failed == 0 { "ok" } else { "FAILED" },
        selected.len() - failed,
        tests.len() - selected.len(),
    )?;
    Ok(failed == 0)
}

/// The harness entry point: parse `std::env::args`, then list or run.
pub fn main(tests: &[Test]) -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(101);
        }
    };
    let mut stdout = io::stdout().lock();
    let ok = if args.list {
        list(tests, &args, &mut stdout).is_ok()
    } else {
        run(tests, &args, &mut stdout).unwrap_or(false)
    };
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(101)
    }
}
