// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = tyba_lib::hook_ipc::maybe_run_hook_mode() {
        std::process::exit(code);
    }
    if let Some(code) = tyba_lib::agent::statusline::maybe_run_statusline_mode() {
        std::process::exit(code);
    }
    if let Some(code) = tyba_lib::agent::browser_bridge::maybe_run_open_url_mode() {
        std::process::exit(code);
    }
    if let Some(code) = tyba_lib::hook_ipc::channel::maybe_run_jail_mode() {
        std::process::exit(code);
    }
    if let Some(code) = tyba_lib::agent::codex_shim::maybe_run_codex_mode() {
        std::process::exit(code);
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Some(code) = tyba_lib::agent::claude_plugin::maybe_run_plugin_hook() {
        std::process::exit(code);
    }
    if let Some(code) = tyba_lib::sandbox::bwrap::maybe_run_seccomp_exec() {
        std::process::exit(code);
    }
    // Só o app: os modos acima rodam dentro de um pane e precisam do env dele.
    tyba_lib::agent::inherited_env::scrub_inherited_session_markers();
    tyba_lib::run()
}
