//! AT-43 "UC1 dictation into the active app" (#231), section 15. The Rust part.
//!
//! The AT script: dictate into a text editor, a browser field, a terminal and a password field;
//! repeat without Accessibility; dictate with Shift held. Pass: the text appears with no Enter
//! and the earlier clipboard returns; the password field shows the reason and Copy; without
//! Accessibility the MOG shows Copy; no agent turn and no Genie call; Shift forces the ask route;
//! key-up to inserted text 1.5 s or less at p95 (local).
//!
//! What runs here is the `insert_text` contract part of MOG-06b: only the brain may call it, and
//! its arguments have no key, submit or Enter field, so the brain cannot ask for one. The paste
//! itself (R10, MOG-06a, MOG-06c) is #58 and stays pending. The brain parts (RTR-01b, RTR-01c,
//! MOG-06d, UX-42) are in `src/acceptance/at-43-dictation.test.ts`. The latency row NFR-01d is
//! the #171 benchmark (`tests/latency_bench.rs`, `nfr_01d_*` and the AT-43 device run); this
//! file does not copy it. The four insertion targets are a person's step:
//! docs/tests/AT-DICTATION-STT-OFFLINE.md.

use mog_core::ipc::{self, InsertTextArgs, IpcError, WindowClass};
use serde_json::json;

const MOG: &str = "00000000-0000-4000-8000-0000000000a1";

#[test]
fn at_43_mog_06b_only_the_brain_may_call_insert_text() {
    let spec = ipc::command("insert_text").expect("insert_text is in the 4.5 table");
    assert_eq!(spec.windows, [WindowClass::Brain]);
    let args = json!({"mogId": MOG, "text": "hello"});
    for label in ["hub", "mog-a1", "card-a1"] {
        let err = ipc::handle(label, "insert_text", args.clone()).unwrap_err();
        assert!(
            matches!(err, IpcError::WindowNotAllowed { .. }),
            "{label}: {err:?}"
        );
    }
    // The brain passes the gateway; the paste itself is #58.
    let err = ipc::handle("brain", "insert_text", args).unwrap_err();
    assert!(matches!(err, IpcError::NotImplemented { .. }), "{err:?}");
}

#[test]
fn at_43_mog_06b_insert_text_takes_no_key_or_submit_field() {
    for extra in [
        "pressEnter",
        "enter",
        "submit",
        "keys",
        "keystrokes",
        "sendKeys",
    ] {
        let mut args = json!({"mogId": MOG, "text": "hello"});
        args[extra] = json!(true);
        let err = ipc::handle("brain", "insert_text", args).unwrap_err();
        assert!(
            matches!(err, IpcError::InvalidArgs { .. }),
            "{extra}: {err:?}"
        );
    }
}

#[test]
fn at_43_mog_06b_multi_line_text_is_one_argument() {
    // A multi-line text reaches R10 as one string, not as lines with a key between them.
    let a: InsertTextArgs =
        serde_json::from_value(json!({"mogId": MOG, "text": "line one\nline two\n"})).unwrap();
    assert_eq!(a.text, "line one\nline two\n");
    assert_eq!(a.mog_id, MOG);
}

#[test]
#[ignore = "pending TODO(#58): R10 insert_text (clipboard paste, clipboard restore) is not built"]
fn at_43_mog_06a_mog_06c_paste_inserts_with_no_enter_and_restores_the_clipboard() {
    // TODO(#58): drive R10 with a clipboard double and a key-event recorder:
    // 1. the clipboard holds "earlier";
    // 2. insert_text {text: "line one\nline two"} gives `inserted`;
    // 3. the key log has the paste chord only, never Return or Enter (MOG-06b);
    // 4. 1 s after the paste the clipboard holds "earlier" again (MOG-06c);
    // 5. the clipboard write had the concealed and transient flags (#213).
    let err = ipc::handle(
        "brain",
        "insert_text",
        json!({"mogId": MOG, "text": "line one\nline two"}),
    )
    .expect_err("TODO(#58): R10 is a stub");
    assert!(matches!(err, IpcError::NotImplemented { .. }));
    todo!("TODO(#58): assert the key log has no Enter and the earlier clipboard is back after 1 s");
}
