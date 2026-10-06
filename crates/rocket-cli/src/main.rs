//! `rocket` binary entry point. Only `--version` exists so far; the command
//! tree is ported in later tasks.

use clap::Parser;

/// Supervise dev processes.
#[derive(Parser)]
#[command(name = "rocket", version)]
struct Cli {}

fn main() {
    let _ = Cli::parse();
}
