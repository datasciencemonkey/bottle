//! The spawned harness process gets a clean environment (#112, ARC-7, RTR-28).
//!
//! This test binary sets secret-looking variables in its own environment, builds the harness
//! command the way R16 AgentHost does, starts a REAL child process that prints its environment,
//! and checks that no token and no inherited secret reaches the child. It is the only test in this
//! binary, so changing the process environment races with nothing.

use std::process::Stdio;

use mog_core::agent_host::adapter::launch_spec;
use mog_core::agent_host::runtime::{harness_command, RuntimeConfig};
use mog_core::agent_host::Harness;

const SECRETS: [(&str, &str); 9] = [
    ("DATABRICKS_TOKEN", "dapi-FAKE-INHERITED-TOKEN-1"),
    ("DATABRICKS_CLIENT_SECRET", "fake-client-secret-2"),
    ("DATABRICKS_HOST", "https://fake-inherited.cloud.databricks.com"),
    ("ANTHROPIC_API_KEY", "sk-ant-fake-3"),
    ("ANTHROPIC_AUTH_TOKEN", "fake-auth-4"),
    ("OPENAI_API_KEY", "sk-fake-5"),
    ("AWS_SECRET_ACCESS_KEY", "fake-aws-6"),
    ("GITHUB_TOKEN", "ghp_fake7"),
    ("MY_APP_PASSWORD", "fake-pass-8"),
];

#[cfg(unix)]
fn env_printer() -> RuntimeConfig {
    RuntimeConfig::new("env", Vec::new())
}

#[cfg(windows)]
fn env_printer() -> RuntimeConfig {
    RuntimeConfig::new("cmd", vec!["/C".into(), "set".into()])
}

fn child_env(h: Harness) -> Vec<(String, String)> {
    let spec = launch_spec(h, &env_printer(), "http://127.0.0.1:45678");
    let out = harness_command(&spec)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .expect("env printer runs");
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

#[test]
fn rtr_28_arc_7_spawned_harness_env_has_no_token_and_no_inherited_secrets() {
    for (k, v) in SECRETS {
        // The only test in this binary: no other thread reads the environment.
        std::env::set_var(k, v);
    }

    for (h, base_env, base) in [
        (
            Harness::Claude,
            "ANTHROPIC_BASE_URL",
            "http://127.0.0.1:45678/anthropic",
        ),
        (
            Harness::Codex,
            "OPENAI_BASE_URL",
            "http://127.0.0.1:45678/openai/v1",
        ),
    ] {
        let env = child_env(h);
        assert!(!env.is_empty(), "the printer printed its env");
        for (k, v) in SECRETS {
            assert!(
                env.iter().all(|(ek, _)| !ek.eq_ignore_ascii_case(k)),
                "{k} must not reach the {h:?} harness"
            );
            assert!(
                env.iter().all(|(_, ev)| !ev.contains(v)),
                "the value of {k} must not reach the {h:?} harness"
            );
        }
        for (k, _) in &env {
            let up = k.to_ascii_uppercase();
            assert!(
                !(up.contains("TOKEN") || up.contains("SECRET") || up.contains("PASSWORD")),
                "{k} looks like a secret name"
            );
        }
        let got = env
            .iter()
            .find(|(k, _)| k == base_env)
            .map(|(_, v)| v.as_str());
        assert_eq!(got, Some(base), "{h:?} gets only the loopback base URL");
    }
}
