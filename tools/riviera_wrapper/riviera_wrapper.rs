//! Wrapper around `vsimsa` for the `RivieraCompile` Bazel action.
//!
//! On top of a bare `vsimsa -do <script>` call it:
//!
//! 1. Points `HOME` at a private scratch dir so Riviera's `~/.aldec/`
//!    state never touches the developer's real home.
//! 2. Prints the transcript only when the compile fails, judged by exit
//!    code and by the `# Error` / `# Fatal` lines vsimsa doesn't always
//!    turn into one.
//! 3. Drops a `library.cfg` into the output dir on success, making it a
//!    Riviera library home that downstream consumers can read.
//! 4. Maps each `--link-library-dir` (such a library home) by absolute
//!    path in a prelude `.do` that then runs the rule's script. Aldec's
//!    `vmap -link` resolves the cfg's relative entries against the CWD
//!    rather than the cfg's directory, so it is not usable here.
//! 5. Removes the session `library.cfg` vsimsa writes into its CWD, the
//!    execroot, unless it already existed; unsandboxed strategies would
//!    otherwise feed stale mappings to the next action.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::Parser;
use riviera_common::{
    library_cfg_contents, vmap_lines_from_library_dir, vsimsa_verdict, write_transcript,
    LIBRARY_CFG,
};

#[derive(Parser, Debug)]
#[command(about, long_about = None)]
struct Args {
    /// Path to the vsimsa executable.
    #[arg(long)]
    vsimsa: PathBuf,

    /// Path to the .do script vsimsa should execute.
    #[arg(long = "do")]
    do_script: PathBuf,

    /// Logical name of the library being compiled.
    #[arg(long)]
    library_name: String,

    /// Output library directory to drop `library.cfg` into.
    #[arg(long)]
    library_dir: PathBuf,

    /// Pre-compiled library homes to `vmap` before the `.do` runs.
    #[arg(long = "link-library-dir")]
    link_library_dirs: Vec<PathBuf>,
}

/// The `.do` vsimsa should run: the rule's script itself when there is
/// nothing to link, otherwise a prelude under `scratch` that maps every
/// linked library by absolute path and then runs the rule's script.
fn compose_do_script(args: &Args, scratch: &Path) -> std::io::Result<PathBuf> {
    if args.link_library_dirs.is_empty() {
        return Ok(args.do_script.clone());
    }
    let mut script = String::from("onerror { quit -code 1 }\n");
    for dir in &args.link_library_dirs {
        vmap_lines_from_library_dir(&mut script, &std::path::absolute(dir)?)?;
    }
    writeln!(
        script,
        "do {}",
        std::path::absolute(&args.do_script)?.display()
    )
    .unwrap();
    let path = scratch.join("build.do");
    fs::write(&path, script)?;
    Ok(path)
}

/// Create a private `$HOME` for this invocation. The name carries its
/// own uniqueness because not every spawn strategy gives each action its
/// own `TMPDIR`.
fn make_home() -> std::io::Result<PathBuf> {
    let home = env::temp_dir().join(format!(
        "riviera-home-{:x}",
        riviera_common::process_nonce()
    ));
    fs::create_dir_all(&home)?;
    Ok(home)
}

fn main() -> ExitCode {
    let args = Args::parse();

    let home = match make_home() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("riviera_wrapper: failed to create HOME dir: {e}");
            return ExitCode::from(1);
        }
    };
    let code = run(&args, &home);
    let _ = fs::remove_dir_all(&home);
    code
}

fn run(args: &Args, home: &Path) -> ExitCode {
    let do_script = match compose_do_script(args, home) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("riviera_wrapper: failed to prepare .do script: {e}");
            return ExitCode::from(1);
        }
    };

    // Only remove the session cfg afterwards if this run created it.
    let session_cfg_preexisted = Path::new(LIBRARY_CFG).exists();
    let output = Command::new(&args.vsimsa)
        .arg("-do")
        .arg(&do_script)
        .env("HOME", home)
        .output();
    if !session_cfg_preexisted {
        let _ = fs::remove_file(LIBRARY_CFG);
    }

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            eprintln!("riviera_wrapper: failed to spawn vsimsa: {e}");
            return ExitCode::from(1);
        }
    };

    match vsimsa_verdict(&output) {
        Ok(()) => {
            let cfg = args.library_dir.join(LIBRARY_CFG);
            if let Err(e) = fs::write(cfg, library_cfg_contents(&args.library_name)) {
                eprintln!("riviera_wrapper: failed to write {LIBRARY_CFG}: {e}");
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            let mut stderr = std::io::stderr().lock();
            let _ = write_transcript(&mut stderr, &output);
            eprintln!("\n{failure}");
            ExitCode::from(failure.exit_code())
        }
    }
}
