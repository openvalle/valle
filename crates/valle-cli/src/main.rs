//! Binary entry point: install and tune the allocator, then call `valle_cli::run`.

/// Use mimalloc for short-lived rendering buffers and prompt memory reclamation.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> std::process::ExitCode {
    valle_cli::tune_process_allocators();
    valle_cli::run()
}
