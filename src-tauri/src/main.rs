// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Genie Code starts `bottle --agent-mcp-shim` as the MCP server of the one `ask_genie` tool
    // (ADR 0119). The shim runs no app code: it ends here with its own exit code.
    #[cfg(unix)]
    if std::env::args().nth(1).as_deref() == Some(mog_core::agent_run::seam::SHIM_ARG) {
        std::process::exit(mog_core::agent_run::guard::shim::run());
    }
    bottle_lib::run();
}
