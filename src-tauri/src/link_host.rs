//! `link_open` host (#523): manages the [`LinkOpenComponent`] over the one T7 and the OS opener.
//! [`crate::profiles_host::install`] runs first. Without T7 the component is not managed and
//! `link_open` answers `host_failed`.

use std::sync::Arc;

use mog_core::http_transport::ProfileHosts;
use mog_core::link_open::LinkOpenComponent;
use mog_core::recording_cloud::opener::SystemOpener;
use tauri::Manager;

use crate::profiles_host::ProfilesHost;

/// Manage the link opener. `AllowedAppUrls` has no policy source yet, so no extra host is trusted.
pub fn install(handle: &tauri::AppHandle<tauri::Wry>) {
    let Some(hosts) = handle
        .try_state::<ProfilesHost>()
        .map(|p| p.0.clone() as Arc<dyn ProfileHosts>)
    else {
        return;
    };
    handle.manage(Arc::new(LinkOpenComponent::new(
        hosts,
        Arc::new(SystemOpener),
        Vec::new(),
    )));
}
