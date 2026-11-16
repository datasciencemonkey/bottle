//! The hub notice actions of #27 (MB-08a, ADR 0044) pass the R12 checks as a hub `view_emit`
//! with mogId `hub`, and only the hub window can send them for that scope.
//! TODO(#393): hub-level view input kinds replace this scope in the contract.

use mog_core::ipc::{self, Handled};
use serde_json::json;

fn accept() -> serde_json::Value {
    json!({"mogId": "hub", "kind": "click", "payload": {"action": "notice.accept", "noticeVersion": 1}})
}

#[test]
fn mb_08a_hub_notice_actions_relay_to_the_brain() {
    for args in [
        accept(),
        json!({"mogId": "hub", "kind": "click", "payload": {"action": "notice.ready"}}),
    ] {
        assert_eq!(
            ipc::handle("hub", "view_emit", args).expect("the hub may send notice actions"),
            Handled::RelayToBrain { from: "hub".into() }
        );
    }
}

#[test]
fn mb_08a_a_mog_or_card_window_cannot_send_for_the_hub_scope() {
    for label in ["mog-a1", "card-a1"] {
        assert!(
            ipc::handle(label, "view_emit", accept()).is_err(),
            "{label} must not speak for the hub scope"
        );
    }
}

#[test]
fn mb_08a_the_brain_cannot_send_a_hub_notice_action() {
    assert!(ipc::handle("brain", "view_emit", accept()).is_err());
}
