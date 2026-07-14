//! Wrapper around `vsimsa` for the `RivieraCompile` Bazel action.
//!
//! Three responsibilities on top of a bare `vsimsa -do <script>` call:
//!
//! 1. **Isolate HOME.** Riviera writes stateful things to `$HOME/.aldec/`
//!    (license cache, preferences, wave configs) during compilation. If
//!    actions inherit the developer's real HOME they contaminate the
//!    interactive `vsim` GUI's state. Point `HOME` at a private subdir
//!    of `TMPDIR` instead, and remove it on the way out.
//!
//! 2. **Silence unless failed.** vsimsa is chatty even on clean builds
//!    (version banner, license info, per-command status). Bazel already
//!    swallows action stdout on success; buffer here so we can (a) grep
//!    for `# Error`/`# Fatal` markers that vsimsa doesn't propagate as
//!    a non-zero exit, and (b) never stream the log unless something
//!    actually went wrong. On failure dump the full captured log.
//!
//! 3. **Write library.cfg.** After a clean compile, drop a `library.cfg`
//!    inside the output library dir naming the one library. That makes
//!    the dir a Riviera "library home", so downstream `vmap -link <dir>`
//!    picks the library up without needing to know its logical name.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::Parser;
use riviera_common::{has_error_marker, merge_output};

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

    /// Extra args passed through to vsimsa.
    #[arg(trailing_var_arg = true)]
    forwarded: Vec<OsString>,
}

fn write_library_cfg(library_dir: &Path, library_name: &str) -> std::io::Result<()> {
    let cfg = library_dir.join("library.cfg");
    let contents = format!(
        "$INCLUDE = \"$VSIMSALIBRARYCFG\"\n{name} = \"./{name}.lib\"\n",
        name = library_name,
    );
    fs::write(cfg, contents)
}

/// Create a private `$HOME` for this invocation.
///
/// Under a sandboxing spawn strategy `TMPDIR` is already per-action, but
/// not every strategy sandboxes (`--spawn_strategy=local`, persistent
/// remote workers reusing a scratch dir), so the directory name has to
/// carry its own uniqueness — otherwise two concurrent `RivieraCompile`
/// actions share one `.aldec/` and race on its license cache and
/// preference files.
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
    let mut cmd = Command::new(&args.vsimsa);
    cmd.arg("-do").arg(&args.do_script);
    for extra in &args.forwarded {
        cmd.arg(extra);
    }
    cmd.env("HOME", home);

    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("riviera_wrapper: failed to spawn vsimsa: {e}");
            return ExitCode::from(1);
        }
    };

    let log = merge_output(&output);
    let rc = output.status.code().unwrap_or(1);
    if rc == 0 && !has_error_marker(&log) {
        if let Err(e) = write_library_cfg(&args.library_dir, &args.library_name) {
            eprintln!("riviera_wrapper: failed to write library.cfg: {e}");
            return ExitCode::from(1);
        }
        return ExitCode::SUCCESS;
    }

    let stderr = std::io::stderr();
    let mut stderr = stderr.lock();
    let _ = stderr.write_all(&log);
    if rc != 0 {
        let _ = writeln!(stderr, "\nvsimsa exited with status {rc}");
        return ExitCode::from(rc as u8);
    }
    let _ = writeln!(stderr, "\nvsimsa log contains # Error / # Fatal markers");
    ExitCode::from(1)
}
