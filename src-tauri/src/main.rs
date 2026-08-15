#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    // The hidden RapidOCR worker is the only headless entrypoint retained by the
    // desktop executable. Pi owns the coding-agent CLI and runtime.
    let mut args = std::env::args_os();
    let _program = args.next();
    if let Some(first) = args.next() {
        if first == beefex::rapidocr::RAPIDOCR_WORKER_ARG {
            return beefex::rapidocr::run_worker_entry(args);
        }
    }

    beefex::run();
    ExitCode::SUCCESS
}
