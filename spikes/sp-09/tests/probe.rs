//! SP-09 (#29): the live probe flow, run against the fake Genie One server.
mod common;

use common::{BUSY_TEXT, Busy, FakeGenie, Reply, WORKSPACE_ID, methods, start};
use serde_json::json;
use sp09::probe::{ProbeOptions, run_probe};
use sp09::token::RotatingTokens;
use sp09::transport::McpClient;
use std::time::Duration;

const T1: &str = "tok-AAA-secret-value";
const T2: &str = "tok-BBB-secret-value";
const ANSWER: &str = "Which quarter and which metric do you mean?";

fn opts() -> ProbeOptions {
    ProbeOptions {
        question: "QUESTION-ONE-private".into(),
        busy_question: "QUESTION-TWO-private".into(),
        clarify_question: "QUESTION-THREE-private".into(),
        warehouse_id: Some("wh-private".into()),
        poll_max: Duration::from_secs(5),
        poll_scale: 0.001,
        print: false,
    }
}

async fn probe_with(busy: Busy, reply: Reply) -> (sp09::probe::ProbeOutput, wiremock::MockServer) {
    probe_cfg(busy, reply, &[T1, T2], |_| {}).await
}

async fn probe_cfg(
    busy: Busy,
    reply: Reply,
    tokens: &[&str],
    extra: impl FnOnce(&mut common::State),
) -> (sp09::probe::ProbeOutput, wiremock::MockServer) {
    let fake = FakeGenie::new(&[T1, T2]);
    fake.with(|s| {
        s.busy = busy;
        s.reply = reply;
        s.poll = json!({"status": "completed", "final_answer": ANSWER, "query_items": null,
                        "progress_steps": null, "deep_link": null});
        extra(s);
    });
    let (server, ep) = start(&fake).await;
    let client = McpClient::new(ep, RotatingTokens::new(tokens));
    (run_probe(&client, &opts()).await, server)
}

#[tokio::test]
async fn sp_09_probe_flow_passes_against_fake_server() {
    for (busy, reply) in [(Busy::IsError, Reply::Json), (Busy::RpcError, Reply::Sse)] {
        let (out, server) = probe_with(busy, reply).await;
        assert!(out.pass, "{busy:?} {reply:?}: {:#}", out.report);
        let steps: Vec<&str> = out.report["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["step"].as_str().unwrap())
            .collect();
        assert_eq!(
            steps,
            vec![
                "initialize",
                "tools_list",
                "ask_1",
                "force_refresh",
                "poll_after_refresh",
                "busy_ask",
                "poll_ask_1_to_end",
                "ask_clarify",
                "poll_clarify_to_end"
            ]
        );
        let pr = &out.report["pass_rule"];
        let want = if busy == Busy::IsError {
            "is_error"
        } else {
            "json_rpc_error"
        };
        assert_eq!(pr["busy_envelope"]["envelope"], want);
        assert_eq!(out.report["steps"][3]["detail"]["token_changed"], true);
        assert_eq!(
            out.report["steps"][8]["detail"]["clarify_signal"]["signal"],
            "clarify"
        );
        // G-15b: the busy ask went out once.
        let asks = methods(&server)
            .await
            .iter()
            .filter(|m| *m == "tools/call:genie_ask")
            .count();
        assert_eq!(asks, 3, "ask_1, busy ask, clarify ask");
        assert_eq!(
            out.tools.as_ref().unwrap()["tools"],
            common::fixture_tools()["tools"]
        );
    }
}

#[tokio::test]
async fn sp_09_probe_report_is_redacted() {
    let (out, _server) = probe_with(Busy::IsError, Reply::Json).await;
    let text = out.report.to_string();
    for secret in [
        T1,
        T2,
        "QUESTION-ONE-private",
        "QUESTION-TWO-private",
        "QUESTION-THREE-private",
        "wh-private",
        ANSWER,
        BUSY_TEXT,
        "sess-1",
        "conv-1",
        "resp-1",
        WORKSPACE_ID,
        "127.0.0.1",
    ] {
        assert!(!text.contains(secret), "report holds {secret:?}");
    }
}

#[tokio::test]
async fn sp_09_probe_reports_busy_not_reproduced() {
    let (out, _server) = probe_with(Busy::None, Reply::Json).await;
    assert!(!out.pass);
    let pr = &out.report["pass_rule"];
    assert_eq!(pr["busy_envelope_recorded"], false);
    assert_eq!(pr["token_refresh_mid_session"], "pass");
    assert_eq!(out.report["steps"][5]["detail"]["busy_reproduced"], false);
}

const SENSITIVE: [&str; 5] = [
    "hr.salaries_of_alice_smith",
    "SELECT ssn FROM hr.people",
    "alice@corp.com",
    "bob@corp.com",
    "finance.revenue_q3",
];

#[tokio::test]
async fn sp_09_probe_report_holds_no_genie_result_values() {
    // Table names, SQL and emails under the keys name, code and type, in a poll
    // result and in an isError result.
    let (out, _server) = probe_cfg(Busy::IsError, Reply::Json, &[T1, T2], |s| {
        s.poll = json!({"status": "completed", "final_answer": ANSWER,
            "query_items": [{"item_id": "pub_1_output", "name": "hr.salaries_of_alice_smith",
                             "code": "SELECT ssn FROM hr.people", "type": "alice@corp.com",
                             "sql": "SELECT ssn FROM hr.people"}],
            "progress_steps": ["Reading finance.revenue_q3"], "deep_link": null});
        s.busy_extra = Some(
            json!({"name": "finance.revenue_q3", "code": "bob@corp.com", "type": "alice@corp.com"}),
        );
    })
    .await;
    let text = out.report.to_string();
    for leak in SENSITIVE {
        assert!(!text.contains(leak), "report holds {leak:?}");
    }
    // The facts that are kept are fixed words.
    assert_eq!(out.report["steps"][6]["detail"]["status"], "completed");
    assert_eq!(
        out.report["steps"][5]["detail"]["facts"]["content_types"],
        json!(["text"])
    );
}

#[tokio::test]
async fn sp_09_probe_refresh_with_same_token_is_not_proven() {
    // --force-refresh gives back the same token: the next call works, but no
    // rotation was shown, so the pass rule must not say PASS (AT-22).
    let (out, _server) = probe_cfg(Busy::IsError, Reply::Json, &[T1, T1], |_| {}).await;
    let pr = &out.report["pass_rule"];
    assert_eq!(pr["token_changed_after_force_refresh"], false);
    assert_eq!(pr["token_refresh_mid_session"], "not_proven");
    assert_eq!(pr["verdict"], "NOT PROVEN");
    assert_eq!(pr["pass"], false);
    assert!(!out.pass);
}

#[cfg(unix)]
#[tokio::test]
async fn sp_09_probe_report_holds_no_cli_text() {
    use std::os::unix::fs::PermissionsExt;
    // A fake CLI that gives a token 4 times, then fails with text that names the
    // host, the workspace ID, the profile and a token-shaped key:value.
    let dir = tempfile::tempdir().unwrap();
    let count = dir.path().join("count");
    let cli = dir.path().join("databricks");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\nn=$(cat '{c}' 2>/dev/null || echo 0); n=$((n+1)); echo $n > '{c}'\n\
             if [ $n -le 4 ]; then printf '{{\"access_token\":\"{T1}\"}}\\n'; exit 0; fi\n\
             echo 'Error: refresh failed for https://adb-9876543210123456.7.azuredatabricks.net profile secretprof workspace 9876543210123456: invalid_grant token:eyJhbGciOiJIUzI1NiJ9.leak.sig' >&2\nexit 1\n",
            c = count.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let fake = FakeGenie::new(&[T1]);
    let (_server, ep) = start(&fake).await;
    let client = McpClient::new(
        ep,
        sp09::token::CliTokenSource::new(&cli, "secretprof").unwrap(),
    );
    let out = run_probe(&client, &opts()).await;
    let text = out.report.to_string();
    for leak in [
        "secretprof",
        "9876543210123456",
        "azuredatabricks",
        "eyJ",
        "leak.sig",
        "invalid_grant",
        T1,
    ] {
        assert!(!text.contains(leak), "report holds {leak:?}: {text}");
    }
    let refresh = &out.report["steps"][3];
    assert_eq!(refresh["step"], "force_refresh");
    assert_eq!(refresh["ok"], false);
    assert!(
        refresh["detail"]["error"]
            .as_str()
            .unwrap()
            .contains("class login_required")
    );
    assert!(!out.pass);
}

const KEY_LEAKS: [&str; 7] = [
    "hr.salaries_of_alice",
    "alice@corp.com",
    "bob@corp.com",
    "SELECT ssn FROM hr.people",
    "finance.revenue_q3",
    "9876543210123456",
    "azuredatabricks",
];

fn assert_no_key_leaks(out: &sp09::probe::ProbeOutput) {
    let text = out.report.to_string();
    for leak in KEY_LEAKS {
        assert!(!text.contains(leak), "report holds {leak:?}: {text}");
    }
}

#[tokio::test]
async fn sp_09_probe_report_holds_no_result_keys_or_server_text() {
    // Poll structuredContent keyed by a table name and an email; an isError
    // _meta keyed by an email; a host in serverInfo.name; an extra tool name.
    let (out, _server) = probe_cfg(Busy::IsError, Reply::Json, &[T1, T2], |s| {
        s.poll = json!({"status": "completed", "final_answer": ANSWER,
            "row_counts": {"hr.salaries_of_alice": 5, "alice@corp.com": 1},
            "bob@corp.com": {"finance.revenue_q3": 1},
            "query_items": [{"SELECT ssn FROM hr.people": "x"}]});
        s.busy_meta = Some(json!({"alice@corp.com": 1}));
        s.server_name = "adb-9876543210123456.7.azuredatabricks.net".into();
        s.extra_tool =
            Some(json!({"name": "hr.salaries_of_alice", "inputSchema": {"type": "object"}}));
    })
    .await;
    assert_no_key_leaks(&out);
    let tl = &out.report["steps"][1]["detail"]["tool_names"];
    assert_eq!(tl["other_count"], 1);
    assert_eq!(out.report["steps"][6]["detail"]["keys"]["other_keys"], 2);
}

#[tokio::test]
async fn sp_09_probe_report_holds_no_keys_from_text_json_poll() {
    // No structuredContent: the poll is JSON text (the structured() fallback).
    let (out, _server) = probe_cfg(Busy::IsError, Reply::Json, &[T1, T2], |s| {
        s.poll_as_text = true;
        s.poll = json!({"status": "completed", "alice@corp.com": {"SELECT ssn FROM hr.people": 1}});
    })
    .await;
    assert_no_key_leaks(&out);
    assert_eq!(out.report["steps"][6]["detail"]["status"], "completed");
}

#[tokio::test]
async fn sp_09_probe_report_holds_no_keys_from_json_rpc_error_data() {
    let (out, _server) = probe_cfg(Busy::RpcError, Reply::Json, &[T1, T2], |s| {
        s.busy_rpc_data =
            Some(json!({"alice@corp.com": "hr.salaries_of_alice", "bob@corp.com": [1]}));
    })
    .await;
    assert_no_key_leaks(&out);
    assert_eq!(
        out.report["pass_rule"]["busy_envelope"]["envelope"],
        "json_rpc_error"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn sp_09_probe_failed_force_refresh_is_fail_not_not_proven() {
    use std::os::unix::fs::PermissionsExt;
    // The CLI gives a token, but `--force-refresh` fails. The next call works with
    // the cached token, so nothing about rotation was shown: the item is "fail".
    let dir = tempfile::tempdir().unwrap();
    let cli = dir.path().join("databricks");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\ncase \"$*\" in *--force-refresh*) echo 'refresh failed' >&2; exit 1;; esac\n\
             printf '{{\"access_token\":\"{T1}\"}}\\n'\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let fake = FakeGenie::new(&[T1]);
    fake.with(|s| {
        s.busy = Busy::IsError;
        s.poll = json!({"status": "completed", "final_answer": ANSWER});
    });
    let (_server, ep) = start(&fake).await;
    let client = McpClient::new(ep, sp09::token::CliTokenSource::new(&cli, "p1").unwrap());
    let out = run_probe(&client, &opts()).await;
    let pr = &out.report["pass_rule"];
    assert_eq!(out.report["steps"][4]["ok"], true, "the next call worked");
    assert_eq!(
        pr["token_changed_after_force_refresh"],
        serde_json::Value::Null
    );
    assert_eq!(pr["token_refresh_mid_session"], "fail");
    assert_eq!(pr["verdict"], "NOT YET");
    assert!(!out.pass);
}
