//! The Claude and Codex harness adapters behind one AgentHost (RTR-22, RTR-23).
//!
//! An adapter knows the harness-specific parts only: the base-URL variable that points the
//! harness at the loopback model proxy, the proxy path prefix, the AI Gateway route of the
//! outbound hop and the model paths that the proxy may forward. Everything else (sessions,
//! secrets, restarts) is shared in [`super::SharedAgentHost`].

use serde_json::{json, Value};

use super::env::harness_env_from_process;
use super::runtime::{LaunchSpec, RuntimeConfig};
use super::Harness;

/// The harness-specific parts of a runtime.
pub trait HarnessAdapter: Send + Sync {
    /// The harness kind.
    fn harness(&self) -> Harness;
    /// The environment variable that holds the model base URL.
    fn base_url_env(&self) -> &'static str;
    /// The path prefix of this harness on the loopback model proxy.
    fn proxy_prefix(&self) -> &'static str;
    /// The AI Gateway route of the outbound hop (appended to `https://<host>`).
    fn gateway_route(&self) -> &'static str;
    /// The model paths (after the prefix) that the proxy forwards. Every other path is refused.
    fn model_paths(&self) -> &'static [&'static str];

    /// The model base URL that the harness sees, for a proxy at `proxy_base`.
    fn harness_base_url(&self, proxy_base: &str) -> String {
        format!("{proxy_base}{}", self.proxy_prefix())
    }

    /// The `open_session` frame for the runtime. TODO(#111): SP-19 fixes the sidecar protocol.
    /// The host adds the `bottleMcp` (#113) and `genieOneMcp` (#139) endpoints.
    fn open_frame(
        &self,
        session_id: &str,
        model_base_url: &str,
        secret: &str,
        model: Option<&str>,
    ) -> Value {
        let mut f = json!({
            "type": "open_session",
            "sessionId": session_id,
            "harness": self.harness().as_str(),
            "modelEndpoint": { "baseUrl": model_base_url, "secret": secret },
        });
        if let Some(m) = model {
            f["model"] = json!(m);
        }
        f
    }
}

/// The Claude harness (Claude Agent SDK or Claude Code headless).
#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeAdapter;

impl HarnessAdapter for ClaudeAdapter {
    fn harness(&self) -> Harness {
        Harness::Claude
    }
    fn base_url_env(&self) -> &'static str {
        "ANTHROPIC_BASE_URL"
    }
    fn proxy_prefix(&self) -> &'static str {
        "/anthropic"
    }
    fn gateway_route(&self) -> &'static str {
        "/ai-gateway/anthropic"
    }
    fn model_paths(&self) -> &'static [&'static str] {
        &["/v1/messages", "/v1/messages/count_tokens"]
    }
}

/// The Codex harness (Codex SDK or CLI). Its OpenAI-compatible calls go to the Unity Gateway
/// route `/ai-gateway/mlflow/v1` (NFR-21a, D57).
#[derive(Debug, Default, Clone, Copy)]
pub struct CodexAdapter;

impl HarnessAdapter for CodexAdapter {
    fn harness(&self) -> Harness {
        Harness::Codex
    }
    fn base_url_env(&self) -> &'static str {
        "OPENAI_BASE_URL"
    }
    fn proxy_prefix(&self) -> &'static str {
        "/openai/v1"
    }
    fn gateway_route(&self) -> &'static str {
        "/ai-gateway/mlflow/v1"
    }
    fn model_paths(&self) -> &'static [&'static str] {
        &["/chat/completions", "/responses"]
    }
}

/// The adapter of `h`.
pub fn adapter(h: Harness) -> &'static dyn HarnessAdapter {
    match h {
        Harness::Claude => &ClaudeAdapter,
        Harness::Codex => &CodexAdapter,
    }
}

/// The launch spec of the shared runtime of `h`: the configured program and arguments, and a
/// clean environment with only the loopback base URL and the scratch home added (ARC-7, RTR-28).
pub fn launch_spec(h: Harness, config: &RuntimeConfig, proxy_base: &str) -> LaunchSpec {
    let a = adapter(h);
    let mut extra = vec![(a.base_url_env(), a.harness_base_url(proxy_base))];
    if let Some(home) = config.home.as_ref().and_then(|p| p.to_str()) {
        extra.push(("HOME", home.to_owned()));
        extra.push(("USERPROFILE", home.to_owned()));
    }
    let env = harness_env_from_process(&extra);
    LaunchSpec {
        harness: h,
        program: config.program.clone(),
        args: config.args.clone(),
        env,
    }
}
