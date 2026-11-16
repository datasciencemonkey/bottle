//! Window kinds of ARC-3 and the builder that makes them.
//!
//! The pure part (`WindowKind`, `WindowSpec`) has no Tauri dependency, so tests check
//! ARC-3 without a GUI. `build` turns a spec into a real window.
//!
//! SP-01 (#10) can change how a transparent MOG window is made on macOS. That choice
//! stays inside `transparency::apply` (see `docs/adr/0002-foundation-proceeds-before-sp01.md`).

use std::time::Duration;

/// ARC-3 / MB-03b: at most 8 MOG windows are active by default.
pub const MAX_ACTIVE_MOGS: usize = 8;

/// ARC-3 / MOG-18b: a hidden card window is destroyed after 5 min.
pub const CARD_HIDDEN_TTL: Duration = Duration::from_secs(5 * 60);

/// How often the R11 reaper looks for cards hidden for `CARD_HIDDEN_TTL`. A hidden card goes
/// between 5:00 and 5:15 after its hide (ADR 0022).
pub const CARD_REAP_TICK: Duration = Duration::from_secs(15);

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
    /// macOS: a click on the window while Bottle is not active reaches the page (MOG-01b).
    pub accept_first_mouse: bool,
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
                accept_first_mouse: false,
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
                accept_first_mouse: false,
                width: 960,
                height: 640,
            },
            // macOS: `panel()` makes it a non-activating NSPanel (#11, ADR 0009).
            // TODO(#80): Windows WS_EX_NOACTIVATE. TODO(#81): click-through outside the sprite.
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
                accept_first_mouse: true,
                width: 160,
                height: 160,
            },
            // macOS: `panel()` makes it a key-capable non-activating NSPanel (#78, ADR 0022).
            // R11 makes it on the first show and the reaper destroys it after 5 min hidden.
            // TODO(#80): the Windows card that keeps the foreground app.
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
                accept_first_mouse: false,
                width: 360,
                height: 240,
            },
        }
    }

    /// The spec the window is built with. A panel kind is built hidden: the native style turns
    /// it into a non-activating panel first and then shows it, so it never shows as a plain
    /// window that can take focus (MOG-01b).
    pub fn build_spec(&self) -> WindowSpec {
        let mut spec = self.spec();
        if self.panel().is_some() {
            spec.visible = false;
        }
        spec
    }

    /// The macOS panel attributes of this kind, or `None` for a plain window (MOG-02).
    /// The card is the only panel that can become key (MOG-03a, UX-19a).
    pub fn panel(&self) -> Option<PanelSpec> {
        match self {
            WindowKind::Mog(_) => Some(PanelSpec {
                level: PanelLevel::Status,
                can_become_key: false,
                can_become_main: false,
                nonactivating: true,
                hides_on_deactivate: false,
                join_all_spaces: true,
                stationary: true,
                full_screen_auxiliary: true,
                ignores_cycle: true,
                transparent: true,
            }),
            // Typing and the IME need a key window; `nonactivating` keeps the frontmost app
            // (MOG-03b). It shows on every Space and over fullscreen apps, like its MOG.
            WindowKind::Card(_) => Some(PanelSpec {
                level: PanelLevel::Floating,
                can_become_key: true,
                can_become_main: false,
                nonactivating: true,
                hides_on_deactivate: false,
                join_all_spaces: true,
                stationary: true,
                full_screen_auxiliary: true,
                ignores_cycle: true,
                transparent: false,
            }),
            _ => None,
        }
    }
}

/// The macOS window level of a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelLevel {
    /// `NSFloatingWindowLevel`.
    Floating,
    /// `NSStatusWindowLevel`: above normal and floating windows (MOG-02).
    Status,
}

/// What the macOS panel of a window kind must be (MOG-02), independent of AppKit.
/// `src/macos_panel.rs` applies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelSpec {
    pub level: PanelLevel,
    /// `canBecomeKeyWindow`: `false` means the panel never takes the keyboard.
    pub can_become_key: bool,
    pub can_become_main: bool,
    /// `NSWindowStyleMaskNonactivatingPanel`: a click does not activate Bottle.
    pub nonactivating: bool,
    pub hides_on_deactivate: bool,
    /// Collection behavior `canJoinAllSpaces`.
    pub join_all_spaces: bool,
    /// Collection behavior `stationary` (stays put in Mission Control and Space changes).
    pub stationary: bool,
    /// Collection behavior `fullScreenAuxiliary` (shows over a fullscreen Space).
    pub full_screen_auxiliary: bool,
    /// Collection behavior `ignoresCycle` (Cmd-` skips it).
    pub ignores_cycle: bool,
    /// The panel paints nothing itself: no background, no shadow (MOG-01a). The card is not.
    pub transparent: bool,
}

/// Platform seam for window transparency (SP-01, #10).
mod transparency {
    use tauri::{Runtime, WebviewWindowBuilder};

    /// Apply the transparent flag. Tauri 2.12 makes the window and the WebView background
    /// transparent on every desktop OS without `macOSPrivateApi` (NFR-14b).
    /// TODO(#10): apply the SP-01 result here (white-flash and halo fixes, or a fallback).
    pub fn apply<'a, R: Runtime, M: tauri::Manager<R>>(
        builder: WebviewWindowBuilder<'a, R, M>,
        transparent: bool,
    ) -> WebviewWindowBuilder<'a, R, M> {
        builder.transparent(transparent)
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
    .accept_first_mouse(spec.accept_first_mouse)
    .inner_size(f64::from(spec.width), f64::from(spec.height));
    transparency::apply(builder, spec.transparent).build()
}
