//! Live harness for SP-09 (#29). A person runs it against a real workspace with a
//! U2M profile they choose. It prints status codes and envelope shapes only: no
//! token, no question text, no answer text, no host, no workspace ID.
//!
//!   live probe   --profile <your-profile> --workspace-id <id> --host <host> --report <path>
//!   live capture --profile <your-profile> --workspace-id <id> --host <host> --out fixtures/genie_tools.json
use clap::{Args, Parser, Subcommand};
use serde_json::Value;
use sp09::contract;
use sp09::endpoint::GenieOneEndpoint;
use sp09::probe::{self, ProbeOptions};
use sp09::token::CliTokenSource;
use sp09::transport::McpClient;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "live", about = "SP-09 (#29) live harness for Genie One MCP")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args)]
struct Target {
    /// Databricks CLI profile with U2M OAuth (you choose it).
    #[arg(long)]
    profile: String,
    /// Workspace ID (the `o=` value).
    #[arg(long)]
    workspace_id: String,
    /// Workspace host, for example adb-123.4.azuredatabricks.net.
    #[arg(long)]
    host: String,
    /// Path to the Databricks CLI (needs `auth token --force-refresh`).
    #[arg(long, default_value = "databricks")]
    cli: PathBuf,
}

#[derive(Subcommand)]
enum Cmd {
    /// initialize, tools/list, ask, forced refresh + next call, busy ask, clarify ask.
    Probe {
        #[command(flatten)]
        target: Target,
        /// Where to write the JSON report (redacted).
        #[arg(long)]
        report: PathBuf,
        /// Optional SQL warehouse ID, sent in `_meta.warehouse_id` (G-06b).
        #[arg(long)]
        warehouse_id: Option<String>,
        #[arg(long, default_value = probe::DEFAULT_QUESTION)]
        question: String,
        #[arg(long, default_value = probe::DEFAULT_BUSY_QUESTION)]
        busy_question: String,
        #[arg(long, default_value = probe::DEFAULT_CLARIFY_QUESTION)]
        clarify_question: String,
        /// Give up a poll loop after this many seconds.
        #[arg(long, default_value_t = 240)]
        poll_max_secs: u64,
        /// Also write the real tools/list result to this path (fixture format).
        #[arg(long)]
        capture: Option<PathBuf>,
    },
    /// initialize + tools/list, then write the result in the fixture format.
    Capture {
        #[command(flatten)]
        target: Target,
        #[arg(long)]
        out: PathBuf,
    },
}

fn connect(t: &Target) -> Result<McpClient<CliTokenSource>, String> {
    let ep = GenieOneEndpoint::new(&t.host, &t.workspace_id).map_err(|e| e.to_string())?;
    let tokens = CliTokenSource::new(&t.cli, &t.profile).map_err(|e| e.to_string())?;
    Ok(McpClient::new(ep, tokens))
}

fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    std::fs::write(path, text + "\n").map_err(|e| format!("cannot write {}: {e}", path.display()))
}

async fn run(cmd: Cmd) -> Result<bool, String> {
    match cmd {
        Cmd::Probe {
            target,
            report,
            warehouse_id,
            question,
            busy_question,
            clarify_question,
            poll_max_secs,
            capture,
        } => {
            let client = connect(&target)?;
            let opts = ProbeOptions {
                question,
                busy_question,
                clarify_question,
                warehouse_id,
                poll_max: Duration::from_secs(poll_max_secs),
                ..ProbeOptions::default()
            };
            let out = probe::run_probe(&client, &opts).await;
            if let (Some(path), Some(tools)) = (&capture, &out.tools) {
                write_json(path, &probe::capture_doc(tools, &out.info))?;
                println!("captured tools/list to {}", path.display());
            }
            write_json(&report, &out.report)?;
            let pr = &out.report["pass_rule"];
            println!(
                "\nSP-09 pass rule: refresh_mid_session={} busy_envelope_recorded={} contract={} => {}",
                pr["token_refresh_mid_session_works"],
                pr["busy_envelope_recorded"],
                pr["contract_matches_live_tools_list"],
                if out.pass { "PASS" } else { "NOT YET" }
            );
            println!("report written to {}", report.display());
            Ok(out.pass)
        }
        Cmd::Capture { target, out } => {
            let client = connect(&target)?;
            let info = client.initialize().await.map_err(|e| e.to_string())?;
            let tools = client.list_tools().await.map_err(|e| e.to_string())?;
            let violations = contract::check_tools(&tools);
            let info = serde_json::to_value(&info).unwrap_or(Value::Null);
            write_json(&out, &probe::capture_doc(&tools, &info))?;
            println!(
                "wrote {} tools to {} (contract violations: {})",
                tools["tools"].as_array().map_or(0, Vec::len),
                out.display(),
                serde_json::to_string(&violations).unwrap_or_default()
            );
            Ok(violations.is_empty())
        }
    }
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run(Cli::parse().cmd).await {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::from(2),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::from(1)
        }
    }
}
