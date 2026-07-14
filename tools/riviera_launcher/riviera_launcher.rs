//! riviera_sim_test launcher.
//!
//! Reads the multiline params file the rule wrote, resolves every
//! rlocation-key argument via the runfiles library, composes the
//! `.do` script asim runs, invokes vsimsa in a HOME under
//! `$TEST_TMPDIR`, then runs any coverage / post-process hooks.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use acdb2lcov::{emit_lcov, parse_acdb_text};
use clap::Parser;
use riviera_common::{has_error_marker, merge_output};
use runfiles::{rlocation, Runfiles};

/// Sentinel the rule prepends to `--asim-arg` and `env` values that must
/// be resolved via runfiles at test time.
const RLOC_PREFIX: &str = "@rloc:";

const COVERAGE_ACDB: &str = "coverage.acdb";
const COVERAGE_TEXT: &str = "coverage.txt";
const COVERAGE_HTML_DIR: &str = "coverage_html";

/// User-facing env var carrying the simulation seed, in both directions.
///
/// **As an input:** when set (e.g. `bazel test
/// --test_env=RIVIERA_SEED=42 ...`, or from a pre_processor env
/// fragment), the launcher uses this seed verbatim instead of generating
/// a fresh one — letting users reproduce a flaky run once they've
/// captured its seed from the test log. Parsed as i32 (see
/// [`pick_seed`]); anything unparseable falls back to a fresh random
/// seed with a warning.
///
/// **As an output:** the launcher re-exports the resolved seed under
/// this name before spawning vsimsa, so it is always readable by the
/// simulation and anything it spawns, including on runs where the
/// launcher picked the value itself.
const SEED_VAR: &str = "RIVIERA_SEED";

/// Pick the simulation seed: `$RIVIERA_SEED` when set + parseable,
/// otherwise a fresh value derived from wall-clock time XOR pid so
/// concurrent test actions on the same worker don't collide.
///
/// Return type is `i32` because asim's `-random_seed` and `-sv_seed`
/// both parse their argument as a signed 32-bit integer; values above
/// `i32::MAX` (2^31 - 1) trigger a SCRIPTER "Unexpected value" error
/// and abort elaboration. We generate in the positive half so users
/// never see leading `-` in reproduction commands (Riviera accepts
/// negative seeds but it's a UX papercut).
///
/// The seed is deliberately printed to stdout — captured by Bazel into
/// the test log — so debuggers can copy it into
/// `--test_env=RIVIERA_SEED=<seed>` to reproduce.
fn pick_seed() -> i32 {
    if let Ok(raw) = env::var(SEED_VAR) {
        match raw.parse::<i32>() {
            Ok(seed) => return seed,
            Err(e) => {
                eprintln!(
                    "riviera_launcher: {SEED_VAR}={raw:?} is not a valid i32 ({e}); \
                     falling back to a fresh random seed"
                );
            }
        }
    }
    // Mask to 31 bits, keeping the result in the positive i32 range.
    (riviera_common::process_nonce() & 0x7FFF_FFFF) as i32
}

#[derive(Parser, Debug)]
#[command(about, long_about = None)]
struct Args {
    /// rlocationpath of the vsimsa executable.
    #[arg(long)]
    vsimsa_rloc: String,

    /// One entry per riviera_library dep, formatted `<name>:<rloc>`
    /// where `<name>` is the RivieraLibraryInfo.name the dep advertises
    /// and `<rloc>` is the rlocationpath of its `library_dir`
    /// TreeArtifact. The launcher generates `vmap <name> <abs_dir>` per
    /// entry so asim resolves `<name>.<top>` at elaboration.
    #[arg(long = "library")]
    libraries: Vec<String>,

    /// Fully-qualified top-level unit (e.g. "my_lib.my_tb").
    #[arg(long)]
    top: String,

    /// Additional fully-qualified top-level units appended to the
    /// primary `--top` on the asim command line. Used to co-elaborate
    /// helper modules like Xilinx `xil_defaultlib.glbl` — asim
    /// instantiates each extra top at simulator root so their internal
    /// signals (`glbl.GSR`, `glbl.GTS`) resolve for hierarchical
    /// references from user HDL (`xpm_cdc`, `xpm_fifo`, etc.).
    /// Multiple `--extra-top` args stack; order matches CLI order.
    #[arg(long = "extra-top")]
    extra_tops: Vec<String>,

    /// -g<name>=<value> elaboration parameters.
    #[arg(long = "generic")]
    generics: Vec<String>,

    /// Extra asim flags. Values prefixed with `@rloc:` are resolved via
    /// the runfiles library at test time; everything else is forwarded
    /// verbatim.
    ///
    /// `allow_hyphen_values` — asim's own CLI is hyphen-prefixed (`-f`,
    /// `-pli`, `-loadvhpi`, …); without this clap treats a leading `-`
    /// as a new option and rejects the arg with "unexpected argument".
    #[arg(long = "asim-arg", allow_hyphen_values = true)]
    asim_args: Vec<String>,

    /// Whether Bazel is collecting coverage; adds `-acdb -acdb_cov sbceam`
    /// to `asim` and `acdb report -txt/-html` commands after `run -all`.
    #[arg(long)]
    coverage: bool,

    /// rlocationpath of an optional post-process executable.
    #[arg(long)]
    post_process_rloc: Option<String>,

    /// rlocationpath of an optional pre-process file to invoke before
    /// spawning vsimsa. Runs in $TEST_TMPDIR; may write
    /// `$TEST_TMPDIR/pre_processor_env` to inject env vars into the
    /// simulator's environment.
    #[arg(long)]
    pre_process_rloc: Option<String>,

    /// Launch the interactive Riviera-PRO IDE instead of batch vsimsa.
    /// Requires `--riviera-rloc`. The `.do` script switches to
    /// interactive mode (`log -recursive`, `add wave`, no `run -all`
    /// or `quit`) so the user can drive time forward from the console.
    /// Batch-mode output-scanning + coverage post-processing are
    /// skipped — GUI runs aren't tests.
    #[arg(long)]
    gui: bool,

    /// rlocationpath of the `riviera` executable (the interactive IDE).
    /// Required when `--gui` is set; ignored otherwise.
    #[arg(long)]
    riviera_rloc: Option<String>,

    /// Region under `sim:/<top>/` to preload into the wave viewer when
    /// `--gui` is set (`add wave sim:/<top>/<region>/*`). Empty string
    /// means add the top's own signals (`sim:/<top>/*`); typical
    /// non-empty value is `tb` when top is an SV outer wrapper that
    /// instantiates the real testbench as `tb`. Ignored in batch mode.
    #[arg(long, default_value = "")]
    gui_wave_region: String,
}

/// Read one `.cfg` file (not necessarily named `library.cfg`), emit a
/// `vmap <name> <abs_path>` line to `out` for every mapping it declares,
/// and recurse through its `$INCLUDE` directives.
///
/// Relative paths resolve against `resolve_dir` — matching how the
/// compiler wrote them, but diverging from Aldec's own `vmap -link`
/// which resolves against CWD (session-level cfg location, wrong for
/// our sandbox layout).
///
/// Returns every library name emitted, so the caller can pass them to
/// asim via `-L <name>` (elaboration-time search-path entries — without
/// them, primitives referenced by bare name in generated Xilinx IP/BD
/// source like `BUFG_GT` in `unisims_ver` fail with
/// `ELBREAD_0081 ... not found in searched libraries`).
///
/// `.cfg` grammar handled:
///   `$INCLUDE = "<path>"` — recurse; `<path>` relative to `resolve_dir`.
///   `<name> = "<path>" [<timestamp>]` — emit `vmap <name> <abs_path>`.
///   `#` / blank lines — skipped.
///
/// Aldec system-cfg magic (`$INCLUDE = "$VSIMSALIBRARYCFG"`) is skipped —
/// the base Aldec install already loads it; re-linking would recurse.
fn emit_vmap_from_cfg_file(out: &mut String, cfg_path: &Path, resolve_dir: &Path) -> Vec<String> {
    let contents = match fs::read_to_string(cfg_path) {
        Ok(s) => s,
        Err(e) => panic!("failed to read {}: {}", cfg_path.display(), e),
    };
    let mut names = Vec::new();
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        // Extract the quoted path (first "...") from the RHS.
        let rest = rest.trim();
        let (quoted, _tail) = match rest.strip_prefix('"').and_then(|r| r.split_once('"')) {
            Some(pair) => pair,
            None => continue,
        };
        if key == "$INCLUDE" {
            // Skip the Aldec-provided system cfg — recursion loops otherwise.
            if quoted.starts_with('$') {
                continue;
            }
            let nested = resolve_dir.join(quoted);
            // Aldec's `$INCLUDE` accepts either a path to a `library.cfg`
            // FILE or to a DIRECTORY (in which case the .cfg is inside
            // it). Mirror both here.
            let (cfg_file, next_dir) = if nested.is_dir() {
                (nested.join("library.cfg"), nested.clone())
            } else {
                let parent = nested.parent().unwrap_or(resolve_dir).to_path_buf();
                (nested.clone(), parent)
            };
            names.extend(emit_vmap_from_cfg_file(out, &cfg_file, &next_dir));
        } else {
            let abs = resolve_dir.join(quoted);
            writeln!(out, "vmap {} {}", key, abs.display()).unwrap();
            names.push(key.to_string());
        }
    }
    names
}

fn resolve(runfiles: &Runfiles, key: &str) -> PathBuf {
    let path =
        rlocation!(runfiles, key).unwrap_or_else(|| panic!("failed to resolve runfile: {key}"));
    if !path.exists() {
        panic!(
            "resolved runfile does not exist: {key} -> {}",
            path.display()
        );
    }
    path
}

fn build_do_script(args: &Args, runfiles: &Runfiles, seed: i32) -> String {
    let mut out = String::new();
    out.push_str("# Auto-generated by rules_riviera riviera_sim_test launcher.\n");
    // `onerror` differs by mode: batch quits with a non-zero exit (Bazel
    // reads that as test failure); GUI stays open so the user can
    // inspect the failing state in the wave viewer / TCL console.
    if args.gui {
        out.push_str("onerror { resume }\n");
    } else {
        out.push_str("onerror { quit -code 1 }\n");
    }

    // Load every library the dep's library.cfg advertises, resolving
    // relative paths against the cfg's own dir (Aldec's `vmap -link`
    // resolves them against the SESSION's CWD instead — wrong for our
    // sandboxed layout). Read the .cfg here, walk `$INCLUDE`s
    // transitively, and emit `vmap <name> <abs_path>` per mapping. This
    // works for both single-lib outputs (riviera_library) and multi-lib
    // packagings — a precompiled vendor bundle that uses `$INCLUDE`
    // chains to ship Xilinx simlibs alongside user HDL, say.
    let mut lib_search_names: Vec<String> = Vec::new();
    for entry in &args.libraries {
        let (name, rloc) = entry.split_once(':').unwrap_or_else(|| {
            panic!("malformed --library entry (expected <name>:<rloc>): {entry}")
        });
        let dir = resolve(runfiles, rloc);
        writeln!(out, "# link library `{}` from {}", name, dir.display()).unwrap();
        lib_search_names.extend(emit_vmap_from_cfg_file(
            &mut out,
            &dir.join("library.cfg"),
            &dir,
        ));
    }
    // Dedup while preserving first-occurrence order. Xilinx simlibs like
    // `xpm` show up in multiple precompiled bundles (base + xpm bundle)
    // and asim rejects duplicate `-L <name>` (it errors "Library %s
    // already added"). `work` and `worklib` are Aldec built-in defaults
    // — asim also rejects `-L work` explicitly, so filter them out.
    let mut seen = std::collections::HashSet::new();
    lib_search_names.retain(|name| {
        if name == "work" || name == "worklib" {
            return false;
        }
        seen.insert(name.clone())
    });

    // Batch mode passes `-c` (console-only, no GUI init). GUI mode
    // omits it so asim initializes with wave-viewer / TCL-console
    // hookups the interactive `riviera` binary needs.
    let mut asim = String::from(if args.gui { "asim" } else { "asim -c" });
    if args.coverage {
        asim.push_str(concat!(
            " -acdb -acdb_file ",
            "coverage.acdb",
            " -acdb_cov sbceam",
        ));
    }
    // `-L <name>` per mapped library. Without this asim only searches
    // `work` for module resolution during elaboration, so bare
    // references like `BUFG_GT` (lives in `unisims_ver`) or `xpm_fifo`
    // (in `xpm`) inside Xilinx-generated IP/BD source fail with
    // `ELBREAD_0081 ... not found in searched libraries: work.`.
    for name in &lib_search_names {
        write!(asim, " -L {name}").unwrap();
    }
    // `-random_seed` seeds `$random`/`$urandom` and any Riviera
    // deterministic-random plumbing; `-sv_seed` is the SV-side equivalent.
    // Both take the same seed so users chasing a flake only need to lock
    // one value via `$RIVIERA_SEED`.
    write!(asim, " -random_seed {seed} -sv_seed {seed}").unwrap();
    for raw in &args.asim_args {
        asim.push(' ');
        if let Some(key) = raw.strip_prefix(RLOC_PREFIX) {
            write!(asim, "{}", resolve(runfiles, key).display()).unwrap();
        } else {
            asim.push_str(raw);
        }
    }
    // `-G`, not `-g`: lowercase binds only generics declared on the
    // elaboration top, uppercase binds by name at every level of the
    // hierarchy. Tops that are thin shims — a SystemVerilog module
    // instantiating the real testbench, say — declare no generics of
    // their own, so `-g` would silently bind nothing.
    for g in &args.generics {
        write!(asim, " -G{g}").unwrap();
    }
    asim.push(' ');
    asim.push_str(&args.top);
    for extra in &args.extra_tops {
        asim.push(' ');
        asim.push_str(extra);
    }

    out.push_str(&asim);
    out.push('\n');

    if args.gui {
        // Interactive tail: log the whole hierarchy for wave visibility,
        // preload a default region of waves, then leave the sim at
        // `run 0` so the user drives it manually. NO `run -all` (the
        // whole point is stepping through) and NO `quit` (the user
        // closes the IDE when done). Wave region is caller-selectable —
        // point it at the inner DUT when the top is a thin wrapper, so
        // the default view is on the interesting signals rather than an
        // empty shell.
        //
        // Guard the pre-populated waves against elaboration-time errors
        // (missing top, unbound components, etc.) so the .do finishes
        // even when the design is broken — otherwise the IDE opens with
        // an empty console and no way to poke at the state.
        writeln!(out, "catch {{ log -recursive sim:/{}/* }}", args.top).unwrap();
        let region_suffix = if args.gui_wave_region.is_empty() {
            String::new()
        } else {
            format!("{}/", args.gui_wave_region)
        };
        writeln!(
            out,
            "catch {{ add wave sim:/{}/{}* }}",
            args.top, region_suffix
        )
        .unwrap();
    } else {
        out.push_str("run -all\n");

        // Coverage reports run inside the same vsimsa invocation because
        // `acdb` isn't a standalone binary in every Riviera install — it's
        // a TCL command on `vsimsa`. `endsim` first, so vsimsa flushes the
        // in-progress `.acdb` before `acdb report` tries to read it —
        // otherwise the file "does not exist" (it's only finalized on
        // simulation end). HTML is wrapped in `catch` because Riviera
        // skips it silently when there's nothing coverable and we don't
        // want that to fail the test.
        if args.coverage {
            out.push_str("endsim\n");
            // Riviera's `acdb report` takes the input via `-i` and the
            // format as `-txt` / `-html` (NOT `-text`); getting either
            // wrong yields a SCRIPTER syntax error, not a nice message.
            writeln!(
                out,
                "acdb report -txt -i {COVERAGE_ACDB} -o {COVERAGE_TEXT}"
            )
            .unwrap();
            writeln!(
                out,
                "catch {{ acdb report -html -i {COVERAGE_ACDB} -o {COVERAGE_HTML_DIR} }}"
            )
            .unwrap();
        }

        out.push_str("quit -code 0\n");
    }
    out
}

fn run_vsimsa(
    vsimsa: &Path,
    do_path: &Path,
    home: &Path,
    work_dir: &Path,
) -> std::io::Result<(i32, Vec<u8>)> {
    let mut cmd = Command::new(vsimsa);
    cmd.arg("-do").arg(do_path);
    cmd.env("HOME", home);
    // vsimsa emits coverage.acdb / coverage.txt / coverage_html/ into
    // its CWD; anchor it to TEST_TMPDIR so post-processing knows where
    // to look and Bazel wipes it between runs.
    cmd.current_dir(work_dir);
    let output = cmd.output()?;

    let log = merge_output(&output);
    std::io::stdout().write_all(&log)?;
    std::io::stdout().flush()?;

    Ok((output.status.code().unwrap_or(1), log))
}

/// Spawn the interactive Riviera-PRO IDE with `-do <do_path>`.
///
/// Unlike `run_vsimsa`, this doesn't capture stdout/stderr — the IDE's
/// console prompts need to reach the user's terminal live, and there's
/// no test log to persist. `HOME` is INHERITED (not overridden to a
/// scratch dir) so the user's `~/.aldec/` preferences and saved wave
/// configurations persist across GUI launches. `DISPLAY` /
/// `XAUTHORITY` propagate via `Command`'s default env inheritance.
fn run_riviera_gui(riviera: &Path, do_path: &Path, work_dir: &Path) -> std::io::Result<i32> {
    let status = Command::new(riviera)
        .arg("-do")
        .arg(do_path)
        .current_dir(work_dir)
        .status()?;
    Ok(status.code().unwrap_or(1))
}

/// Move coverage.txt + coverage_html/ (produced inside vsimsa) into
/// $TEST_UNDECLARED_OUTPUTS_DIR, then run acdb2lcov on the text report
/// to feed $COVERAGE_OUTPUT_FILE for Bazel's combined lcov.
fn post_process_coverage(work_dir: &Path) -> std::io::Result<()> {
    let text_report = work_dir.join(COVERAGE_TEXT);
    if !text_report.exists() {
        // Sim didn't produce a report — likely no coverable items or
        // coverage wasn't wired up. Nothing to do.
        return Ok(());
    }
    let html_dir = work_dir.join(COVERAGE_HTML_DIR);

    // Human-readable artefacts stashed in the test-outputs zip.
    if let Ok(undeclared) = env::var("TEST_UNDECLARED_OUTPUTS_DIR") {
        let und = PathBuf::from(&undeclared);
        fs::create_dir_all(&und)?;
        let _ = fs::copy(&text_report, und.join(COVERAGE_TEXT));
        if html_dir.exists() {
            let dst = und.join(COVERAGE_HTML_DIR);
            let _ = fs::remove_dir_all(&dst);
            let _ = copy_dir_all(&html_dir, &dst);
        }
    }

    // lcov: only runs when Bazel is collecting coverage.
    let Ok(coverage_out) = env::var("COVERAGE_OUTPUT_FILE") else {
        return Ok(());
    };
    let text = fs::read_to_string(&text_report)?;
    fs::write(coverage_out, emit_lcov(&parse_acdb_text(&text)))?;
    Ok(())
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

/// Env var whose value is the file path the pre_processor writes
/// its `K=V` env fragment to. The launcher chooses the path, exposes
/// it to the pre_processor via this variable, and reads back the file
/// after the pre_processor exits successfully. Making the path explicit
/// via env (rather than a hardcoded convention) keeps the pre_processor
/// side agnostic to launcher's tmpdir layout.
const PRE_PROCESSOR_ENV_FILE_VAR: &str = "RIVIERA_PRE_PROCESSOR_ENV_FILE";

/// Invoke the pre_processor. Runs with CWD=$TEST_TMPDIR, inheriting
/// the launcher's env (including RUNFILES_DIR / RUNFILES_MANIFEST_FILE
/// so it can resolve its own runfiles). Also sets
/// `RIVIERA_PRE_PROCESSOR_ENV_FILE` so the pre_processor knows where to
/// write env vars destined for vsimsa.
fn run_pre_processor(
    pre_processor: &Path,
    work_dir: &Path,
    env_file: &Path,
) -> std::io::Result<i32> {
    let status = Command::new(pre_processor)
        .current_dir(work_dir)
        .env(PRE_PROCESSOR_ENV_FILE_VAR, env_file)
        .status()?;
    Ok(status.code().unwrap_or(1))
}

/// Read the pre_processor env fragment file (if present) and inject
/// each `K=V` line into the current process's environment so
/// `Command::env_clear`-free child spawns pick them up. Silently
/// skipped if the file doesn't exist — the pre_processor may have
/// nothing to contribute.
///
/// Lines are trimmed; blank lines and lines starting with `#` are
/// skipped. Anything without a `=` fails the test (misformatted
/// fragment = bug in the pre_processor).
fn load_pre_processor_env(env_file: &Path) -> std::io::Result<()> {
    let content = match fs::read_to_string(env_file) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for (lineno, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "{}: line {}: expected K=V, got {:?}",
                    env_file.display(),
                    lineno + 1,
                    line,
                ),
            ));
        };
        env::set_var(k.trim(), v.trim());
    }
    Ok(())
}

/// Resolve `@rloc:`-tagged environment variables in place.
///
/// The rule tags any `env` value that changed under `$(rlocationpath(s))`
/// expansion with [`RLOC_PREFIX`], since the rlocation path it expands to
/// only means anything once the runfiles tree exists at test time. The
/// tag covers the whole value, so a multi-label `$(rlocationpaths ...)`
/// arrives as one marked, space-separated list — resolve every token.
///
/// Runs before the pre_processor so both it and vsimsa see real paths.
/// A pre_processor reading such a variable is the common case: it is how
/// a harness locates data files it has to stage before the sim starts.
fn resolve_env_rlocations(runfiles: &Runfiles) {
    let tagged: Vec<(String, String)> = env::vars()
        .filter_map(|(k, v)| {
            v.strip_prefix(RLOC_PREFIX)
                .map(|rest| (k, rest.to_string()))
        })
        .collect();
    for (key, value) in tagged {
        let resolved = value
            .split_whitespace()
            .map(|token| resolve(runfiles, token).display().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        env::set_var(key, resolved);
    }
}

fn main() -> ExitCode {
    let runfiles = Runfiles::create().expect("failed to initialise the runfiles library");
    resolve_env_rlocations(&runfiles);

    let args_file_key = env::var("RIVIERA_ARGS_FILE_RLOC").expect("RIVIERA_ARGS_FILE_RLOC not set");
    let args_file = resolve(&runfiles, &args_file_key);
    let raw = fs::read_to_string(&args_file).expect("read args file");
    let args = Args::parse_from(std::iter::once("riviera_launcher").chain(raw.lines()));

    let tmpdir = env::var("TEST_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    // Pre-process before generating the .do — a pre_processor that
    // fails should shortcut before we do vsimsa setup. Also lets the
    // pre_processor inject env vars (by writing K=V lines to the file
    // named by $RIVIERA_PRE_PROCESSOR_ENV_FILE) into the environment
    // vsimsa inherits.
    if let Some(rloc) = args.pre_process_rloc.as_ref() {
        let pre = resolve(&runfiles, rloc);
        let env_file = tmpdir.join("pre_processor_env");
        let rc = run_pre_processor(&pre, &tmpdir, &env_file).expect("spawn pre_processor");
        if rc != 0 {
            eprintln!("pre_processor exited with status {rc}");
            return ExitCode::from(rc as u8);
        }
        if let Err(e) = load_pre_processor_env(&env_file) {
            eprintln!("failed to load pre_processor env fragment: {e}");
            return ExitCode::from(1);
        }
    }

    // Pick the seed AFTER pre_processor runs — the pre_processor may
    // have set $RIVIERA_SEED via its env fragment, which is how a
    // caller-supplied harness can compute or forward a seed of its own.
    // Print it plainly to stdout so it lands in the Bazel test log;
    // anyone chasing a flake can grep for `riviera_sim_test seed:` and
    // copy the value back into --test_env.
    let seed = pick_seed();
    println!("riviera_sim_test seed: {seed}");
    env::set_var(SEED_VAR, seed.to_string());

    let do_path = tmpdir.join("run.do");
    fs::write(&do_path, build_do_script(&args, &runfiles, seed)).expect("write .do script");

    if args.gui {
        // GUI path: spawn the interactive IDE. No stdout/stderr scrape,
        // no coverage post-processing, no error-marker check — the user
        // is in the loop. Exit code is Riviera's own.
        //
        // No rule in this repo sets `--gui`; `riviera_sim_test` is always
        // batch. The flag, `--riviera-rloc`, `--gui-wave-region` and the
        // `.do` branch they drive exist so a consumer-authored rule can
        // launch the IDE over the same params file, which is also why the
        // toolchain keeps a `riviera` attr nothing here invokes.
        let riviera_rloc = args
            .riviera_rloc
            .as_ref()
            .expect("--gui requires --riviera-rloc");
        let riviera = resolve(&runfiles, riviera_rloc);
        let rc = run_riviera_gui(&riviera, &do_path, &tmpdir).expect("spawn riviera");
        return ExitCode::from(rc as u8);
    }

    // Mock HOME under TEST_TMPDIR — Bazel wipes TEST_TMPDIR between
    // runs, so no explicit cleanup here.
    let home = tmpdir.join("home");
    fs::create_dir_all(&home).expect("create HOME");

    let vsimsa = resolve(&runfiles, &args.vsimsa_rloc);
    let (rc, log) = run_vsimsa(&vsimsa, &do_path, &home, &tmpdir).expect("spawn vsimsa");
    if rc != 0 {
        eprintln!("vsimsa exited with status {rc}");
        return ExitCode::from(rc as u8);
    }
    if has_error_marker(&log) {
        eprintln!("vsimsa log contains # Error / # Fatal markers");
        return ExitCode::from(1);
    }
    if let Err(e) = post_process_coverage(&tmpdir) {
        eprintln!("coverage post-processing failed: {e}");
        return ExitCode::from(1);
    }
    if let Some(pp_rloc) = args.post_process_rloc.as_ref() {
        let post = resolve(&runfiles, pp_rloc);
        let status = Command::new(post).status().expect("spawn post_process");
        if !status.success() {
            let code = status.code().unwrap_or(1);
            eprintln!("post_process exited with status {code}");
            return ExitCode::from(code as u8);
        }
    }
    ExitCode::SUCCESS
}
