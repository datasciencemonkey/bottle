//! Open a Catalog Explorer link with the OS opener (ADR 0103). Views never get a raw URL to
//! open: Rust builds the link from the profile host and opens it. [`open_link`] opens only
//! `https://<profile host>/explore/data/volumes/...` links with a strict charset, so no other
//! host, scheme or argument can reach the opener.

use crate::error::{CoreError, CoreResult};

use super::layout::bare_https_origin;

const ID: &str = "recording_open";

fn refused() -> CoreError {
    CoreError::Unavailable {
        component: ID,
        reason: "link_refused",
    }
}

/// Opens one URL in the default browser.
pub trait UrlOpener: Send + Sync {
    /// Open `url`.
    fn open(&self, url: &str) -> CoreResult<()>;
}

/// The OS opener: `open` on macOS, `rundll32 url.dll,FileProtocolHandler` on Windows. No shell
/// runs; the URL is one argument.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemOpener;

impl UrlOpener for SystemOpener {
    fn open(&self, url: &str) -> CoreResult<()> {
        #[cfg(target_os = "macos")]
        let mut cmd = {
            let mut c = std::process::Command::new("/usr/bin/open");
            c.arg(url);
            c
        };
        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = std::process::Command::new("rundll32.exe");
            c.arg("url.dll,FileProtocolHandler").arg(url);
            c
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut cmd = {
            let mut c = std::process::Command::new("xdg-open");
            c.arg(url);
            c
        };
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|_| CoreError::Unavailable {
                component: ID,
                reason: "open_failed",
            })
    }
}

/// Open `url` only if it is a Catalog Explorer volume link on the profile host `host_url`.
pub fn open_link(opener: &dyn UrlOpener, host_url: &str, url: &str) -> CoreResult<()> {
    let origin = bare_https_origin(host_url).ok_or_else(refused)?;
    let prefix = format!("{origin}/explore/data/volumes/");
    let rest = url.strip_prefix(&prefix).ok_or_else(refused)?;
    let charset_ok = rest
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-._~%/?&=@".contains(&b));
    if !charset_ok || rest.is_empty() || url.len() > 2048 {
        return Err(refused());
    }
    opener.open(url)
}
