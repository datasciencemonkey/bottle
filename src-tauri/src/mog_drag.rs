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
