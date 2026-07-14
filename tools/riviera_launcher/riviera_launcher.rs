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
use riviera_common::{vmap_lines_from_library_dir, vsimsa_verdict, write_transcript};
use runfiles::{rlocation, Runfiles};

/// Tag the rule wraps around the rlocation key(s) a location template in
/// `asim_opts` / `env` expanded to. Every whitespace-separated key inside
/// resolves to an absolute path; text outside the tag is literal.
const RLOC_TAG_OPEN: &str = "@rloc{";
const RLOC_TAG_CLOSE: char = '}';

const COVERAGE_ACDB: &str = "coverage.acdb";
const COVERAGE_TEXT: &str = "coverage.txt";
const COVERAGE_HTML_DIR: &str = "coverage_html";

/// Simulation seed: honoured when set and parseable, otherwise generated
/// and re-exported under the same name (see the `riviera_sim_test` doc).
const SEED_VAR: &str = "RIVIERA_SEED";

/// `i32` because asim's `-random_seed` / `-sv_seed` reject anything
/// larger; generated seeds stay positive so reproduction commands never
/// need a leading `-`.
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

    /// rlocationpath of each library home (`library_dir` TreeArtifact)
    /// to `vmap` before elaboration, `link_libraries` first.
    #[arg(long = "library")]
    libraries: Vec<String>,

    /// Fully-qualified top-level unit (e.g. "my_lib.my_tb").
    #[arg(long)]
    top: String,

    /// Additional top-level units appended after `--top`.
    #[arg(long = "extra-top")]
    extra_tops: Vec<String>,

    /// Elaboration generics, each emitted as `-G<name>=<value>`.
    #[arg(long = "generic")]
    generics: Vec<String>,

    /// Extra asim flags; `@rloc{...}` tags are resolved at test time.
    /// `allow_hyphen_values` because asim's own flags start with `-`.
    #[arg(long = "asim-arg", allow_hyphen_values = true)]
    asim_args: Vec<String>,

    /// Enable ACDB coverage collection and reporting.
    #[arg(long)]
    coverage: bool,

    /// rlocationpath of an executable to run after a successful simulation.
    #[arg(long)]
    post_process_rloc: Option<String>,

    /// rlocationpath of an executable to run before vsimsa; it may write
    /// `K=V` lines to `$RIVIERA_PRE_PROCESSOR_ENV_FILE`.
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

/// Replace every `@rloc{<keys>}` tag in `value` with the resolved
/// absolute path(s), keeping the surrounding text. `resolve_key` is a
/// parameter so the splicing can be unit-tested without a runfiles tree.
fn substitute_rloc_tags(value: &str, mut resolve_key: impl FnMut(&str) -> String) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(idx) = rest.find(RLOC_TAG_OPEN) {
        out.push_str(&rest[..idx]);
        let after = &rest[idx + RLOC_TAG_OPEN.len()..];
        let Some(close) = after.find(RLOC_TAG_CLOSE) else {
            panic!("unterminated rlocation tag in {value:?}");
        };
        let resolved: Vec<String> = after[..close]
            .split_whitespace()
            .map(&mut resolve_key)
            .collect();
        out.push_str(&resolved.join(" "));
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

fn resolve_rloc_tags(runfiles: &Runfiles, value: &str) -> String {
    substitute_rloc_tags(value, |key| resolve(runfiles, key).display().to_string())
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

    // `vmap` every library each home's cfg declares, by absolute path;
    // a vendor bundle may declare many via `$INCLUDE` chains.
    let mut lib_search_names: Vec<String> = Vec::new();
    for rloc in &args.libraries {
        let dir = resolve(runfiles, rloc);
        let names = vmap_lines_from_library_dir(&mut out, &dir).unwrap_or_else(|e| panic!("{e}"));
        lib_search_names.extend(names);
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
        write!(asim, " -acdb -acdb_file {COVERAGE_ACDB} -acdb_cov sbceam").unwrap();
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
        asim.push_str(&resolve_rloc_tags(runfiles, raw));
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

/// Run vsimsa with its CWD in `work_dir` (where the coverage files land)
/// and echo its transcript into the test log.
fn run_vsimsa(
    vsimsa: &Path,
    do_path: &Path,
    home: &Path,
    work_dir: &Path,
) -> std::io::Result<std::process::Output> {
    let output = Command::new(vsimsa)
        .arg("-do")
        .arg(do_path)
        .env("HOME", home)
        .current_dir(work_dir)
        .output()?;
    let mut stdout = std::io::stdout().lock();
    write_transcript(&mut stdout, &output)?;
    stdout.flush()?;
    Ok(output)
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

/// Copy coverage.txt + coverage_html/ into $TEST_UNDECLARED_OUTPUTS_DIR,
/// then convert the text report to lcov for $COVERAGE_OUTPUT_FILE.
/// A missing text report (nothing coverable) is not an error.
fn post_process_coverage(work_dir: &Path) -> std::io::Result<()> {
    let text = match fs::read_to_string(work_dir.join(COVERAGE_TEXT)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };

    if let Ok(undeclared) = env::var("TEST_UNDECLARED_OUTPUTS_DIR") {
        let und = PathBuf::from(&undeclared);
        fs::create_dir_all(&und)?;
        let _ = fs::write(und.join(COVERAGE_TEXT), &text);
        let html_dir = work_dir.join(COVERAGE_HTML_DIR);
        if html_dir.exists() {
            let dst = und.join(COVERAGE_HTML_DIR);
            let _ = fs::remove_dir_all(&dst);
            let _ = copy_dir_all(&html_dir, &dst);
        }
    }

    if let Ok(coverage_out) = env::var("COVERAGE_OUTPUT_FILE") {
        fs::write(coverage_out, emit_lcov(&parse_acdb_text(&text)))?;
    }
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

/// Env var naming the file a pre_processor may write `K=V` lines to;
/// the launcher loads them into vsimsa's environment afterwards.
const PRE_PROCESSOR_ENV_FILE_VAR: &str = "RIVIERA_PRE_PROCESSOR_ENV_FILE";

/// Run the pre_processor in `work_dir` with the launcher's environment.
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

/// Load the pre_processor's `K=V` fragment (if any) into this process's
/// environment. Blank and `#` lines are skipped; a line without `=` is
/// an error.
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

/// Resolve `@rloc{...}` tags in environment variables in place. Runs
/// before the pre_processor so it and vsimsa both see real paths.
fn resolve_env_rlocations(runfiles: &Runfiles) {
    let tagged: Vec<(String, String)> = env::vars()
        .filter(|(_, v)| v.contains(RLOC_TAG_OPEN))
        .collect();
    for (key, value) in tagged {
        env::set_var(key, resolve_rloc_tags(runfiles, &value));
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

    // After the pre_processor, which may have set $RIVIERA_SEED. Printed
    // so a flaky run's seed can be copied back into --test_env.
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
    let output = run_vsimsa(&vsimsa, &do_path, &home, &tmpdir).expect("spawn vsimsa");
    if let Err(failure) = vsimsa_verdict(&output) {
        eprintln!("{failure}");
        return ExitCode::from(failure.exit_code());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_resolve(key: &str) -> String {
        format!("/runfiles/{key}")
    }

    #[test]
    fn literal_values_pass_through() {
        assert_eq!(
            substitute_rloc_tags("-f plain.f", fake_resolve),
            "-f plain.f"
        );
    }

    #[test]
    fn single_tag_is_spliced_in_place() {
        assert_eq!(
            substitute_rloc_tags("-f @rloc{_main/tests/pkg/flist}", fake_resolve),
            "-f /runfiles/_main/tests/pkg/flist"
        );
        assert_eq!(
            substitute_rloc_tags("+define+HEX=@rloc{_main/rom.hex}", fake_resolve),
            "+define+HEX=/runfiles/_main/rom.hex"
        );
    }

    #[test]
    fn multiple_keys_in_one_tag_resolve_to_a_list() {
        assert_eq!(
            substitute_rloc_tags("@rloc{_main/a.hex _main/b.hex}", fake_resolve),
            "/runfiles/_main/a.hex /runfiles/_main/b.hex"
        );
    }

    #[test]
    fn mixed_tags_and_text_keep_order() {
        assert_eq!(
            substitute_rloc_tags(
                "-pli @rloc{_main/vpi.so} -f @rloc{_main/x.f _main/y.f} -quiet",
                fake_resolve
            ),
            "-pli /runfiles/_main/vpi.so -f /runfiles/_main/x.f /runfiles/_main/y.f -quiet"
        );
    }

    #[test]
    #[should_panic(expected = "unterminated rlocation tag")]
    fn unterminated_tag_panics() {
        substitute_rloc_tags("-f @rloc{_main/flist", fake_resolve);
    }
}
