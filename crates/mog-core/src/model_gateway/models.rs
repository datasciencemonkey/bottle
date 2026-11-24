//! The orchestrator model list and status (ADR 0122).
//!
//! No read-only route lists the chat models of a profile's AI Gateway: R7 has only
//! `mlflow.chat_completions` and `mlflow.embeddings` on the gateway (`http_transport::ROUTES`),
//! and the recording routes add no list call. So [`list_orchestrator_models`] returns a curated
//! list of known `system.ai.*` chat models. A model that the gateway does not serve gives
//! `model_not_found` at the first call, and the status goes to `no_model`.

use serde::{Deserialize, Serialize};

use super::client::{MODEL_NOT_FOUND, OFFLINE, SIGNED_OUT, TIMEOUT};

/// One model of the picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// The model name on the gateway, for example `system.ai.claude-haiku-5-5`.
    pub id: String,
    /// The picker label.
    pub label: String,
    /// The model supports tool calls (the ask loop needs this).
    pub tools: bool,
    /// The model has a passing eval record (NFR-22a).
    pub tested: bool,
}

/// Known chat models: `(id, label, supports tools)`. This is the capabilities table.
const KNOWN: &[(&str, &str, bool)] = &[
    ("system.ai.claude-haiku-5-5", "Claude Haiku 5.5", true),
    ("system.ai.claude-sonnet-5-5", "Claude Sonnet 5.5", true),
    ("system.ai.claude-opus-5-5", "Claude Opus 5.5", true),
    ("system.ai.gpt-oss-20b", "GPT OSS 20B", true),
    ("system.ai.gpt-oss-120b", "GPT OSS 120B", true),
    (
        "system.ai.meta-llama-3-1-8b-instruct",
        "Llama 3.1 8B Instruct",
        true,
    ),
    ("system.ai.gpt-5-4-mini", "GPT 5.4 mini", true),
    ("system.ai.gemini-3-8-flash", "Gemini 3 Flash", true),
];

/// The eval record: the models that have an eval run. Both are `false` until the eval runs; a
/// model passes the day its entry here turns `true`.
const EVAL_RECORD: &[(&str, bool)] = &[
    ("system.ai.claude-haiku-5-5", false),
    ("system.ai.gpt-oss-20b", false),
];

/// The chat models that `profile`'s gateway is expected to serve, default first. No call goes to
/// the network. `profile` is kept for the day a list route exists.
pub fn list_orchestrator_models(_profile: &str) -> Vec<ModelInfo> {
    KNOWN
        .iter()
        .map(|(id, label, tools)| ModelInfo {
            id: (*id).to_owned(),
            label: (*label).to_owned(),
            tools: *tools,
            tested: EVAL_RECORD.iter().any(|(m, pass)| m == id && *pass),
        })
        .collect()
}

/// Whether the orchestrator can answer now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestratorStatus {
    /// A home profile and a model are set, and the last call did not fail.
    Ready,
    /// There is no home profile.
    NoHome,
    /// The sign-in of the home profile is missing or expired.
    SignedOut,
    /// No model is set, or the gateway does not serve the one that is set.
    NoModel,
    /// The gateway did not answer in time or is not reachable.
    Unreachable,
}

impl OrchestratorStatus {
    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NoHome => "no_home",
            Self::SignedOut => "signed_out",
            Self::NoModel => "no_model",
            Self::Unreachable => "unreachable",
        }
    }
}

/// The status from the home `profile`, the chosen `model` and the code of the last chat call
/// (one of [`super::client::CODES`], or `None` when it worked or there was none). Pure.
pub fn orchestrator_status(
    profile: Option<&str>,
    model: Option<&str>,
    last_error: Option<&str>,
) -> OrchestratorStatus {
    let set = |s: Option<&str>| s.is_some_and(|s| !s.trim().is_empty());
    if !set(profile) {
        return OrchestratorStatus::NoHome;
    }
    if last_error == Some(SIGNED_OUT) {
        return OrchestratorStatus::SignedOut;
    }
    if !set(model) || last_error == Some(MODEL_NOT_FOUND) {
        return OrchestratorStatus::NoModel;
    }
    if matches!(last_error, Some(OFFLINE | TIMEOUT)) {
        return OrchestratorStatus::Unreachable;
    }
    OrchestratorStatus::Ready
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_gateway::{DEFAULT_ORCHESTRATOR_MODEL, FALLBACK_ORCHESTRATOR_MODEL};

    #[test]
    fn adr_0122_model_list_is_curated_with_the_default_first_and_system_ai_ids() {
        let l = list_orchestrator_models("home");
        assert_eq!(l[0].id, DEFAULT_ORCHESTRATOR_MODEL);
        assert!(l.iter().any(|m| m.id == FALLBACK_ORCHESTRATOR_MODEL));
        assert_eq!(l.len(), 8);
        assert!(l.iter().all(|m| m.id.starts_with("system.ai.")));
    }

    #[test]
    fn adr_0122_model_list_marks_no_model_tested_until_the_eval_runs() {
        assert!(list_orchestrator_models("home").iter().all(|m| !m.tested));
    }

    #[test]
    fn adr_0122_status_states() {
        let ok = Some("system.ai.claude-haiku-5-5");
        assert_eq!(
            orchestrator_status(None, ok, None),
            OrchestratorStatus::NoHome
        );
        assert_eq!(
            orchestrator_status(Some(" "), ok, None),
            OrchestratorStatus::NoHome
        );
        assert_eq!(
            orchestrator_status(Some("h"), None, None),
            OrchestratorStatus::NoModel
        );
        assert_eq!(
            orchestrator_status(Some("h"), ok, Some("model_not_found")),
            OrchestratorStatus::NoModel
        );
        assert_eq!(
            orchestrator_status(Some("h"), ok, Some("signed_out")),
            OrchestratorStatus::SignedOut
        );
        assert_eq!(
            orchestrator_status(Some("h"), ok, Some("offline")),
            OrchestratorStatus::Unreachable
        );
        assert_eq!(
            orchestrator_status(Some("h"), ok, Some("timeout")),
            OrchestratorStatus::Unreachable
        );
        assert_eq!(
            orchestrator_status(Some("h"), ok, None),
            OrchestratorStatus::Ready
        );
        assert_eq!(
            orchestrator_status(Some("h"), ok, Some("rate_limited")),
            OrchestratorStatus::Ready
        );
        assert_eq!(OrchestratorStatus::NoHome.as_str(), "no_home");
    }
}
