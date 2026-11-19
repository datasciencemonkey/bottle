//! The native drag of a MOG window (MOG-01c, UX-27a), as a pure state machine.
//!
//! The macOS event monitor in `macos_panel` feeds it the left-mouse events of MOG panels.
//! A move of `DRAG_THRESHOLD_PT` or more from the press starts one native window drag. From then
//! on the gesture swallows its events, including the mouse-up, so the page never gets a `click`
//! for a drag and no capture starts. A smaller move passes every event to the page, which keeps
//! the click rules (MOG-12a, #83).

/// UX-27a: a move of 4 px or more starts a native drag. AppKit gives points.
pub const DRAG_THRESHOLD_PT: f64 = 4.0;

/// A left-mouse event in screen coordinates (points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MouseInput {
    Down { x: f64, y: f64 },
    Dragged { x: f64, y: f64 },
    Up,
}

/// What to do with the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Give the event to the page as usual.
    Pass,
    /// Start a native window drag with this event, and do not give it to the page.
    StartDrag,
    /// Do not give the event to the page (part of a drag).
    Swallow,
}

/// One press-move-release gesture on a MOG window.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DragGesture {
    press: Option<(f64, f64)>,
    dragging: bool,
}

impl DragGesture {
    /// Feed one event and get what to do with it.
    pub fn on(&mut self, input: MouseInput) -> Verdict {
        match input {
            MouseInput::Down { x, y } => {
                *self = DragGesture {
                    press: Some((x, y)),
                    dragging: false,
                };
                Verdict::Pass
            }
            MouseInput::Dragged { x, y } => {
                if self.dragging {
                    return Verdict::Swallow;
                }
                match self.press {
                    Some((px, py)) if (x - px).hypot(y - py) >= DRAG_THRESHOLD_PT => {
                        self.dragging = true;
                        Verdict::StartDrag
                    }
                    _ => Verdict::Pass,
                }
            }
            MouseInput::Up => {
                let was_dragging = self.dragging;
                *self = DragGesture::default();
                if was_dragging {
                    Verdict::Swallow
                } else {
                    Verdict::Pass
                }
            }
        }
    }

    /// True between the start of a drag and its mouse-up.
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }
}

/// refs #10, #103: a completed click (mouse-down then mouse-up, not a drag) on a MOG or
/// card panel, in AppKit screen points, with its AppKit timestamp. These panels never become key
/// and the MOG never activates the app (UX-19a, MOG-03b), and on such a window AppKit's own
/// `NSEvent.clickCount` never passes 1: every click arrives through the `acceptsFirstMouse`
/// path, which AppKit does not count toward a multi-click sequence. So a genuine double click on
/// one of these panels never becomes a native `dblclick` in the page by itself, and the native
/// side must notice the gesture itself from two `ClickEvent`s (`macos_panel::notice_click`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClickEvent {
    pub timestamp: f64,
    pub x: f64,
    pub y: f64,
}

/// How far apart (in points) two clicks of a double click may land: the same as the drag
/// threshold, so a move past it is a new click, not a double click, on the same panel.
pub const DOUBLE_CLICK_DISTANCE_PT: f64 = DRAG_THRESHOLD_PT;

/// Whether `second` completes a double click that started at `first`, given the system's own
/// double-click interval (`NSEvent.doubleClickInterval`, read natively).
pub fn is_double_click(first: ClickEvent, second: ClickEvent, max_interval: f64) -> bool {
    let dt = second.timestamp - first.timestamp;
    let dist = (second.x - first.x).hypot(second.y - first.y);
    (0.0..=max_interval).contains(&dt) && dist <= DOUBLE_CLICK_DISTANCE_PT
}
