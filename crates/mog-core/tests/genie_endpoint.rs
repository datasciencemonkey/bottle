//! G-01, G-02, G-03 (#28): the backend enum, the Genie One MCP endpoint and the rejected
//! backends. AT-07 and AT-11 (offline parts).

use mog_core::genie_endpoint::{rejected_backend, EndpointError, GenieOneEndpoint, GENIE_ONE_PATH};
use mog_core::ipc::JsonObject;
use mog_core::mog_config::{parse_config, BackendKind, ConfigError, BACKEND_KINDS};
use serde_json::{json, Value};

fn obj(v: Value) -> JsonObject {
    match v {
        Value::Object(m) => m.into_iter().collect(),
        _ => panic!("not an object"),
    }
}

fn config_with_backend(backend: Value) -> JsonObject {
    obj(json!({
        "schemaVersion": 1,
        "name": "Sales MOG",
        "binding": {"home": "dev", "backend": backend}
    }))
}

// ---------- G-01 ----------

#[test]
fn g_01_backend_enum_is_exactly_the_three_kinds() {
    let names: Vec<&str> = BACKEND_KINDS.iter().map(|k| k.as_str()).collect();
    assert_eq!(names, ["genie_one_mcp", "genie_agent", "custom_agent"]);
    assert_eq!(BackendKind::GenieOneMcp.as_str(), "genie_one_mcp");
    for k in BACKEND_KINDS {
        assert_eq!(BackendKind::parse(k.as_str()), Some(k));
        assert_eq!(serde_json::to_value(k).unwrap(), json!(k.as_str()));
    }
}

#[test]
fn g_01_at_11_schema_rejects_other_backend_kinds() {
    for kind in [
        "genie_mcp",
        "genie",
        "GENIE_ONE_MCP",
        "custom",
        "",
        "genie_one_api",
    ] {
        assert_eq!(BackendKind::parse(kind), None, "{kind}");
        let cfg = config_with_backend(json!({"kind": kind, "profile": "dev"}));
        assert_eq!(
            parse_config(&cfg).unwrap_err(),
            ConfigError::InvalidConfig,
            "{kind}"
        );
    }
}

#[test]
fn g_01_genie_agent_mode_is_chat_or_agent_mode_only() {
    for mode in ["chat", "agent_mode"] {
        let cfg = config_with_backend(
            json!({"kind": "genie_agent", "profile": "dev", "spaceId": "01ef", "mode": mode}),
        );
        assert!(parse_config(&cfg).is_ok(), "{mode}");
    }
    let cfg = config_with_backend(
        json!({"kind": "genie_agent", "profile": "dev", "spaceId": "01ef", "mode": "deep"}),
    );
    assert_eq!(parse_config(&cfg).unwrap_err(), ConfigError::InvalidConfig);
}

#[test]
fn g_01_config_backend_kind_names_the_enum() {
    let cfg = parse_config(&config_with_backend(
        json!({"kind": "genie_one_mcp", "profile": "dev"}),
    ))
    .unwrap();
    assert_eq!(
        cfg.binding.backend.as_ref().unwrap().kind(),
        BackendKind::GenieOneMcp
    );
}

// ---------- G-02 ----------

#[test]
fn g_02_at_07_default_endpoint_carries_o_workspace_id() {
    let e = GenieOneEndpoint::new("adb-1.azuredatabricks.net", "1234").unwrap();
    assert_eq!(
        e.url(),
        "https://adb-1.azuredatabricks.net/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=1234"
    );
    assert_eq!(e.host(), "adb-1.azuredatabricks.net");
    assert_eq!(e.workspace_id(), "1234");
    assert!(e.url().contains(GENIE_ONE_PATH));
}

#[test]
fn g_02_endpoint_accepts_a_workspace_url_with_scheme_and_slash() {
    let e = GenieOneEndpoint::new("https://Dbc-1.cloud.databricks.com/", "77").unwrap();
    assert_eq!(
        e.url(),
        "https://dbc-1.cloud.databricks.com/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=77"
    );
}

#[test]
fn g_02_endpoint_refuses_a_missing_or_bad_workspace_id() {
    for id in ["", "12a", "1 2", "-1", "1&x=2"] {
        assert_eq!(
            GenieOneEndpoint::new("h.cloud.databricks.com", id).unwrap_err(),
            EndpointError::MissingWorkspaceId,
            "{id:?}"
        );
    }
}

#[test]
fn g_02_endpoint_refuses_a_bad_host() {
    for host in [
        "",
        "http://h.cloud.databricks.com",
        "h.cloud.databricks.com/path",
        "user@h.cloud.databricks.com",
        "h.cloud.databricks.com:8443",
        "h.cloud.databricks.com?x=1",
        "h cloud",
        ".cloud.databricks.com",
    ] {
        assert!(GenieOneEndpoint::new(host, "1").is_err(), "{host:?}");
    }
}

#[test]
fn g_02_parse_accepts_only_the_genie_one_url_with_o() {
    let ok = "https://h.cloud.databricks.com/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=5";
    let e = GenieOneEndpoint::parse(ok).unwrap();
    assert_eq!(e.workspace_id(), "5");
    assert_eq!(e.url(), ok);
    let no_o = "https://h.cloud.databricks.com/ai-gateway/mcp-services/system.ai.genie_one_mcp";
    assert_eq!(
        GenieOneEndpoint::parse(no_o).unwrap_err(),
        EndpointError::MissingWorkspaceId
    );
    let two_o = format!("{no_o}?o=1&o=2");
    assert_eq!(
        GenieOneEndpoint::parse(&two_o).unwrap_err(),
        EndpointError::MissingWorkspaceId
    );
    let other = "https://h.cloud.databricks.com/ai-gateway/mcp-services/other?o=1";
    assert_eq!(
        GenieOneEndpoint::parse(other).unwrap_err(),
        EndpointError::WrongPath
    );
    let http = "http://h.cloud.databricks.com/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=1";
    assert_eq!(
        GenieOneEndpoint::parse(http).unwrap_err(),
        EndpointError::NotHttps
    );
}

// ---------- G-03 ----------

#[test]
fn g_03_at_07_beta_genie_mcp_url_is_rejected_with_a_clear_error() {
    let beta = "https://h.cloud.databricks.com/api/2.0/mcp/genie";
    let e = GenieOneEndpoint::parse(beta).unwrap_err();
    assert_eq!(e, EndpointError::BetaGenieMcp);
    assert_eq!(rejected_backend(beta), Some(EndpointError::BetaGenieMcp));
    assert_eq!(e.code(), "genie_beta_mcp_rejected");
    let msg = e.message();
    assert!(msg.contains("/api/2.0/mcp/genie"), "{msg}");
    assert!(msg.contains("2026-11-30"), "{msg}");
    assert!(msg.contains(GENIE_ONE_PATH), "{msg}");
    assert_eq!(e.to_string(), msg);
}

#[test]
fn g_03_per_space_genie_mcp_and_genie_one_sse_are_rejected() {
    let cases = [
        (
            "https://h.cloud.databricks.com/api/2.0/mcp/genie/01ef0123",
            EndpointError::PerSpaceGenieMcp,
            "genie_space_mcp_rejected",
        ),
        (
            "https://h.cloud.databricks.com/API/2.0/MCP/GENIE/",
            EndpointError::BetaGenieMcp,
            "genie_beta_mcp_rejected",
        ),
        (
            "https://h.cloud.databricks.com/api/2.0/genie/one/conversations/stream",
            EndpointError::GenieOneApiSse,
            "genie_one_sse_rejected",
        ),
        (
            "/api/2.0/mcp/genie?o=1",
            EndpointError::BetaGenieMcp,
            "genie_beta_mcp_rejected",
        ),
    ];
    for (url, err, code) in cases {
        assert_eq!(rejected_backend(url), Some(err), "{url}");
        assert_eq!(err.code(), code);
        assert!(!err.message().is_empty());
        if url.starts_with("https://") {
            assert_eq!(GenieOneEndpoint::parse(url).unwrap_err(), err, "{url}");
        }
    }
}

#[test]
fn g_03_allowed_urls_are_not_flagged() {
    for url in [
        "https://h.cloud.databricks.com/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=1",
        "https://my-app-123.aws.databricksapps.com",
        "https://h.cloud.databricks.com/api/2.0/genie/spaces/01ef/start-conversation",
        "https://h.cloud.databricks.com/api/2.0/mcp/genies",
        "ask about /api/2.0 limits",
    ] {
        assert_eq!(rejected_backend(url), None, "{url}");
    }
}

#[test]
fn g_03_at_07_config_loader_rejects_a_beta_url_with_its_own_code() {
    let cases = [
        (
            json!({"kind": "custom_agent", "profile": "dev", "protocol": "invocations",
                   "appUrl": "https://h.cloud.databricks.com/api/2.0/mcp/genie"}),
            "genie_beta_mcp_rejected",
        ),
        (
            json!({"kind": "custom_agent", "profile": "dev", "protocol": "responses",
                   "appUrl": "https://h.cloud.databricks.com/api/2.0/mcp/genie/01ef"}),
            "genie_space_mcp_rejected",
        ),
        (
            json!({"kind": "genie_one_mcp", "profile": "dev",
                   "url": "https://h.cloud.databricks.com/api/2.0/genie/one/stream"}),
            "genie_one_sse_rejected",
        ),
    ];
    for (backend, code) in cases {
        let err = parse_config(&config_with_backend(backend)).unwrap_err();
        assert_eq!(err, ConfigError::RejectedBackend(rejected(code)), "{code}");
        assert_eq!(err.code(), code);
        let shown = err.to_string();
        assert!(shown.contains(code), "{shown}");
        assert!(!shown.contains("h.cloud.databricks.com"), "{shown}");
    }
}

fn rejected(code: &str) -> EndpointError {
    match code {
        "genie_beta_mcp_rejected" => EndpointError::BetaGenieMcp,
        "genie_space_mcp_rejected" => EndpointError::PerSpaceGenieMcp,
        _ => EndpointError::GenieOneApiSse,
    }
}

#[test]
fn g_03_a_valid_custom_agent_url_still_loads() {
    let cfg = config_with_backend(json!({"kind": "custom_agent", "profile": "dev",
        "protocol": "invocations", "appUrl": "https://my-app-1.aws.databricksapps.com"}));
    assert!(parse_config(&cfg).is_ok());
}

#[test]
fn g_01_at_11_ddl_check_lists_exactly_the_enum() {
    let ddl = include_str!("../src/local_store/migrations/0001_local_mirror.sql");
    let want = format!(
        "CHECK (backend_kind IN ({}))",
        BACKEND_KINDS
            .iter()
            .map(|k| format!("'{}'", k.as_str()))
            .collect::<Vec<_>>()
            .join(",")
    );
    let columns = ddl.matches("backend_kind TEXT NOT NULL").count();
    assert!(columns >= 2, "mog and genie_exchange carry backend_kind");
    assert_eq!(ddl.matches(want.as_str()).count(), columns, "{want}");
}
