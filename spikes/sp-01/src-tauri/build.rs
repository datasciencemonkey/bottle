fn main() {
    // `bundle.resources` points at ../web/vendor (filled by scripts/copy-vendor.mjs).
    // Make sure it exists so `cargo clippy` works on a clean checkout.
    let _ = std::fs::create_dir_all("../web/vendor");
    tauri_build::build()
}
