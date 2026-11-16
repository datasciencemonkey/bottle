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
    let fake = FakeGenie::new(&[T1, T2]);
    fake.with(|s| {
        s.busy = busy;
        s.reply = reply;
        s.poll = json!({"status": "completed", "final_answer": ANSWER, "query_items": null,
                        "progress_steps": null, "deep_link": null});
    });
    let (server, ep) = start(&fake).await;
    let client = McpClient::new(ep, RotatingTokens::new(&[T1, T2]));
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
    assert_eq!(pr["token_refresh_mid_session_works"], true);
    assert_eq!(out.report["steps"][5]["detail"]["busy_reproduced"], false);
}
