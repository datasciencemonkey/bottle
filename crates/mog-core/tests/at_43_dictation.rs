//! AT-43 "UC1 dictation into the active app" (#231), section 15. The Rust part.
//!
//! The AT script: dictate into a text editor, a browser field, a terminal and a password field;
//! repeat without Accessibility; dictate and tap G in the hold. Pass: the text appears with no Enter
//! and the earlier clipboard returns; the password field shows the reason and Copy; without
//! Accessibility the MOG shows Copy; no agent turn and no Genie call; ⌥G forces the ask route;
//! key-up to inserted text 1.5 s or less at p95 (local).
//!
//! What runs here is the `insert_text` contract part of MOG-06b: only the brain may call it, and
//! its arguments have no key, submit or Enter field, so the brain cannot ask for one. The paste
//! itself (R11, MOG-06a, MOG-06c, #58) runs through the gateway over a clipboard and key double.
//! The brain parts (RTR-01b, RTR-01c, MOG-06d, UX-42) are in
//! `src/acceptance/at-43-dictation.test.ts`. The latency row NFR-01d is the #171 benchmark
//! (`tests/latency_bench.rs`, `nfr_01d_*` and the AT-43 device run); this file does not copy it. The four insertion targets are a person's step:
//! docs/tests/AT-DICTATION-STT-OFFLINE.md.

use mog_core::error::CoreResult;
use mog_core::insertion::{
    is_enter_key, paste_chord, AppId, Focus, InsertHost, InsertTextComponent, KeyStroke,
    TargetTable,
};
use mog_core::ipc::{self, InsertTextArgs, IpcError, WindowClass};
use mog_core::ipc_gateway::{hardened_gateway_with, IpcGateway, NoLog};
use mog_core::window_host::{
    clipboard_formats, ClipboardBackend, ClipboardFormat, ClipboardPlatform, ClipboardWriter,
};
use serde_json::json;
use std::sync::Mutex;
use std::time::Duration;

const MOG: &str = "00000000-0000-4000-8000-0000000000a1";

#[test]
fn at_43_mog_06b_only_the_brain_may_call_insert_text() {
    let spec = ipc::command("insert_text").expect("insert_text is in the 4.5 table");
    assert_eq!(spec.windows, [WindowClass::Brain]);
    let args = json!({"mogId": MOG, "text": "hello", "leaseId": "1"});
    for label in ["hub", "mog-a1", "card-a1"] {
        let err = ipc::handle(label, "insert_text", args.clone()).unwrap_err();
        assert!(
            matches!(err, IpcError::WindowNotAllowed { .. }),
            "{label}: {err:?}"
        );
    }
    // The brain passes the checks; `handle` leaves the paste to the R11 component (#58).
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
        let mut args = json!({"mogId": MOG, "text": "hello", "leaseId": "1"});
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
    // A multi-line text passes the gateway as one string, not as lines with a key between them:
    // the checks accept it (the R11 component pastes it), and the parsed text keeps every newline.
    let args = json!({"mogId": MOG, "text": "line one\nline two\n", "leaseId": "1"});
    let err = ipc::handle("brain", "insert_text", args.clone()).unwrap_err();
    assert!(matches!(err, IpcError::NotImplemented { .. }), "{err:?}");
    let a: InsertTextArgs = serde_json::from_value(args).unwrap();
    assert_eq!(a.text, "line one\nline two\n");
    // A list of lines is not a text.
    let err = ipc::handle(
        "brain",
        "insert_text",
        json!({"mogId": MOG, "text": ["line one", "line two"], "leaseId": "1"}),
    )
    .unwrap_err();
    assert!(matches!(err, IpcError::InvalidArgs { .. }), "{err:?}");
}

/// A clipboard and key-event recorder for the R11 paste (MOG-06a, MOG-06c).
struct Desk {
    clipboard: Mutex<String>,
    mark: Mutex<u64>,
    keys: Mutex<Vec<KeyStroke>>,
    writes: Mutex<Vec<String>>,
    /// The formats that reached the clipboard, through the same writer as the OS hosts.
    formats: Mutex<Vec<Vec<ClipboardFormat>>>,
}

impl ClipboardBackend for &Desk {
    fn write(&self, formats: &[ClipboardFormat]) -> CoreResult<()> {
        self.formats.lock().unwrap().push(formats.to_vec());
        Ok(())
    }
    fn clear(&self) -> CoreResult<()> {
        Ok(())
    }
}

impl InsertHost for Desk {
    type Snapshot = String;
    fn input_permitted(&self) -> bool {
        true
    }
    fn focus(&self) -> Focus {
        Focus::Field
    }
    fn focus_target(&self) -> u64 {
        1
    }
    fn focused_app_pid(&self) -> AppId {
        AppId::new(100, 0)
    }
    fn snapshot_clipboard(&self) -> CoreResult<String> {
        Ok(self.clipboard.lock().unwrap().clone())
    }
    fn is_concealed(&self, _snapshot: &String) -> bool {
        false
    }
    fn write_concealed(&self, text: &str) -> CoreResult<()> {
        ClipboardWriter::new(self).write_text_for(ClipboardPlatform::MacOs, text)?;
        self.writes.lock().unwrap().push(text.to_owned());
        *self.clipboard.lock().unwrap() = text.to_owned();
        *self.mark.lock().unwrap() += 1;
        Ok(())
    }
    fn clipboard_mark(&self) -> u64 {
        *self.mark.lock().unwrap()
    }
    fn send_keys(&self, keys: &[KeyStroke]) -> CoreResult<()> {
        self.keys.lock().unwrap().extend_from_slice(keys);
        Ok(())
    }
    fn restore_clipboard(&self, snapshot: String) -> CoreResult<()> {
        *self.clipboard.lock().unwrap() = snapshot;
        Ok(())
    }
    fn sleep(&self, _d: Duration) {}
}

#[test]
fn at_43_mog_06a_mog_06c_paste_inserts_with_no_enter_and_restores_the_clipboard() {
    // Through the hardened gateway, as the host runs it (ADR 0040).
    let desk = Desk {
        clipboard: Mutex::new("earlier".into()),
        mark: Mutex::new(0),
        keys: Mutex::new(Vec::new()),
        writes: Mutex::new(Vec::new()),
        formats: Mutex::new(Vec::new()),
    };
    let platform = ClipboardPlatform::MacOs;
    // ADR 0118: lease 1 of this MOG was granted while the app of the desk had the focus.
    let targets: &'static TargetTable = Box::leak(Box::new(TargetTable::new()));
    let note = targets.begin_note(0);
    targets.finish_note(note, AppId::new(100, 0));
    targets.bind(1, MOG, 1);
    let gw = hardened_gateway_with(
        NoLog,
        InsertTextComponent::with_targets(&desk, Some(platform), targets),
    );
    let reply = gw
        .dispatch(
            "brain",
            "insert_text",
            json!({"mogId": MOG, "text": "line one\nline two", "leaseId": "1"}),
        )
        .unwrap();
    assert_eq!(reply, json!({"result": "inserted"}));
    // The text went on the clipboard once, unchanged (one field, MOG-06b).
    assert_eq!(*desk.writes.lock().unwrap(), vec!["line one\nline two"]);
    // The key log has the paste chord only, never Return or Enter (MOG-06b).
    let keys = desk.keys.lock().unwrap().clone();
    assert_eq!(keys, paste_chord(platform));
    assert!(keys.iter().all(|k| !is_enter_key(platform, k.key)));
    // After the paste the clipboard holds "earlier" again (MOG-06c).
    assert_eq!(*desk.clipboard.lock().unwrap(), "earlier");
    // Step 5: the clipboard write had the concealed and transient flags (#213, SEC-30), in one
    // write with the text. The hosts' own writes are checked in `insertion/macos.rs` and
    // `insertion/windows.rs`. TODO(#408): on a Mac with Maccy (or Windows with Win+V history
    // on), dictate "echo the dictation test works" into TextEdit (Notepad); expected: the text
    // is pasted and the clipboard manager history has no new entry for it.
    let written = desk.formats.lock().unwrap().clone();
    assert_eq!(
        written,
        vec![clipboard_formats(platform, "line one\nline two")]
    );
    let names: Vec<&str> = written[0].iter().map(|f| f.name).collect();
    assert!(
        names.contains(&"org.nspasteboard.ConcealedType"),
        "{names:?}"
    );
    assert!(
        names.contains(&"org.nspasteboard.TransientType"),
        "{names:?}"
    );
    // A view window still cannot reach it.
    assert!(gw
        .dispatch(
            "mog-a1",
            "insert_text",
            json!({"mogId": MOG, "text": "x", "leaseId": "1"})
        )
        .is_err());
}

/// ADR 0118, decision 21 and security requirement 10: no IPC command sets or reads a paste
/// target. No argument of any command is a pid, an app or a target.
#[test]
fn ux2_no_ipc_command_takes_a_pid_or_a_target_argument() {
    fn names(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                if let Some(serde_json::Value::Object(props)) = m.get("properties") {
                    out.extend(props.keys().cloned());
                }
                m.values().for_each(|c| names(c, out));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|c| names(c, out)),
            _ => {}
        }
    }
    let schemas = ipc::schemas();
    assert!(schemas.contains_key("insert_text"));
    for (command, schema) in schemas {
        let mut props = Vec::new();
        names(&schema, &mut props);
        for p in props {
            let banned = [
                "pid",
                "processid",
                "apppid",
                "appid",
                "targetpid",
                "targetapp",
                "frontmost",
                "focusedapp",
                "pasteTarget",
            ];
            assert!(
                !banned.iter().any(|b| b.eq_ignore_ascii_case(&p)),
                "{command} has the argument {p}"
            );
        }
    }
    // insert_text itself: only the MOG, the text and the lease.
    let mut props = Vec::new();
    names(&ipc::schemas()["insert_text"], &mut props);
    props.sort();
    assert_eq!(props, ["leaseId", "mogId", "text"]);
}
