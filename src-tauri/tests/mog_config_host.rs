//! #25: the host keeps the MOG config store in the app config dir (MB-02, ARC-4).

use bottle_lib::mog_config_host::{install, store_dir, STORE_DIR};
use bottle_lib::windows::WindowKind;
use mog_core::mog_config::{valid_id, MogConfigStore};
use std::path::Path;
use std::sync::Arc;
use tauri::Manager;

#[test]
fn mb_02_store_dir_is_under_the_app_config_dir() {
    let base = Path::new("cfg-root");
    assert_eq!(store_dir(base), base.join(STORE_DIR));
    assert_eq!(STORE_DIR, "mogs");
}

#[test]
fn mb_02_host_manages_one_store_in_the_app_config_dir() {
    let app = tauri::test::mock_app();
    install(app.handle()).unwrap();
    let store = app.state::<Arc<MogConfigStore>>();
    let expected = store_dir(&app.path().app_config_dir().unwrap());
    assert_eq!(store.dir(), expected.as_path());
    // Nothing is written until the first config_write.
    assert_eq!(store.list().unwrap().items.len(), 0);
}

#[test]
fn mb_02_config_ids_are_mog_window_ids() {
    // Every config ID also names its `mog-<id>` window. Config IDs are a stricter subset:
    // lower case only and no Windows device name, so two IDs never share one file.
    let long = "a".repeat(65);
    for id in [
        "a",
        "8f14e45f-ceea-467f-a0e6-2b4c1d0a9e11",
        "",
        "a.b",
        "a/b",
        "a_b",
        "\u{fc}",
        "Upper",
        "con",
        long.as_str(),
        &long[..64],
    ] {
        if valid_id(id) {
            assert!(WindowKind::mog(id).is_some(), "{id:?}");
        }
    }
    assert!(valid_id("8f14e45f-ceea-467f-a0e6-2b4c1d0a9e11") && valid_id(&long[..64]));
    assert!(WindowKind::mog("Upper").is_some() && !valid_id("Upper"));
}
