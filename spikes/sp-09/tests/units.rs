//! SP-09 (#29) unit tests: endpoint, SSE, envelope, signals, contract, tokens, shapes.
use serde_json::{Value, json};
use sp09::contract;
use sp09::endpoint::{EndpointError, GenieOneEndpoint};
use sp09::envelope::{
    EnvelopeKind, RpcError, Signal, SignalRules, TextSource, ToolCallOutcome, ToolResult,
};
use sp09::shape::{
    envelope_facts, known_status, poll_keys, poll_shape, result_shape, shape, tool_names,
};
use sp09::sse::parse_events;
use sp09::token::{
    CliErrorClass, CliTokenSource, RotatingTokens, Token, TokenError, TokenSource, scrub,
};

const GOOD: &str =
    "https://adb-1.azuredatabricks.net/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=1234";

// ---------- G-02 / G-03 endpoint ----------

#[test]
fn g_02_builds_genie_one_url_with_o_param() {
    let ep = GenieOneEndpoint::new("adb-1.azuredatabricks.net", "1234").unwrap();
    assert_eq!(ep.url().as_str(), GOOD);
    assert_eq!(ep.workspace_id(), "1234");
    // A host given with a scheme and a trailing slash gives the same URL.
    let ep2 = GenieOneEndpoint::new("https://adb-1.azuredatabricks.net/", "1234").unwrap();
    assert_eq!(ep2, ep);
    assert_eq!(GenieOneEndpoint::parse(GOOD).unwrap(), ep);
}

#[test]
fn g_02_rejects_missing_or_bad_workspace_id() {
    let no_o = "https://h.cloud.databricks.com/ai-gateway/mcp-services/system.ai.genie_one_mcp";
    assert_eq!(
        GenieOneEndpoint::parse(no_o),
        Err(EndpointError::MissingWorkspaceId)
    );
    assert_eq!(
        GenieOneEndpoint::new("h.cloud.databricks.com", ""),
        Err(EndpointError::MissingWorkspaceId)
    );
    assert_eq!(
        GenieOneEndpoint::new("h.cloud.databricks.com", "12&x=1"),
        Err(EndpointError::MissingWorkspaceId)
    );
    assert_eq!(
        GenieOneEndpoint::new("h/x", "1"),
        Err(EndpointError::BadHost)
    );
}

#[test]
fn g_03_rejects_beta_genie_endpoint() {
    let err =
        GenieOneEndpoint::parse("https://h.cloud.databricks.com/api/2.0/mcp/genie").unwrap_err();
    assert_eq!(err, EndpointError::BetaGenieEndpoint);
    assert!(
        err.to_string().contains("/api/2.0/mcp/genie"),
        "clear error: {err}"
    );
    assert!(err.to_string().contains("2026-11-30"));
    assert_eq!(
        GenieOneEndpoint::parse("https://h.cloud.databricks.com/api/2.0/mcp/genie/?o=1"),
        Err(EndpointError::BetaGenieEndpoint)
    );
}

#[test]
fn g_03_rejects_per_space_genie_mcp_and_genie_one_api_sse() {
    assert_eq!(
        GenieOneEndpoint::parse("https://h.cloud.databricks.com/api/2.0/mcp/genie/01ef1234abcd"),
        Err(EndpointError::PerSpaceGenieMcp)
    );
    assert_eq!(
        GenieOneEndpoint::parse("https://h.cloud.databricks.com/api/2.0/genie/one/responses"),
        Err(EndpointError::GenieOneApiSse)
    );
}

#[test]
fn g_03_rejects_plain_http_and_other_paths() {
    assert_eq!(
        GenieOneEndpoint::parse(&GOOD.replace("https://", "http://")),
        Err(EndpointError::NotHttps)
    );
    assert_eq!(
        GenieOneEndpoint::parse("https://h.cloud.databricks.com/ai-gateway/mcp-services/other?o=1"),
        Err(EndpointError::WrongPath)
    );
    // The loopback test constructor accepts http only for loopback hosts.
    let lb = "http://127.0.0.1:9/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=1";
    assert!(GenieOneEndpoint::parse_loopback_for_tests(lb).is_ok());
    assert_eq!(
        GenieOneEndpoint::parse_loopback_for_tests(&lb.replace("127.0.0.1:9", "evil.example.com")),
        Err(EndpointError::NotHttps)
    );
}

// ---------- SSE ----------

#[test]
fn sp_09_sse_parser_handles_crlf_comments_and_multiline_data() {
    let body = ": ping\r\nevent: message\r\nid: 7\r\ndata: {\"a\":\r\ndata: 1}\r\n\r\ndata: x\n\n";
    let ev = parse_events(body);
    assert_eq!(ev.len(), 2);
    assert_eq!(ev[0].event.as_deref(), Some("message"));
    assert_eq!(ev[0].id.as_deref(), Some("7"));
    assert_eq!(ev[0].data, "{\"a\":\n1}");
    assert_eq!(ev[1].data, "x");
    // A final event with no blank line after it still counts.
    assert_eq!(parse_events("data: tail").len(), 1);
    assert!(parse_events(": only a comment\n\n").is_empty());
}

#[test]
fn sp_09_sse_parser_accepts_bare_cr_line_ends() {
    let ev = parse_events("data: a\rdata: b\r\rid: 3\rdata: c\r");
    assert_eq!(ev.len(), 2);
    assert_eq!(ev[0].data, "a\nb");
    assert_eq!(ev[1].id.as_deref(), Some("3"));
    assert_eq!(ev[1].data, "c");
}

// ---------- envelopes and signals ----------

#[test]
fn sp_09_envelope_distinguishes_json_rpc_error_from_is_error() {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32603, "message": "boom"}});
    let is_err = json!({"jsonrpc": "2.0", "id": 1, "result": {"content": [{"type": "text", "text": "boom"}], "isError": true}});
    let ok = json!({"jsonrpc": "2.0", "id": 1, "result": {"content": [], "structuredContent": {"status": "completed"}}});
    let a = ToolCallOutcome::from_message(&rpc).unwrap();
    let b = ToolCallOutcome::from_message(&is_err).unwrap();
    let c = ToolCallOutcome::from_message(&ok).unwrap();
    assert_eq!(a.kind(), EnvelopeKind::JsonRpcError);
    assert_eq!(
        a,
        ToolCallOutcome::RpcError(RpcError {
            code: -32603,
            message: "boom".into(),
            data: None
        })
    );
    assert_eq!(b.kind(), EnvelopeKind::IsError);
    assert_eq!(c.kind(), EnvelopeKind::Result);
    assert!(ToolCallOutcome::from_message(&json!({"jsonrpc": "2.0", "id": 1})).is_err());
}

#[test]
fn sp_09_tool_result_text_and_structured_fallback() {
    let r = ToolResult {
        raw: json!({"content": [{"type": "text", "text": "{\"status\":\"completed\"}"}]}),
    };
    assert!(!r.is_error());
    assert_eq!(r.text(), "{\"status\":\"completed\"}");
    // No structuredContent: the text is read as a JSON object.
    assert_eq!(r.structured().unwrap()["status"], "completed");
    let r2 = ToolResult {
        raw: json!({"content": [{"type": "text", "text": "plain"}]}),
    };
    assert!(r2.structured().is_none());
}

#[test]
fn g_15a_busy_text_matches_in_every_source_case_insensitive() {
    let rules = SignalRules::builtin();
    assert_eq!(
        rules.classify_http_body("Error: OneChat STREAM CLOSED BEFORE RESPONSE.CREATED event"),
        Some(Signal::ConversationBusy {
            rule_id: "busy-stream-closed".into(),
            source: TextSource::HttpBody
        })
    );
    let ok = ToolCallOutcome::Ok(ToolResult {
        raw: json!({"content": [{"type": "text", "text": "fine"}]}),
    });
    assert_eq!(rules.classify_call(&ok), None);
}

#[test]
fn sp_09_signal_rules_are_data_driven() {
    let custom = r#"{"busy": [{"id": "x", "contains": "another active response", "sources": ["json_rpc_error"]}],
        "clarify": {"status_field": "status", "completed_value": "completed", "answer_field": "final_answer",
        "query_items_field": "query_items", "max_chars": 10, "ends_with": "?", "explicit_fields": ["follow_up_question"]}}"#;
    let rules = SignalRules::from_json(custom).unwrap();
    let rpc = ToolCallOutcome::RpcError(RpcError {
        code: 1,
        message: "There is another active response".into(),
        data: None,
    });
    assert_eq!(
        rules.classify_call(&rpc),
        Some(Signal::ConversationBusy {
            rule_id: "x".into(),
            source: TextSource::JsonRpcError
        })
    );
    // The rule lists only json_rpc_error, so the same text as isError does not match.
    let is_err = ToolCallOutcome::ToolError(ToolResult {
        raw: json!({"content": [{"type": "text", "text": "another active response"}], "isError": true}),
    });
    assert_eq!(rules.classify_call(&is_err), None);
    // max_chars 10 from the data, and an explicit field wins over the heuristic.
    assert_eq!(
        rules.clarify(&json!({"status": "completed", "final_answer": "Which one is it?"})),
        None
    );
    let explicit = rules.clarify(&json!({"status": "completed", "final_answer": "long answer text here.", "follow_up_question": "Which?"}));
    assert_eq!(
        explicit,
        Some(Signal::Clarify {
            question_chars: 6,
            via: "explicit:follow_up_question".into()
        })
    );
}

#[test]
fn g_22_clarify_rule_cases() {
    let rules = SignalRules::builtin();
    let q = "Do you mean revenue or bookings?";
    assert_eq!(
        rules.clarify(&json!({"status": "completed", "final_answer": q, "query_items": []})),
        Some(Signal::Clarify {
            question_chars: q.chars().count(),
            via: "heuristic".into()
        })
    );
    // Trailing white space is ignored.
    assert!(
        rules
            .clarify(&json!({"status": "completed", "final_answer": format!("{q}  \n")}))
            .is_some()
    );
    // Not short (400 chars or more).
    let long = format!("{}?", "a".repeat(399));
    assert_eq!(
        rules.clarify(&json!({"status": "completed", "final_answer": long})),
        None
    );
    let short = format!("{}?", "a".repeat(398));
    assert!(
        rules
            .clarify(&json!({"status": "completed", "final_answer": short}))
            .is_some()
    );
    // Has query items.
    assert_eq!(
        rules.clarify(&json!({"status": "completed", "final_answer": q, "query_items": [{"item_id": "pub_1_output"}]})),
        None
    );
    // Not completed, or no question mark.
    assert_eq!(
        rules.clarify(&json!({"status": "in_progress", "final_answer": q})),
        None
    );
    assert_eq!(
        rules.clarify(&json!({"status": "completed", "final_answer": "Revenue was 4M."})),
        None
    );
    assert_eq!(
        rules.clarify(&json!({"status": "completed", "final_answer": null})),
        None
    );
}

// ---------- contract against fixtures/genie_tools.json ----------

fn fixture() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/genie_tools.json"
    ))
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn sp_09_contract_fixture_lists_required_tools_and_request_shapes_match() {
    let violations = contract::check_tools(&fixture());
    assert!(violations.is_empty(), "{violations:#?}");
}

#[test]
fn sp_09_contract_holds_for_any_tool_order_and_extra_tools() {
    // A capture can list the tools in another order and add view_ask.
    let mut f = fixture();
    let tools = f["tools"].as_array_mut().unwrap();
    tools.reverse();
    tools.insert(
        0,
        json!({"name": "view_ask", "inputSchema": {"type": "object"}}),
    );
    assert!(contract::check_tools(&f).is_empty());
}

#[test]
fn sp_09_contract_detects_missing_tool_and_bad_schema() {
    let mut f = fixture();
    f["tools"]
        .as_array_mut()
        .unwrap()
        .retain(|t| t["name"] != "genie_cancel_response");
    // genie_ask now needs an argument the client does not send.
    let ask = f["tools"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|t| t["name"] == "genie_ask")
        .unwrap();
    ask["inputSchema"]["required"] = json!(["question", "space_id"]);
    let v = contract::check_tools(&f);
    assert!(
        v.iter()
            .any(|x| x.tool == "genie_cancel_response" && x.problem.contains("missing")),
        "{v:#?}"
    );
    assert!(
        v.iter()
            .any(|x| x.tool == "genie_ask" && x.problem.contains("required argument <name len=8>")),
        "{v:#?}"
    );
}

#[test]
fn sp_09_contract_check_args_types_and_unknown_properties() {
    let schema = json!({"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"],
                        "additionalProperties": false});
    assert!(contract::check_args(&schema, &json!({"a": "x"})).is_empty());
    assert!(!contract::check_args(&schema, &json!({"a": 1})).is_empty());
    assert!(!contract::check_args(&schema, &json!({"a": "x", "b": "y"})).is_empty());
    assert!(!contract::check_args(&schema, &json!({})).is_empty());
}

#[test]
fn sp_09_contract_request_builders() {
    assert_eq!(contract::ask_args("q", None), json!({"question": "q"}));
    assert_eq!(
        contract::ask_args("q", Some("c")),
        json!({"question": "q", "conversation_id": "c"})
    );
    assert_eq!(
        contract::poll_args("c", "r"),
        json!({"conversation_id": "c", "response_id": "r"})
    );
    assert_eq!(
        contract::query_result_args("c", "r", "pub_1_output"),
        json!({"conversation_id": "c", "response_id": "r", "item_id": "pub_1_output"})
    );
    assert_eq!(contract::cancel_args("c"), json!({"conversation_id": "c"}));
    assert_eq!(
        contract::ask_meta(Some("w")),
        Some(json!({"warehouse_id": "w"}))
    );
    assert_eq!(contract::ask_meta(None), None);
}

#[test]
fn sp_09_fixture_placeholder_is_marked() {
    let f = fixture();
    if contract::is_placeholder(&f) {
        assert!(f["_note"].as_str().unwrap().contains("PLACEHOLDER"));
        assert!(f["_sources"].as_array().is_some_and(|s| !s.is_empty()));
        eprintln!(
            "NOTE: fixtures/genie_tools.json is still the PLACEHOLDER; replace it with `live capture`."
        );
    }
    assert!(!contract::is_placeholder(&json!({"tools": []})));
}

// ---------- tokens ----------

#[test]
fn sp_09_token_debug_is_redacted() {
    let t = Token::new("dapi-very-secret");
    let d = format!("{t:?}");
    assert!(!d.contains("dapi-very-secret"));
    assert!(d.contains("redacted"));
    assert_eq!(t.expose_secret(), "dapi-very-secret");
}

#[test]
fn sp_09_scrub_removes_token_like_words() {
    let s =
        scrub("refresh failed for eyJhbGciOiJSUzI1NiJ9.e30.sig and dapi0123456789abcdef0123 ok");
    assert!(!s.contains("eyJhbGci") && !s.contains("dapi0123"), "{s}");
    assert!(s.contains("refresh failed") && s.contains("ok"));
    // Embedded forms: key:value, JSON values, key=value, tabs, URLs and IDs.
    let s = scrub(
        "token:eyJabc.def {\"access_token\":\"dose-abc\"} x=eyJhbGciOiJIUzI1NiJ9.a.b Bearer\tdapi123 \
         https://adb-9876543210123456.7.azuredatabricks.net/?o=9876543210123456 h.cloud.databricks.com",
    );
    for leak in [
        "eyJabc",
        "dose-abc",
        "eyJhbGci",
        "dapi123",
        "9876543210123456",
        "azuredatabricks",
        "cloud.databricks",
    ] {
        assert!(!s.contains(leak), "scrub kept {leak:?}: {s}");
    }
    assert!(s.contains("token:") && s.contains("Bearer\t"), "{s}");
}

#[tokio::test]
async fn sp_09_rotating_tokens_double() {
    let t = RotatingTokens::new(&["a", "b"]);
    assert_eq!(t.token(false).await.unwrap().expose_secret(), "a");
    assert_eq!(t.token(true).await.unwrap().expose_secret(), "b");
    assert_eq!(t.token(false).await.unwrap().expose_secret(), "b");
    assert!(matches!(t.token(true).await, Err(TokenError::Exhausted)));
    assert_eq!(t.calls(), vec![false, true, false, true]);
}

#[test]
fn sp_09_cli_token_args_and_profile_check() {
    let src = CliTokenSource::new("databricks", "my-profile").unwrap();
    assert_eq!(
        src.args(false),
        vec![
            "auth",
            "token",
            "--profile",
            "my-profile",
            "--output",
            "json"
        ]
    );
    assert_eq!(src.args(true).last().unwrap(), "--force-refresh");
    for bad in ["", "--host", "-p", "a b", "x\n"] {
        assert!(
            matches!(
                CliTokenSource::new("databricks", bad),
                Err(TokenError::BadProfile)
            ),
            "{bad:?}"
        );
    }
}

#[cfg(unix)]
fn fake_cli(dir: &std::path::Path, script: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("databricks");
    std::fs::write(&p, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[cfg(unix)]
#[tokio::test]
async fn sp_09_cli_token_source_runs_auth_token_with_profile() {
    let dir = tempfile::tempdir().unwrap();
    let args_file = dir.path().join("args.txt");
    let cli = fake_cli(
        dir.path(),
        &format!(
            "echo \"$@\" >> '{}'\nprintf '{{\"access_token\":\"tok-123\",\"token_type\":\"Bearer\",\"expiry\":\"2026-11-15T10:00:00Z\"}}\\n'",
            args_file.display()
        ),
    );
    let src = CliTokenSource::new(&cli, "p1").unwrap();
    assert_eq!(src.token(false).await.unwrap().expose_secret(), "tok-123");
    assert_eq!(src.token(true).await.unwrap().expose_secret(), "tok-123");
    let args = std::fs::read_to_string(&args_file).unwrap();
    let lines: Vec<&str> = args.lines().collect();
    assert_eq!(
        lines,
        vec![
            "auth token --profile p1 --output json",
            "auth token --profile p1 --output json --force-refresh"
        ]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn sp_09_cli_token_errors_do_not_leak_secrets() {
    let dir = tempfile::tempdir().unwrap();
    // The CLI text names the host, the workspace ID, the profile and a token.
    let cli = fake_cli(
        dir.path(),
        "echo 'Error: refresh failed for https://adb-9876543210123456.7.azuredatabricks.net profile secretprof (workspace 9876543210123456): invalid_grant token:eyJhbGciOiJIUzI1NiJ9.leakme.sig' >&2\nexit 3",
    );
    let err = CliTokenSource::new(&cli, "secretprof")
        .unwrap()
        .token(false)
        .await
        .unwrap_err();
    let text = format!("{err} {err:?}");
    assert!(
        matches!(
            err,
            TokenError::Exit {
                code: Some(3),
                class: CliErrorClass::LoginRequired
            }
        ),
        "{text}"
    );
    for leak in [
        "leakme",
        "eyJ",
        "secretprof",
        "9876543210123456",
        "azuredatabricks",
        "invalid_grant",
    ] {
        assert!(!text.contains(leak), "error holds {leak:?}: {text}");
    }
    let cli2 = fake_cli(dir.path(), "echo '{\"token_type\":\"Bearer\"}'");
    assert!(matches!(
        CliTokenSource::new(&cli2, "p1").unwrap().token(false).await,
        Err(TokenError::BadOutput)
    ));
    let missing = CliTokenSource::new(dir.path().join("nope"), "p1").unwrap();
    assert!(matches!(
        missing.token(false).await,
        Err(TokenError::Spawn(_))
    ));
}

#[test]
fn sp_09_cli_error_classes() {
    use sp09::token::classify_cli_error as c;
    assert_eq!(
        c("Error: invalid_grant: refresh token expired"),
        CliErrorClass::LoginRequired
    );
    assert_eq!(
        c("Error: profile foo not found in ~/.databrickscfg"),
        CliErrorClass::ProfileNotFound
    );
    assert_eq!(
        c("M2M authentication is not supported"),
        CliErrorClass::NotU2m
    );
    assert_eq!(c("context deadline exceeded"), CliErrorClass::Timeout);
    assert_eq!(c("boom"), CliErrorClass::Other);
}

// ---------- shapes for the live report ----------

#[test]
fn sp_09_shape_keeps_no_value_at_any_depth() {
    let v = json!({"jsonrpc": "2.0", "result": {"isError": true,
        "content": [{"type": "alice@corp.com", "text": "user data"}],
        "structuredContent": {"status": "completed", "final_answer": "secret answer", "query_items": [
            {"name": "hr.salaries_of_alice_smith", "code": "SELECT ssn FROM hr.people", "type": "alice@corp.com"}],
            "n": 3}}});
    let s = serde_json::to_value(shape(&v)).unwrap();
    let text = s.to_string();
    for leak in [
        "user data",
        "secret answer",
        "hr.salaries_of_alice_smith",
        "SELECT ssn",
        "alice@corp.com",
        "completed",
        "2.0",
        "jsonrpc",
        "structuredContent",
        "final_answer",
        "query_items",
    ] {
        assert!(!text.contains(leak), "shape holds {leak:?}: {text}");
    }
    // Types, lengths and counts only.
    assert_eq!(s["t"], "object");
    assert_eq!(s["keys"], 2);
    assert_eq!(s["fields"][0]["key"]["key_len"], 7);
    assert_eq!(s["fields"][0]["value"], json!({"t": "string", "len": 3}));
}

#[test]
fn sp_09_shape_keeps_key_names_only_at_fixed_protocol_paths() {
    // A result map keyed by table names, SQL and emails, under a known poll key
    // and at the top level of structuredContent.
    let r = json!({"isError": false, "_meta": {"alice@corp.com": 1},
        "content": [{"type": "text", "text": "x"}],
        "structuredContent": {"status": "completed",
            "row_counts": {"hr.salaries_of_alice": 5, "alice@corp.com": 1},
            "query_items": [{"SELECT ssn FROM hr.people": "pub_1_output"}],
            "bob@corp.com": {"finance.revenue_q3": 1}}});
    let text = serde_json::to_string(&result_shape(&r)).unwrap();
    for leak in [
        "alice",
        "hr.salaries",
        "SELECT ssn",
        "bob@corp.com",
        "finance.revenue_q3",
        "row_counts",
        "pub_1_output",
    ] {
        assert!(!text.contains(leak), "result_shape holds {leak:?}: {text}");
    }
    for kept in [
        "\"isError\"",
        "\"_meta\"",
        "\"content\"",
        "\"structuredContent\"",
        "\"status\"",
        "\"query_items\"",
    ] {
        assert!(text.contains(kept), "missing {kept}: {text}");
    }
    let k = poll_keys(&r["structuredContent"]);
    assert_eq!(k.known, vec!["status", "query_items"]);
    assert_eq!(k.other_keys, 2);
    let t =
        serde_json::to_string(&poll_shape(&json!({"alice@corp.com": {"SELECT ssn": 1}}))).unwrap();
    assert!(!t.contains("alice") && !t.contains("SELECT"), "{t}");
}

#[test]
fn sp_09_tool_names_keep_only_known_names() {
    let tl = json!({"tools": [{"name": "genie_ask"}, {"name": "hr.salaries_of_alice"}, {"name": "view_ask"}]});
    let n = tool_names(&tl, &contract::KNOWN_TOOLS);
    assert_eq!(n.known, vec!["genie_ask", "view_ask"]);
    assert_eq!(n.other_count, 1);
    assert_eq!(n.other_name_lens, vec![20]);
    assert!(!serde_json::to_string(&n).unwrap().contains("salaries"));
}

#[test]
fn sp_09_contract_violation_text_has_no_server_names() {
    let mut f = fixture();
    let ask = f["tools"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|t| t["name"] == "genie_ask")
        .unwrap();
    ask["inputSchema"]["required"] = json!(["question", "alice@corp.com"]);
    ask["inputSchema"]["properties"]["question"]["type"] = json!("SELECT ssn");
    let text = serde_json::to_string(&contract::check_tools(&f)).unwrap();
    assert!(
        !text.contains("alice") && !text.contains("SELECT"),
        "{text}"
    );
    assert!(text.contains("<name len=14>"), "{text}");
    // An unknown type word gives no type problem, so no server type text is printed.
}

#[test]
fn sp_09_retry_after_is_parsed_not_echoed() {
    use sp09::transport::{RetryAfter, parse_retry_after};
    assert_eq!(parse_retry_after("120"), RetryAfter::Seconds(120));
    assert_eq!(
        parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"),
        RetryAfter::HttpDate
    );
    assert_eq!(parse_retry_after("alice@corp.com"), RetryAfter::Invalid);
    assert_eq!(parse_retry_after(""), RetryAfter::Invalid);
}

#[test]
fn sp_09_envelope_facts_keep_only_fixed_words_at_fixed_paths() {
    let r = json!({"isError": true, "content": [{"type": "text", "text": "x"}, {"type": "alice@corp.com"}],
        "structuredContent": {"status": "completed", "name": "hr.people"}});
    let f = envelope_facts(&r);
    assert_eq!(f["isError"], true);
    assert_eq!(f["content_types"], json!(["text", "<other>"]));
    assert_eq!(f["structured_status"], "completed");
    assert!(!f.to_string().contains("alice") && !f.to_string().contains("hr.people"));
    assert_eq!(known_status(Some(&json!("SELECT 1"))), json!("<other>"));
    assert_eq!(
        known_status(Some(&json!("in_progress"))),
        json!("in_progress")
    );
}
