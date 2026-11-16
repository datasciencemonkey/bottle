//! The SP-09 live probe flow, generic over the token source so tests can run it
//! against the fake server. The report holds status codes, fixed protocol words
//! (envelope kind, content types, poll status from a fixed list, JSON-RPC code)
//! and typed value-free shapes (types, lengths, counts; key names only from fixed
//! protocol lists). Default deny: no server `Value` goes in. It holds no token,
//! question, answer, result value, host, workspace ID, conversation ID, session ID
//! or CLI text.
use crate::contract;
use crate::envelope::{SignalRules, ToolCallOutcome};
use crate::shape::{
    envelope_facts, known_status, poll_keys, poll_shape, result_shape, shape, tool_names,
};
use crate::token::{TokenError, TokenSource};
use crate::transport::{McpClient, McpError};
use serde_json::{Value, json};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DEFAULT_QUESTION: &str =
    "Which 5 tables that I can query have the most rows? Show each table name and its row count.";
pub const DEFAULT_BUSY_QUESTION: &str = "Now show the same for the smallest 5 tables.";
pub const DEFAULT_CLARIFY_QUESTION: &str = "Show me the numbers for last quarter.";

pub struct ProbeOptions {
    pub question: String,
    pub busy_question: String,
    pub clarify_question: String,
    pub warehouse_id: Option<String>,
    /// Give up a poll loop after this time.
    pub poll_max: Duration,
    /// Poll delays are 2, 3, 5 s then x1.5 up to 15 s, times this factor (1.0 live).
    pub poll_scale: f64,
    /// Print one line per step to stdout.
    pub print: bool,
}

impl Default for ProbeOptions {
    fn default() -> Self {
        ProbeOptions {
            question: DEFAULT_QUESTION.into(),
            busy_question: DEFAULT_BUSY_QUESTION.into(),
            clarify_question: DEFAULT_CLARIFY_QUESTION.into(),
            warehouse_id: None,
            poll_max: Duration::from_secs(240),
            poll_scale: 1.0,
            print: true,
        }
    }
}

pub struct ProbeOutput {
    /// The redacted report (write it to the path the person chose).
    pub report: Value,
    /// The live `tools/list` result, for `--capture`.
    pub tools: Option<Value>,
    /// `initialize` facts, for the capture header.
    pub info: Value,
    pub pass: bool,
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The fixture format: the `tools/list` result plus a `_capture` header.
///
/// Only tools on `KNOWN_TOOLS` are kept, and of each only `name` (from the
/// fixed list), `description` and `inputSchema`. `_meta`, `annotations`,
/// `outputSchema` and every other tool are dropped. The description is server
/// text and can name a host, space, catalog, table or person: the person must
/// read it before committing or attaching the file (docs/spikes/SP-09.md).
pub fn capture_doc(tools: &Value, info: &Value) -> Value {
    let all: Vec<Value> = tools
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let kept: Vec<Value> = all
        .iter()
        .filter_map(|t| {
            let name = t.get("name").and_then(Value::as_str)?;
            let name = contract::KNOWN_TOOLS.iter().copied().find(|k| *k == name)?;
            Some(json!({
                "name": name,
                "description": t.get("description").cloned().unwrap_or(Value::Null),
                "inputSchema": t.get("inputSchema").cloned().unwrap_or(Value::Null),
            }))
        })
        .collect();
    json!({
        "_capture": {
            "source": "live capture (spikes/sp-09 `live capture` or `live probe --capture`)",
            "captured_at_unix": now_unix(),
            "protocolVersion": info.get("protocol_version"),
            "server_name_len": info.get("server_name_len"),
            "dropped_tools": all.len() - kept.len(),
            "review": "Read every description before you commit or attach this file. Remove anything that names your workspace, host, space, catalog, table or a person.",
        },
        "tools": kept,
    })
}

/// A JSON-RPC error code only if it is in the reserved range or small.
pub fn safe_rpc_code(code: i64) -> Value {
    if (-32768..=-32000).contains(&code) || (-999..=999).contains(&code) {
        json!(code)
    } else {
        json!("out_of_range")
    }
}

struct Run<'a, T> {
    client: &'a McpClient<T>,
    rules: SignalRules,
    steps: Vec<Value>,
    mark: usize,
    print: bool,
}

impl<T: TokenSource> Run<'_, T> {
    /// Record one step with the HTTP exchanges since the last step.
    fn step(&mut self, name: &str, ok: bool, detail: Value) {
        let ex = self.client.exchanges();
        let new = &ex[self.mark.min(ex.len())..];
        let http: Vec<Value> = new
            .iter()
            .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
            .collect();
        let statuses: Vec<u16> = new.iter().map(|e| e.http_status).collect();
        self.mark = ex.len();
        if self.print {
            let tag = if ok { "ok" } else { "FAIL" };
            println!("[{tag}] {name}: http {statuses:?} {detail}");
        }
        self.steps
            .push(json!({"step": name, "ok": ok, "detail": detail, "http": http}));
    }

    fn outcome_detail(&self, out: &ToolCallOutcome) -> Value {
        let signal = self.rules.classify_call(out);
        match out {
            ToolCallOutcome::RpcError(e) => json!({
                "envelope": out.kind(),
                "rpc_code": safe_rpc_code(e.code),
                "rpc_message_chars": e.message.chars().count(),
                "rpc_data_shape": e.data.as_ref().map(shape),
                "signal": signal,
            }),
            ToolCallOutcome::ToolError(r) | ToolCallOutcome::Ok(r) => json!({
                "envelope": out.kind(),
                "facts": envelope_facts(&r.raw),
                "text_chars": r.text().chars().count(),
                "structured_from": if r.raw.get("structuredContent").is_some() { "structuredContent" } else { "text" },
                "result_shape": result_shape(&r.raw),
                "signal": signal,
            }),
        }
    }

    /// Poll until a terminal status or the time cap. One poll at a time (G-07b).
    async fn poll_to_end(
        &mut self,
        cid: &str,
        rid: &str,
        cap: Duration,
        scale: f64,
    ) -> (Option<Value>, usize, String) {
        let start = Instant::now();
        let schedule = [2.0, 3.0, 5.0];
        let mut delay = schedule[0];
        let mut n = 0usize;
        let mut last = None;
        let mut err = String::new();
        while start.elapsed() < cap {
            tokio::time::sleep(Duration::from_secs_f64(delay * scale)).await;
            n += 1;
            match self
                .client
                .call_tool("genie_poll_response", contract::poll_args(cid, rid), None)
                .await
            {
                Ok(ToolCallOutcome::Ok(r)) => {
                    let sc = r.structured();
                    let status = sc
                        .as_ref()
                        .and_then(|s| s.get("status"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    // G-18a: `unknown` is not terminal.
                    let done = !matches!(status, "in_progress" | "unknown" | "");
                    last = sc;
                    if done {
                        break;
                    }
                }
                Ok(other) => {
                    err = format!("poll envelope {:?}", other.kind());
                    break;
                }
                Err(e) => {
                    err = e.to_string();
                    break;
                }
            }
            delay = schedule.get(n).copied().unwrap_or((delay * 1.5).min(15.0));
        }
        if last.is_none() && err.is_empty() {
            err = "no poll result before the time cap".into();
        }
        (last, n, err)
    }
}

fn ids(out: &ToolCallOutcome) -> Option<(String, String)> {
    let ToolCallOutcome::Ok(r) = out else {
        return None;
    };
    let sc = r.structured()?;
    Some((
        sc.get("conversation_id")?.as_str()?.to_string(),
        sc.get("response_id")?.as_str()?.to_string(),
    ))
}

fn poll_summary(rules: &SignalRules, sc: &Option<Value>) -> Value {
    let Some(sc) = sc else {
        return json!({"structured": null});
    };
    let answer = sc.get("final_answer").and_then(Value::as_str).unwrap_or("");
    json!({
        "status": known_status(sc.get("status")),
        "keys": poll_keys(sc),
        "final_answer_chars": answer.chars().count(),
        "final_answer_ends_with_question_mark": answer.trim_end().ends_with('?'),
        "query_items_count": sc.get("query_items").and_then(Value::as_array).map(Vec::len),
        "progress_steps_is_null": sc.get("progress_steps").is_some_and(Value::is_null),
        "deep_link_is_null": sc.get("deep_link").is_none_or(Value::is_null),
        "narration_instruction_present": sc.get("narration_instruction").is_some_and(|v| !v.is_null()),
        "clarify_signal": rules.clarify(sc),
        "shape": poll_shape(sc),
    })
}

/// Run the probe steps in order: initialize, tools_list, ask_1, force_refresh,
/// poll_after_refresh, busy_ask, poll_ask_1_to_end, ask_clarify, poll_clarify_to_end.
pub async fn run_probe<T: TokenSource>(client: &McpClient<T>, opts: &ProbeOptions) -> ProbeOutput {
    let mut run = Run {
        client,
        rules: SignalRules::builtin(),
        steps: Vec::new(),
        mark: 0,
        print: opts.print,
    };
    let meta = contract::ask_meta(opts.warehouse_id.as_deref());
    let mut out = ProbeOutput {
        report: Value::Null,
        tools: None,
        info: Value::Null,
        pass: false,
    };
    let mut pass_refresh = false;
    let mut token_changed: Option<bool> = None;
    let mut busy_envelope: Option<Value> = None;
    let mut contract_ok = false;

    'flow: {
        // 1. initialize + notifications/initialized
        match client.initialize().await {
            Ok(i) => {
                out.info = serde_json::to_value(&i).unwrap_or(Value::Null);
                let ok = matches!(i.initialized_status, 200 | 202);
                run.step("initialize", ok, out.info.clone());
            }
            Err(e) => {
                run.step("initialize", false, json!({"error": e.to_string()}));
                break 'flow;
            }
        }

        // 2. tools/list + contract against the live list
        match client.list_tools().await {
            Ok(tools) => {
                let names = tool_names(&tools, &contract::KNOWN_TOOLS);
                let violations = contract::check_tools(&tools);
                contract_ok = violations.is_empty();
                run.step(
                    "tools_list",
                    contract_ok,
                    json!({"tool_names": names, "contract_violations": violations}),
                );
                out.tools = Some(tools);
            }
            Err(e) => run.step("tools_list", false, json!({"error": e.to_string()})),
        }

        // 3. first ask
        let (cid, rid) = match client
            .call_tool(
                "genie_ask",
                contract::ask_args(&opts.question, None),
                meta.clone(),
            )
            .await
        {
            Ok(o) => {
                let d = run.outcome_detail(&o);
                let got = ids(&o);
                run.step("ask_1", got.is_some(), d);
                match got {
                    Some(x) => x,
                    None => break 'flow,
                }
            }
            Err(e) => {
                run.step("ask_1", false, json!({"error": e.to_string()}));
                break 'flow;
            }
        };

        // 4. forced token refresh in the middle of the session (CLI --force-refresh)
        let session_before = client.session_id();
        let refresh = async {
            let before = client.tokens().token(false).await?;
            let after = client.tokens().token(true).await?;
            Ok::<bool, TokenError>(before.expose_secret() != after.expose_secret())
        }
        .await;
        match refresh {
            Ok(changed) => {
                token_changed = Some(changed);
                // AT-22: an unchanged token proves nothing about rotation.
                run.step("force_refresh", changed, json!({"token_changed": changed}))
            }
            Err(e) => run.step("force_refresh", false, json!({"error": e.to_string()})),
        }

        // 5. the next call: same session, new token, no new transport (G-04d)
        let mut first_status = Value::Null;
        let poll = client
            .call_tool("genie_poll_response", contract::poll_args(&cid, &rid), None)
            .await;
        let same_session = client.session_id() == session_before;
        match &poll {
            Ok(o) => {
                let mut d = run.outcome_detail(o);
                if let ToolCallOutcome::Ok(r) = o {
                    first_status =
                        known_status(r.structured().as_ref().and_then(|s| s.get("status")));
                }
                d["session_unchanged"] = json!(same_session);
                d["status"] = first_status.clone();
                pass_refresh = matches!(o, ToolCallOutcome::Ok(_)) && same_session;
                run.step("poll_after_refresh", pass_refresh, d);
            }
            Err(e) => run.step(
                "poll_after_refresh",
                false,
                json!({"error": e.to_string(), "session_unchanged": same_session}),
            ),
        }

        // 6. second ask to the same busy conversation: sent once, never retried (G-15b)
        let busy = client
            .call_tool(
                "genie_ask",
                contract::ask_args(&opts.busy_question, Some(&cid)),
                meta.clone(),
            )
            .await;
        let mut d = match &busy {
            Ok(o) => run.outcome_detail(o),
            Err(McpError::Http {
                status,
                body_signal,
                rpc_error,
            }) => json!({
                "envelope": "http_error", "http_status": status, "signal": body_signal,
                "rpc_code": rpc_error.as_ref().map(|e| safe_rpc_code(e.code)),
            }),
            Err(e) => json!({"envelope": "client_error", "error": e.to_string()}),
        };
        d["first_ask_status_before"] = first_status.clone();
        let reproduced = d.get("signal").is_some_and(|s| !s.is_null());
        d["busy_reproduced"] = json!(reproduced);
        if !reproduced && first_status != json!("in_progress") {
            d["note"] =
                json!("the first ask was no longer in_progress; rerun with a heavier --question");
        }
        busy_envelope = reproduced.then(|| d.clone());
        run.step("busy_ask", reproduced, d);

        // 7. finish the first ask
        let (last, n, err) = run
            .poll_to_end(&cid, &rid, opts.poll_max, opts.poll_scale)
            .await;
        let mut d = poll_summary(&run.rules, &last);
        d["polls"] = json!(n);
        d["error"] = json!(err);
        run.step("poll_ask_1_to_end", err.is_empty(), d);

        // 8. a question that should make Genie ask for clarification (new conversation)
        match client
            .call_tool(
                "genie_ask",
                contract::ask_args(&opts.clarify_question, None),
                meta.clone(),
            )
            .await
        {
            Ok(o) => {
                let d = run.outcome_detail(&o);
                let got = ids(&o);
                run.step("ask_clarify", got.is_some(), d);
                if let Some((c2, r2)) = got {
                    let (last, n, err) = run
                        .poll_to_end(&c2, &r2, opts.poll_max, opts.poll_scale)
                        .await;
                    let mut d = poll_summary(&run.rules, &last);
                    d["polls"] = json!(n);
                    d["error"] = json!(err);
                    let clar = d.get("clarify_signal").is_some_and(|s| !s.is_null());
                    run.step("poll_clarify_to_end", clar, d);
                }
            }
            Err(e) => run.step("ask_clarify", false, json!({"error": e.to_string()})),
        }
    }

    // Refresh item: pass only if the next call worked on the same session AND the
    // token really changed; an OK call with the same token is "not_proven".
    let refresh_verdict = match (pass_refresh, token_changed) {
        (true, Some(true)) => "pass",
        // The next call worked, but the same token came back: rotation not shown.
        (true, Some(false)) => "not_proven",
        // `--force-refresh` itself failed, or the next call failed.
        _ => "fail",
    };
    out.pass = refresh_verdict == "pass" && busy_envelope.is_some() && contract_ok;
    let verdict = if out.pass {
        "PASS"
    } else if refresh_verdict == "not_proven" {
        "NOT PROVEN"
    } else {
        "NOT YET"
    };
    out.report = json!({
        "spike": "SP-09",
        "issue": 29,
        "created_unix": now_unix(),
        "harness_version": env!("CARGO_PKG_VERSION"),
        "redaction": "default deny: no token, question, answer, result value, result key, CLI text, server text, host, workspace ID, profile, conversation ID or session ID is in this file; shapes keep types, lengths and counts, and key names only from fixed protocol lists",
        "pass_rule": {
            "token_refresh_mid_session": refresh_verdict,
            "token_changed_after_force_refresh": token_changed,
            "busy_envelope_recorded": busy_envelope.is_some(),
            "busy_envelope": busy_envelope,
            "contract_matches_live_tools_list": contract_ok,
            "pass": out.pass,
            "verdict": verdict,
        },
        "steps": run.steps,
    });
    out
}
