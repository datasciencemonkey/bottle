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
use mog_core::mog_look::{CursorSource, LookPlanner, WindowGeom, FAR_PERIOD};
use std::time::{Duration, Instant};
use tauri::Manager;

/// How long the window rectangles stay valid. A drag moves a MOG; the eyes can lag a little.
const GEOMETRY_TTL: Duration = Duration::from_millis(250);

struct TauriCursor<'a>(&'a tauri::AppHandle<tauri::Wry>);

impl CursorSource for TauriCursor<'_> {
    fn position(&mut self) -> Option<(f64, f64)> {
        self.0.cursor_position().ok().map(|p| (p.x, p.y))
    }
}

/// The visible MOG windows, as `(mog id, physical rectangle, scale)`.
fn visible_mogs(app: &tauri::AppHandle<tauri::Wry>) -> Vec<(String, WindowGeom)> {
    let mut out = Vec::new();
    for (label, w) in app.webview_windows() {
        let Some(id) = label.strip_prefix("mog-") else {
            continue;
        };
        if !w.is_visible().unwrap_or(false) {
            continue;
        }
        let (Ok(pos), Ok(size), Ok(scale)) = (w.outer_position(), w.outer_size(), w.scale_factor())
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

/// Start the poll thread. It ends when the brain channel is no longer managed state.
pub fn spawn_look_poller(
    app: tauri::AppHandle<tauri::Wry>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("bottle-mog-look".into())
        .spawn(move || {
            let mut planner = LookPlanner::default();
            let mut windows: Vec<(String, WindowGeom)> = Vec::new();
            let mut read_at: Option<Instant> = None;
            let mut period = FAR_PERIOD;
            loop {
                std::thread::sleep(period);
                let Some(brain) = app.try_state::<crate::ipc::BrainChannel>() else {
                    return;
                };
                if read_at.is_none_or(|t| t.elapsed() >= GEOMETRY_TTL) {
                    windows = visible_mogs(&app);
                    read_at = Some(Instant::now());
                }
                let (updates, next) = planner.poll(&mut TauriCursor(&app), &windows);
                period = next;
                for u in updates {
                    brain.send(
                        events::MOG_LOOK,
                        serde_json::json!({ "mogId": u.mog_id, "dx": u.dx, "dy": u.dy }),
                    );
                }
            }
        })
}
