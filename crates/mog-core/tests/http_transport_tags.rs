//! R7 HttpTransport request identity (#342, ADR 0033): the trait takes the IPC tags, a fixed
//! map gives each route ID its call kind, and `outbound_headers` builds the headers that every
//! R7 implementation sends (SEC-06a, SEC-06b). An unknown route fails closed (ARC-6, 4.6 rule 1).

use mog_core::error::CoreError;
use mog_core::http_transport::{
    outbound_headers, route_call_kind, HttpTransport, StubHttpTransport, ROUTES,
};
use mog_core::ipc::RouteId;
use mog_core::request_identity::{BottleIdentity, CallKind, TAGS_HEADER, USER_AGENT_HEADER};
use mog_core::types::{TagApp, TagOp, Tags};

fn tags(mog: &str) -> Tags {
    serde_json::from_value(serde_json::json!({"app": "bottle", "mog": mog, "op": "cleanup"}))
        .unwrap()
}

fn names(h: &[(&'static str, String)]) -> Vec<&'static str> {
    h.iter().map(|(n, _)| *n).collect()
}

/// Every wire name of `ipc::RouteId`, read from its JSON Schema (so a new variant is seen).
fn ipc_route_ids() -> Vec<String> {
    let schema = schemars::schema_for!(RouteId).to_value();
    let mut out: Vec<String> = Vec::new();
    let mut walk = vec![schema];
    while let Some(v) = walk.pop() {
        match v {
            serde_json::Value::Object(m) => {
                if let Some(serde_json::Value::Array(e)) = m.get("enum") {
                    out.extend(e.iter().filter_map(|x| x.as_str().map(str::to_owned)));
                }
                if let Some(serde_json::Value::String(c)) = m.get("const") {
                    out.push(c.clone());
                }
                walk.extend(m.into_values());
            }
            serde_json::Value::Array(a) => walk.extend(a),
            _ => {}
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn sec_06a_unity_gateway_model_routes_carry_the_tags_header() {
    for route in ["mlflow.chat_completions", "mlflow.embeddings"] {
        assert_eq!(
            route_call_kind(route),
            Some(CallKind::UnityGateway),
            "{route}"
        );
        let h = outbound_headers(&BottleIdentity::new(), route, &tags("a1")).unwrap();
        assert_eq!(names(&h), vec![USER_AGENT_HEADER, TAGS_HEADER], "{route}");
        let value = &h[1].1;
        assert_eq!(value, r#"{"app":"bottle","mog":"a1","op":"cleanup"}"#);
    }
}

#[test]
fn sec_06b_every_ipc_route_id_is_mapped_and_gets_the_user_agent_only() {
    let ids = ipc_route_ids();
    assert!(ids.len() >= 19, "{ids:?}");
    for id in &ids {
        assert_eq!(route_call_kind(id), Some(CallKind::Workspace), "{id}");
        let h = outbound_headers(&BottleIdentity::new(), id, &tags("a1")).unwrap();
        // Data minimisation (ADR 0023 decision 5): no MOG ID on workspace REST calls.
        assert_eq!(names(&h), vec![USER_AGENT_HEADER], "{id}");
        assert!(h[0].1.starts_with("bottle/"));
    }
}

#[test]
fn sec_06b_the_route_map_is_fixed_and_has_no_duplicates() {
    let mut seen: Vec<&str> = ROUTES.iter().map(|(r, _)| *r).collect();
    let n = seen.len();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), n, "duplicate route ID in ROUTES");
    // Only the IPC routes and the two Unity Gateway model routes.
    assert_eq!(n, ipc_route_ids().len() + 2);
}

#[test]
fn arc_6_an_unknown_route_or_a_url_fails_closed() {
    for bad in [
        "",
        "https://evil.example/api",
        "/ai-gateway/mlflow/v1/chat/completions",
        "SCIM.ME",
        "mlflow.chat_completions ",
    ] {
        assert_eq!(route_call_kind(bad), None, "{bad:?}");
        let err = outbound_headers(&BottleIdentity::new(), bad, &tags("a1")).unwrap_err();
        assert_eq!(err.code(), "unknown_route", "{bad:?}");
        assert!(!err.to_string().contains("evil"), "{err}");
    }
}

#[test]
fn sec_06a_bad_tags_fail_closed_with_a_content_free_code() {
    for mog in ["someone@example.com", "", "a b", "https://example.com"] {
        let t = Tags {
            app: TagApp::Bottle,
            mog: mog.to_owned(),
            op: TagOp::Cleanup,
        };
        let err = outbound_headers(&BottleIdentity::new(), "mlflow.embeddings", &t).unwrap_err();
        assert!(err.code().starts_with("tags_"), "{mog:?}: {}", err.code());
        assert!(!err.to_string().contains("example"), "{err}");
    }
}

#[test]
fn sec_06a_the_stub_transport_takes_the_tags_argument() {
    let s = StubHttpTransport;
    let r = s.request("p", "scim.me", None, &tags("a1"));
    assert!(matches!(
        r,
        Err(CoreError::NotImplemented { component: "R7" })
    ));
}
