//! `acdb report -txt` → lcov CLI.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use acdb2lcov::{emit_lcov, parse_acdb_text};

#[derive(Parser, Debug)]
#[command(about = "Convert Riviera `acdb report -txt` output to lcov.")]
struct Args {
    /// Path to the text report `acdb report -txt` produced.
    #[arg(long)]
    input: PathBuf,

    /// Destination `.info` (lcov) file.
    #[arg(long)]
    output: PathBuf,
}

fn main() -> ExitCode {
    let args = Args::parse();

    let text = match fs::read_to_string(&args.input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("acdb2lcov: failed to read {}: {e}", args.input.display());
            return ExitCode::from(1);
        }
    };
    let report = parse_acdb_text(&text);
    let lcov = emit_lcov(&report);
    if let Err(e) = fs::write(&args.output, lcov) {
        eprintln!("acdb2lcov: failed to write {}: {e}", args.output.display());
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
