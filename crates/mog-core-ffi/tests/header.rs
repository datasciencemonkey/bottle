//! The committed C header `include/mog_core.h` matches the exported ABI (NFR-18a, NFR-18c).
//!
//! cbindgen (pinned in Cargo.toml) regenerates the header from `src/lib.rs`.
//! To update it: `BOTTLE_UPDATE_HEADER=1 cargo test -p mog-core-ffi --test header`.

use std::fs;
use std::path::Path;

#[test]
fn nfr_18a_c_header_matches_the_exported_abi() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config = cbindgen::Config::from_file(dir.join("cbindgen.toml")).expect("cbindgen.toml");
    let bindings = cbindgen::Builder::new()
        .with_config(config)
        .with_src(dir.join("src/lib.rs"))
        .generate()
        .expect("cbindgen generates the header");
    let mut generated = Vec::new();
    bindings.write(&mut generated);
    let path = dir.join("include/mog_core.h");
    if std::env::var_os("BOTTLE_UPDATE_HEADER").is_some() {
        fs::write(&path, &generated).expect("write the header");
    }
    let committed = fs::read(&path).unwrap_or_default();
    assert_eq!(
        String::from_utf8_lossy(&committed),
        String::from_utf8_lossy(&generated),
        "include/mog_core.h is out of date: run BOTTLE_UPDATE_HEADER=1 cargo test -p mog-core-ffi --test header"
    );
}

#[test]
fn nfr_18c_c_header_declares_every_abi_function() {
    let header =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("include/mog_core.h"))
            .unwrap_or_default();
    for item in [
        "uint32_t mog_abi_version(void);",
        "int32_t mog_init(const char *config_json);",
        "char *mog_call(const char *request_json);",
        "void mog_free_string(char *s);",
        "int32_t mog_set_event_callback(MogEventCallback callback, void *user_data);",
        "typedef void (*MogEventCallback)(const char *event_json, void *user_data);",
        "#define MOG_ABI_VERSION 1",
        "#define MOG_EVENT_QUEUE_CAPACITY",
        // N3, N4: the host rules for the callback.
        "before it frees `user_data`",
        "does not support unloading",
        "while it holds a lock that the callback takes",
    ] {
        assert!(header.contains(item), "the header lacks `{item}`");
    }
}
