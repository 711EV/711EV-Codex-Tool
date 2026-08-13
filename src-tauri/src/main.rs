#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if codex_local_sync_lib::portable::run_elevated_helper() {
        return;
    }
    codex_local_sync_lib::run();
}
