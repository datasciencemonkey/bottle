//! Window kinds of ARC-3 and the builder that makes them.
//!
//! The pure part (`WindowKind`, `WindowSpec`) has no Tauri dependency, so tests check
//! ARC-3 without a GUI. `build` turns a spec into a real window.
//!
//! SP-01 (#10) can change how a transparent MOG window is made on macOS. That choice
//! stays inside `transparency::apply` (see `docs/adr/0001-foundation-proceeds-before-sp01.md`).

use std::time::Duration;

/// ARC-3 / MB-03b: at most 8 MOG windows are active by default.
pub const MAX_ACTIVE_MOGS: usize = 8;

/// ARC-3: a hidden card window is destroyed after 5 min.
/// TODO(#78): the reaper timer that uses this value.
pub const CARD_HIDDEN_TTL: Duration = Duration::from_secs(5 * 60);

/// The longest MOG or card id we accept in a window label (a uuid v4 is 36).
const MAX_ID_LEN: usize = 64;

/// The four window kinds of ARC-3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowKind {
    /// Hidden window with all domain logic (ARC-2). Exactly one.
    Brain,
    /// Normal hub window (wizard, settings, history). Zero or one.
    Hub,
    /// One floating MOG view per active MOG.
    Mog(String),
    /// One card view per MOG, made on the first open.
    Card(String),
}

/// What a window looks like and how it behaves, independent of Tauri.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowSpec {
    pub label: String,
    /// The HTML entry point inside the app bundle.
    pub url: &'static str,
    pub title: &'static str,
    pub visible: bool,
    pub transparent: bool,
    pub decorations: bool,
    pub always_on_top: bool,
    /// `false` = the window never takes focus from the app the user types in.
    pub focusable: bool,
    pub skip_taskbar: bool,
    pub resizable: bool,
    pub shadow: bool,
    pub width: u32,
    pub height: u32,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

impl WindowKind {
    /// The Tauri window label. Capabilities are scoped by these labels (ARC-4).
    pub fn label(&self) -> String {
        match self {
            WindowKind::Brain => "brain".to_owned(),
            WindowKind::Hub => "hub".to_owned(),
            WindowKind::Mog(id) => format!("mog-{id}"),
            WindowKind::Card(id) => format!("card-{id}"),
        }
    }

    /// Parse a window label back into a kind. Unknown labels give `None`.
    pub fn from_label(label: &str) -> Option<WindowKind> {
        match label {
            "brain" => Some(WindowKind::Brain),
            "hub" => Some(WindowKind::Hub),
            _ => {
                if let Some(id) = label.strip_prefix("mog-") {
                    valid_id(id).then(|| WindowKind::Mog(id.to_owned()))
                } else if let Some(id) = label.strip_prefix("card-") {
                    valid_id(id).then(|| WindowKind::Card(id.to_owned()))
                } else {
                    None
                }
            }
        }
    }

    /// Make a MOG kind from an id, or `None` if the id is not label-safe.
    pub fn mog(id: &str) -> Option<WindowKind> {
        valid_id(id).then(|| WindowKind::Mog(id.to_owned()))
    }

    /// Make a card kind from an id, or `None` if the id is not label-safe.
    pub fn card(id: &str) -> Option<WindowKind> {
        valid_id(id).then(|| WindowKind::Card(id.to_owned()))
    }

    /// The ARC-3 window attributes for this kind.
    pub fn spec(&self) -> WindowSpec {
        let label = self.label();
        match self {
            WindowKind::Brain => WindowSpec {
                label,
                url: "brain.html",
                title: "Bottle brain",
                visible: false,
                transparent: false,
                decorations: false,
                always_on_top: false,
                focusable: false,
                skip_taskbar: true,
                resizable: false,
                shadow: false,
                width: 1,
                height: 1,
            },
            WindowKind::Hub => WindowSpec {
                label,
                url: "hub.html",
                title: "Bottle",
                visible: true,
                transparent: false,
                decorations: true,
                always_on_top: false,
                focusable: true,
                skip_taskbar: false,
                resizable: true,
                shadow: true,
                width: 960,
                height: 640,
            },
            // TODO(#11): macOS non-activating NSPanel. TODO(#80): Windows WS_EX_NOACTIVATE.
            // TODO(#81): click-through outside the sprite hit shape.
            WindowKind::Mog(_) => WindowSpec {
                label,
                url: "mog.html",
                title: "MOG",
                visible: true,
                transparent: true,
                decorations: false,
                always_on_top: true,
                focusable: false,
                skip_taskbar: true,
                resizable: false,
                shadow: false,
                width: 160,
                height: 160,
            },
            // TODO(#78): key-capable non-activating panel at Floating level, lazy create, reaper.
            WindowKind::Card(_) => WindowSpec {
                label,
                url: "card.html",
                title: "MOG card",
                visible: true,
                transparent: false,
                decorations: false,
                always_on_top: true,
                focusable: true,
                skip_taskbar: true,
                resizable: false,
                shadow: true,
                width: 360,
                height: 240,
            },
        }
    }
}

/// Platform seam for window transparency (SP-01, #10).
mod transparency {
    use tauri::{Runtime, WebviewWindowBuilder};

    /// Apply the transparent flag where the platform allows it without `macOSPrivateApi`.
    ///
    /// On macOS, Tauri gates `transparent` behind the `macos-private-api` feature. NFR-14b
    /// forbids it unless SP-01 fails, so macOS gets no transparency here yet.
    /// TODO(#10): apply the SP-01 result (own objc2 fix, or `macOSPrivateApi` as the fallback).
    pub fn apply<'a, R: Runtime, M: tauri::Manager<R>>(
        builder: WebviewWindowBuilder<'a, R, M>,
        transparent: bool,
    ) -> WebviewWindowBuilder<'a, R, M> {
        #[cfg(not(target_os = "macos"))]
        {
            builder.transparent(transparent)
        }
        #[cfg(target_os = "macos")]
        {
            let _ = transparent;
            builder
        }
    }
}

/// Make a window from a spec. Returns the existing window if the label is already open.
pub fn build<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    spec: &WindowSpec,
) -> tauri::Result<tauri::WebviewWindow<R>> {
    use tauri::Manager;
    if let Some(existing) = app.get_webview_window(&spec.label) {
        return Ok(existing);
    }
    let builder = tauri::WebviewWindowBuilder::new(
        app,
        spec.label.clone(),
        tauri::WebviewUrl::App(spec.url.into()),
    )
    .title(spec.title)
    .visible(spec.visible)
    .decorations(spec.decorations)
    .always_on_top(spec.always_on_top)
    .focused(spec.focusable && spec.visible)
    .focusable(spec.focusable)
    .skip_taskbar(spec.skip_taskbar)
    .resizable(spec.resizable)
    .shadow(spec.shadow)
    .inner_size(f64::from(spec.width), f64::from(spec.height));
    transparency::apply(builder, spec.transparent).build()
}
