//! The host side of MOG eye tracking (lamp-alive): read the global pointer, plan with
//! `mog_core::mog_look`, and send each MOG's look vector to the brain as `mog:look`.
//!
//! The pointer read is the Tauri cursor read, which is `NSEvent.mouseLocation` on macOS and
//! `GetCursorPos` on Windows, so it needs no new permission and no new crate. Only the look
//! vector `{ mogId, dx, dy }` leaves this thread. A pointer position is never logged, stored or
//! sent (ARC-5: the app crate has no Tauri event; the brain forwards the vector to the one view).
//!
//! Poll rate: about 30 Hz while the pointer is within 400 px of a visible MOG, about 4 Hz when
//! it is further away or no MOG is visible. The window rectangles are read at most every
//! `GEOMETRY_TTL`, so a fast poll costs one pointer read per tick.

use mog_core::ipc::events;
use mog_core::mog_look::{CursorSource, LookGate, LookLoop, WindowGeom, WindowSource};
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tauri::Manager;

/// The one wake signal of the poll thread. The window host wakes it when a MOG window is shown;
/// the `view_emit` of a MOG view wakes it when the user hides or shows the art (`hide_notice`).
pub fn gate() -> &'static Arc<LookGate> {
    static GATE: OnceLock<Arc<LookGate>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(LookGate::default()))
}

/// The `action` of the `click` a MOG view sends when the user hides or shows its art (the Hide
/// item, or a click on the hidden lamp). Payload `{ action, hidden }`. The host takes it for
/// itself and does not relay it to the brain: it is not a user click.
pub const VIEW_HIDDEN: &str = "view.hidden";

/// If `input` (the `view_emit` arguments of window `from`) is the hide notice of a MOG view, tell
/// the gate and return `true`; the caller then does not relay it.
pub fn hide_notice(from: &str, input: &serde_json::Value) -> bool {
    let Some(id) = from.strip_prefix("mog-") else {
        return false;
    };
    let p = input.get("payload");
    if input.get("kind").and_then(|k| k.as_str()) != Some("click")
        || p.and_then(|p| p.get("action")).and_then(|a| a.as_str()) != Some(VIEW_HIDDEN)
    {
        return false;
    }
    if let Some(hidden) = p.and_then(|p| p.get("hidden")).and_then(|h| h.as_bool()) {
        gate().set_hidden(id, hidden);
    }
    true
}

struct TauriCursor<'a>(&'a tauri::AppHandle<tauri::Wry>);

impl CursorSource for TauriCursor<'_> {
    fn position(&mut self) -> Option<(f64, f64)> {
        self.0.cursor_position().ok().map(|p| (p.x, p.y))
    }
}

struct TauriWindows<'a>(&'a tauri::AppHandle<tauri::Wry>);

impl WindowSource for TauriWindows<'_> {
    fn visible(&mut self) -> Vec<(String, WindowGeom)> {
        let mut out = Vec::new();
        for (label, w) in self.0.webview_windows() {
            let Some(id) = label.strip_prefix("mog-") else {
                continue;
            };
            if !w.is_visible().unwrap_or(false) {
                continue;
            }
            let (Ok(pos), Ok(size), Ok(scale)) =
                (w.outer_position(), w.outer_size(), w.scale_factor())
            else {
                continue;
            };
            out.push((
                id.to_owned(),
                WindowGeom {
                    x: f64::from(pos.x),
                    y: f64::from(pos.y),
                    w: f64::from(size.width),
                    h: f64::from(size.height),
                    scale,
                },
            ));
        }
        out
    }
}

/// Start the poll thread. It blocks on the gate while no MOG is visible (none open, or every one
/// hidden by the user): then it reads neither the pointer nor the window list and has no timer.
/// It ends when the brain channel is no longer managed state.
pub fn spawn_look_poller(
    app: tauri::AppHandle<tauri::Wry>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("bottle-mog-look".into())
        .spawn(move || {
            let gate = gate();
            let start = Instant::now();
            let mut look = LookLoop::default();
            let mut seen = u64::MAX;
            loop {
                let Some(brain) = app.try_state::<crate::ipc::BrainChannel>() else {
                    return;
                };
                let (epoch, hidden) = gate.snapshot();
                let woken = epoch != seen;
                let now_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                let (updates, wait) = look.tick(
                    now_ms,
                    woken,
                    &hidden,
                    &mut TauriCursor(&app),
                    &mut TauriWindows(&app),
                );
                for u in updates {
                    brain.send(
                        events::MOG_LOOK,
                        serde_json::json!({ "mogId": u.mog_id, "dx": u.dx, "dy": u.dy }),
                    );
                }
                seen = gate.wait(epoch, wait);
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_hide_notice_of_a_mog_view_is_taken_and_wakes_the_gate() {
        let before = gate().snapshot().0;
        let input = json!({"mogId": "t1", "kind": "click", "payload": {"action": "view.hidden", "hidden": true}});
        assert!(hide_notice("mog-t1", &input));
        assert!(gate().snapshot().1.contains("t1"));
        assert!(gate().snapshot().0 > before);
        let input = json!({"mogId": "t1", "kind": "click", "payload": {"action": "view.hidden", "hidden": false}});
        assert!(hide_notice("mog-t1", &input));
        assert!(!gate().snapshot().1.contains("t1"));
    }

    #[test]
    fn other_input_and_other_windows_are_not_taken() {
        let hidden = json!({"kind": "click", "payload": {"action": "view.hidden", "hidden": true}});
        assert!(!hide_notice("card-t2", &hidden), "only a MOG view");
        assert!(!hide_notice("hub", &hidden));
        for other in [
            json!({"kind": "click"}),
            json!({"kind": "hover", "payload": {"action": "view.hidden", "hidden": true}}),
            json!({"kind": "click", "payload": {"action": "view.ready"}}),
        ] {
            assert!(!hide_notice("mog-t2", &other), "{other}");
        }
        assert!(!gate().snapshot().1.contains("t2"));
    }
}
