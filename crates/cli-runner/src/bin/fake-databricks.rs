//! Test fixture: a tiny stand-in for the `databricks` CLI.
//!
//! - `--version` prints `Databricks CLI v<version>`, where the version is read
//!   from `fake-version.txt` next to this binary (default 1.25.0). A file is
//!   used because the clean env drops any variable we could set.
//! - `env-names` prints the name of every env variable it received, one per
//!   line (names only, never values).
//! - `sleep` waits 60 s, so a test can check the child timeout.
//! - `orphan` starts `nap` (a 5 s wait) as a grandchild on the same pipes, then exits.
//! - `family` starts `nap` as a grandchild, writes its pid to `family-pid.txt` next to this
//!   binary, then waits 60 s, so a test can check that a cancel reaches the grandchild.
//! - anything else exits with code 2.

use std::path::PathBuf;

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    match arg.as_str() {
        "--version" => {
            let file = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("fake-version.txt")))
                .unwrap_or_else(|| PathBuf::from("fake-version.txt"));
            let version = std::fs::read_to_string(file).unwrap_or_else(|_| "1.25.0".into());
            println!("Databricks CLI v{}", version.trim());
        }
        "env-names" => {
            for (name, _) in std::env::vars_os() {
                println!("{}", name.to_string_lossy());
            }
        }
        "sleep" => std::thread::sleep(std::time::Duration::from_secs(60)),
        "nap" => std::thread::sleep(std::time::Duration::from_secs(5)),
        "orphan" => {
            let exe = std::env::current_exe().expect("own path");
            // The grandchild inherits stdout and stderr, so the pipes stay open while it naps.
            let _ = std::process::Command::new(exe).arg("nap").spawn();
            println!("parent done");
        }
        "family" => {
            let exe = std::env::current_exe().expect("own path");
            let mut child = std::process::Command::new(&exe)
                .arg("nap")
                .spawn()
                .expect("start nap");
            if let Some(dir) = exe.parent() {
                let _ = std::fs::write(dir.join("family-pid.txt"), child.id().to_string());
            }
            std::thread::sleep(std::time::Duration::from_secs(60));
            let _ = child.wait();
        }
        _ => {
            eprintln!("fake-databricks: unknown command");
            std::process::exit(2);
        }
    }
}
