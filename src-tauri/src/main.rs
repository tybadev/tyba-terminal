// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = tyba_lib::hook_ipc::maybe_run_hook_mode() {
        std::process::exit(code);
    }
    if let Some(code) = tyba_lib::sandbox::bwrap::maybe_run_seccomp_exec() {
        std::process::exit(code);
    }
    tyba_lib::run()
}
