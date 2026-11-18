//! #307: the Tauri host sends `agent_start`, `agent_stop` and `agent_send` to the one R16
//! `SharedAgentHost` of the app through the hardened gateway (RTR-21a, ARC-7). Only the brain
//! may call them (ARC-4). The host here is the production build (`agent_host::build`); it has
//! no model target resolver yet (TODO(#16, #18)), so a start answers `agent_target_unavailable`
//! before any runtime starts. No harness, no model and no token is used.

use bottle_lib::agent_host;
use bottle_lib::ipc::{route, runs_off_main_thread, BrainLink, HostComponents};
use mog_core::ipc::ChannelRef;
use mog_core::ipc_gateway::GatewayComponents;
use serde_json::{json, Value};
use std::sync::Arc;

const MOG: &str = "00000000-0000-4000-8000-0000000003d4";
const QUESTION: &str = "test-question-not-real";

struct NoBrain;

impl BrainLink for NoBrain {
    fn relay(&self, _event: &str, _payload: Value) -> bool {
        false
    }
    fn subscribe(&self, _channel: &ChannelRef) -> bool {
        false
    }
    fn notify_hub(&self, _notice: Value) -> bool {
        false
    }
    fn subscribe_hub(&self, _channel: &ChannelRef) -> bool {
        false
    }
}

fn call(
    from: &str,
    agent: Option<&dyn GatewayComponents>,
    command: &str,
    args: Value,
) -> Result<Value, Value> {
    let components = HostComponents {
        agent,
        ..HostComponents::none(from, &NoBrain)
    };
    route(components, command, args)
}

fn commands() -> [(&'static str, Value); 3] {
    [
        ("agent_start", json!({"mogId": MOG, "harness": "claude"})),
        ("agent_stop", json!({"mogId": MOG})),
        (
            "agent_send",
            json!({"mogId": MOG, "turn": {"mogId": MOG, "question": QUESTION}}),
        ),
    ]
}

fn host() -> agent_host::AgentHostApp {
    agent_host::build(Arc::new(|_event: &str, _payload: Value| {})).expect("the host builds")
}

#[test]
fn rtr_21a_arc_7_the_agent_commands_reach_the_shared_agent_host() {
    let h = host();
    let agent = Some(&h as &dyn GatewayComponents);
    // No model target yet (TODO(#16, #18)): the start stops before any runtime starts.
    assert_eq!(
        call("brain", agent, "agent_start", commands()[0].1.clone()),
        Err(json!({"error": "agent_target_unavailable"}))
    );
    assert_eq!(
        call("brain", agent, "agent_stop", commands()[1].1.clone()),
        Err(json!({"error": "agent_unknown_mog"}))
    );
    let sent = call("brain", agent, "agent_send", commands()[2].1.clone());
    assert_eq!(sent, Err(json!({"error": "agent_unknown_mog"})));
    assert!(!format!("{sent:?}").contains(QUESTION));
    assert_eq!(h.host().runtime_count(), 0, "no runtime started");
}

#[test]
fn arc_4_a_view_window_may_not_call_the_agent_commands() {
    let h = host();
    for from in ["hub", "mog-a1", "card-a1"] {
        for (command, args) in commands() {
            assert_eq!(
                call(from, Some(&h), command, args),
                Err(json!({"error": "window_not_allowed"})),
                "{from} {command}"
            );
        }
    }
    assert_eq!(h.host().session_count(), 0);
}

#[test]
fn arc_7_without_the_agent_host_the_commands_answer_host_failed() {
    for (command, args) in commands() {
        assert_eq!(
            call("brain", None, command, args),
            Err(json!({"error": "host_failed"})),
            "{command}"
        );
    }
}

#[test]
fn rtr_21a_the_agent_commands_run_off_the_main_thread() {
    // A start can spawn the runtime and wait for the target lookup; a send can wait for the
    // host lock while another MOG starts; a stop kills and reaps the runtime.
    for command in ["agent_start", "agent_stop", "agent_send"] {
        assert!(runs_off_main_thread(command), "{command}");
    }
}
