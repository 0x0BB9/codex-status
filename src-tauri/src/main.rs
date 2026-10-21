// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(result) = codex_status_floater_lib::run_history_bridge_if_requested() {
        match result {
            Ok(code) => std::process::exit(code),
            Err(error) => {
                eprintln!("History compatibility bridge failed: {error}");
                std::process::exit(1);
            }
        }
    }
    codex_status_floater_lib::run()
}
