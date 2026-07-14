//! Helpers shared between the RivieraCompile action wrapper and the
//! `riviera_sim_test` launcher.
//!
//! Both drivers spawn `vsimsa`, judge the run by exit code plus the
//! error / fatal transcript lines Riviera doesn't reliably turn into an
//! exit code, read Aldec `library.cfg` files, and need a value distinct
//! per concurrent invocation.

use std::fmt::{self, Write as _};
use std::fs;
use std::io;
use std::path::Path;
use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};

/// Aldec's library-home config file name.
pub const LIBRARY_CFG: &str = "library.cfg";

/// A value distinct across concurrently running processes on one machine.
///
/// Wall-clock nanos alone collide when two processes start within the
/// same clock tick, so mix in the pid, which is unique among live
/// processes. Non-cryptographic — callers want distinctness, not
/// unpredictability.
pub fn process_nonce() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    nanos ^ (std::process::id() as u64).rotate_left(21)
}

/// Write both of vsimsa's streams, stdout first, to `w`.
pub fn write_transcript(w: &mut dyn io::Write, output: &Output) -> io::Result<()> {
    w.write_all(&output.stdout)?;
    w.write_all(&output.stderr)
}

/// Why a vsimsa run counts as failed.
#[derive(Debug, PartialEq, Eq)]
pub enum VsimsaFailure {
    /// Non-zero exit (or killed by a signal, reported as 1).
    Exit(i32),
    /// Exit was 0 but the transcript carries an error / fatal line.
    ErrorMarker,
}

impl VsimsaFailure {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Exit(rc) => *rc as u8,
            Self::ErrorMarker => 1,
        }
    }
}

impl fmt::Display for VsimsaFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exit(rc) => write!(f, "vsimsa exited with status {rc}"),
            Self::ErrorMarker => f.write_str("vsimsa log contains # Error / # Fatal markers"),
        }
    }
}

/// Judge a finished vsimsa run by exit status, then by transcript scan.
pub fn vsimsa_verdict(output: &Output) -> Result<(), VsimsaFailure> {
    match output.status.code().unwrap_or(1) {
        0 => {}
        rc => return Err(VsimsaFailure::Exit(rc)),
    }
    if has_error_marker(&output.stdout) || has_error_marker(&output.stderr) {
        return Err(VsimsaFailure::ErrorMarker);
    }
    Ok(())
}

/// Contents of the `library.cfg` that makes a directory a Riviera
/// library home for the single library `name`, stored in `./<name>.lib`.
pub fn library_cfg_contents(name: &str) -> String {
    format!("$INCLUDE = \"$VSIMSALIBRARYCFG\"\n{name} = \"./{name}.lib\"\n")
}

/// Append one `vmap <name> <abs_path>` line to `out` for every library
/// the home at `dir` declares, following `$INCLUDE` chains, and return
/// the names. Relative cfg paths resolve against the cfg's own directory
/// (Aldec's `vmap -link` would resolve them against the CWD instead).
pub fn vmap_lines_from_library_dir(out: &mut String, dir: &Path) -> io::Result<Vec<String>> {
    vmap_lines_from_cfg(out, &dir.join(LIBRARY_CFG), dir)
}

/// `.cfg` grammar handled:
///   `$INCLUDE = "<path>"` — recurse; a cfg file or a dir holding one.
///   `<name> = "<path>" [<timestamp>]` — emit `vmap <name> <abs_path>`.
///   `#` / blank lines — skipped.
/// `$INCLUDE = "$VSIMSALIBRARYCFG"` (Aldec's system cfg) is skipped.
fn vmap_lines_from_cfg(
    out: &mut String,
    cfg_path: &Path,
    resolve_dir: &Path,
) -> io::Result<Vec<String>> {
    let contents = fs::read_to_string(cfg_path).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("failed to read {}: {e}", cfg_path.display()),
        )
    })?;
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
        let Some((quoted, _tail)) = rest.strip_prefix('"').and_then(|r| r.split_once('"')) else {
            continue;
        };
        if key == "$INCLUDE" {
            if quoted.starts_with('$') {
                continue;
            }
            let nested = resolve_dir.join(quoted);
            if nested.is_dir() {
                names.extend(vmap_lines_from_library_dir(out, &nested)?);
            } else {
                let parent = nested.parent().unwrap_or(resolve_dir).to_path_buf();
                names.extend(vmap_lines_from_cfg(out, &nested, &parent)?);
            }
        } else {
            let abs = resolve_dir.join(quoted);
            writeln!(out, "vmap {} {}", key, abs.display()).unwrap();
            names.push(key.to_string());
        }
    }
    Ok(names)
}

/// Severity words of a failing diagnostic. `failure` and `error` cover
/// VHDL assertions (`assert ... severity failure` prints
/// `# EXECUTION:: FAILURE: <msg>` and leaves the exit code at 0, like
/// `$fatal`); `warning` and `note` are not failures.
const ERROR_SEVERITIES: [&[u8]; 4] = [b"fatal error", b"fatal", b"error", b"failure"];

/// True when any line in `log` is a Riviera error / fatal diagnostic.
///
/// A diagnostic is `# [<TOOL>:[:]] <Severity>: <message>`:
///
/// ```text
/// # ALOG: Error: VCP2000 /path/foo.sv : (2, 9): Syntax error. ...
/// # KERNEL: Fatal Error: /path/fatal_tb.sv (6): deliberate failure
/// # Error: ELBREAD_0081 unit not found
/// # EXECUTION:: FAILURE: deliberate failure
/// ```
///
/// So: the severity is one of the first two non-empty colon-delimited
/// fields. Only fields *followed by* a colon count, which is what keeps
/// a testbench's `$display("Error count = 0")` (`# KERNEL: Error count
/// = 0`) from matching. Lines without the `# ` transcript prefix are
/// vsimsa echoing the `.do` script and are never scanned.
pub fn has_error_marker(log: &[u8]) -> bool {
    log.split(|&b| b == b'\n').any(line_has_error_marker)
}

fn line_has_error_marker(raw: &[u8]) -> bool {
    let Some(body) = raw.trim_ascii_start().strip_prefix(b"#") else {
        return false;
    };
    let colons = body.iter().filter(|&&b| b == b':').count();
    body.split(|&b| b == b':')
        .take(colons)
        .map(<[u8]>::trim_ascii)
        .filter(|field| !field.is_empty())
        .take(2)
        .any(|field| {
            ERROR_SEVERITIES
                .iter()
                .any(|sev| field.eq_ignore_ascii_case(sev))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::process::ExitStatus;

    /// An `ExitStatus` whose `.code()` is `code`. The raw encoding is
    /// platform-specific: a unix wait status carries the code in its
    /// high byte, Windows stores it directly.
    fn exit_status(code: i32) -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(code << 8)
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(code as u32)
        }
    }

    fn output(code: i32, stdout: &str, stderr: &str) -> Output {
        Output {
            status: exit_status(code),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn transcript_is_stdout_then_stderr() {
        let mut buf = Vec::new();
        write_transcript(&mut buf, &output(0, "# transcript\n", "# warning\n")).unwrap();
        assert_eq!(buf, b"# transcript\n# warning\n");
    }

    #[test]
    fn verdict_prefers_exit_status_then_markers() {
        assert_eq!(vsimsa_verdict(&output(0, "# VSIM: done\n", "")), Ok(()));
        assert_eq!(
            vsimsa_verdict(&output(3, "# VSIM: Error: x\n", "")),
            Err(VsimsaFailure::Exit(3))
        );
        assert_eq!(
            vsimsa_verdict(&output(0, "", "# KERNEL: Fatal Error: x\n")),
            Err(VsimsaFailure::ErrorMarker)
        );
        assert_eq!(VsimsaFailure::Exit(3).exit_code(), 3);
        assert_eq!(VsimsaFailure::ErrorMarker.exit_code(), 1);
    }

    // Every positive/negative sample below is verbatim from a real
    // Riviera-PRO 2025.04 run through these rules.

    #[test]
    fn detects_compile_errors() {
        assert!(has_error_marker(
            b"# ALOG: Error: VCP2000 /execroot/_main/foo.sv : (2, 9): Syntax error.\n"
        ));
        assert!(has_error_marker(
            b"# COMP96: Error: COMP96_0015: foo.vhd : (3, 1): Design unit declaration expected.\n"
        ));
    }

    #[test]
    fn detects_elaboration_errors() {
        assert!(has_error_marker(
            b"# VSIM: Error: Unknown library unit \"no_such_module\" specified.\n"
        ));
        assert!(has_error_marker(
            b"# VSIM: Error: Simulation initialization failed.\n"
        ));
    }

    #[test]
    fn detects_runtime_fatal() {
        // `$fatal` in a testbench: vsimsa still exits 0 and the `.do`
        // still reaches `quit -code 0`, so this line is the ONLY signal
        // the test failed.
        assert!(has_error_marker(
            b"# KERNEL: Fatal Error: /execroot/_main/fatal_tb.sv (6): deliberate failure\n"
        ));
    }

    #[test]
    fn detects_vhdl_assertion_failures() {
        // `assert ... severity failure` / `severity error`: vsimsa exits
        // 0 and the `.do` reaches `quit -code 0`, like `$fatal`.
        assert!(has_error_marker(
            b"# EXECUTION:: FAILURE: deliberate failure\n"
        ));
        assert!(has_error_marker(
            b"# EXECUTION:: ERROR  : deliberate error\n"
        ));
        // `report` / `severity note` and `severity warning` are not failures.
        assert!(!has_error_marker(b"# EXECUTION:: NOTE   : PASS: link_tb\n"));
        assert!(!has_error_marker(b"# EXECUTION:: WARNING: something odd\n"));
    }

    #[test]
    fn detects_untagged_severity() {
        assert!(has_error_marker(b"# Error: ELBREAD_0081 unit not found\n"));
        assert!(has_error_marker(b"# Fatal: assertion failed\n"));
        // All-caps severity is itself tool-tag-shaped; must still match.
        assert!(has_error_marker(b"# ERROR: something broke\n"));
    }

    #[test]
    fn finds_a_marker_on_any_line() {
        let log = b"# Loading design\n# VSIM: Error: something broke\n# done\n";
        assert!(has_error_marker(log));
    }

    #[test]
    fn tolerates_crlf() {
        assert!(has_error_marker(b"# VSIM: Error: broke\r\n"));
    }

    #[test]
    fn testbench_prints_are_not_diagnostics() {
        // The regression this matcher exists for: a passing testbench
        // that prints the word "Error" must not fail the test.
        assert!(!has_error_marker(b"# KERNEL: Error count = 0\n"));
        assert!(!has_error_marker(b"# KERNEL: PASS: 0 errors, 0 warnings\n"));
        assert!(!has_error_marker(
            b"# ALOG: Compile failure 1 Errors 0 Warnings  Analysis time: 0[s].\n"
        ));
    }

    #[test]
    fn non_error_diagnostics_are_ignored() {
        let log = b"# Loading design\n\
                    # RUNTIME: Info: RUNTIME_0068 fatal_tb.sv (6): $finish called.\n\
                    # SCRIPTER: /execroot/_main/run.do : (5, 1): Executing onerror command.\n\
                    # KERNEL: Warning: unused signal\n\
                    # VSIM: Simulation has finished.\n";
        assert!(!has_error_marker(log));
    }

    #[test]
    fn marker_must_start_the_line() {
        // Riviera prefixes its own transcript lines with `# `. Text that
        // merely mentions an error mid-line (a testbench's own print, a
        // quoted message) must not trip the scan.
        let log = b"# note: the string \"# Error:\" appears in this message\n";
        assert!(!has_error_marker(log));
        // vsimsa echoes `.do` commands with no `#` prefix; a path or
        // top-level name embedding the word must not trip it either.
        let log = b"asim -c -L error_lib error_lib.error_tb\n";
        assert!(!has_error_marker(log));
    }

    #[test]
    fn empty_log_has_no_marker() {
        assert!(!has_error_marker(b""));
    }

    /// Scratch dir unique to one test, under the system temp dir.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("riviera_common_{name}_{:x}", process_nonce()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cfg_mappings_become_absolute_vmap_lines() {
        let dir = scratch("cfg");
        let mut cfg = library_cfg_contents("foo");
        cfg.push_str("# comment\n\nbar = \"./bar.lib\" 1700000000\n");
        fs::write(dir.join(LIBRARY_CFG), cfg).unwrap();

        let mut out = String::new();
        let names = vmap_lines_from_library_dir(&mut out, &dir).unwrap();

        assert_eq!(names, ["foo", "bar"]);
        assert_eq!(
            out,
            format!(
                "vmap foo {}\nvmap bar {}\n",
                dir.join("./foo.lib").display(),
                dir.join("./bar.lib").display()
            )
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cfg_include_recurses_into_files_and_dirs() {
        let dir = scratch("include");
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join(LIBRARY_CFG), library_cfg_contents("nested")).unwrap();
        fs::write(dir.join("extra.cfg"), "extra = \"./extra.lib\"\n").unwrap();
        fs::write(
            dir.join(LIBRARY_CFG),
            "$INCLUDE = \"sub\"\n$INCLUDE = \"extra.cfg\"\ntop = \"./top.lib\"\n",
        )
        .unwrap();

        let mut out = String::new();
        let names = vmap_lines_from_library_dir(&mut out, &dir).unwrap();

        assert_eq!(names, ["nested", "extra", "top"]);
        for (name, home) in [("nested", &sub), ("extra", &dir), ("top", &dir)] {
            let lib = home.join(format!("./{name}.lib"));
            let line = format!("vmap {name} {}\n", lib.display());
            assert!(out.contains(&line), "{out}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_cfg_is_an_error_naming_the_file() {
        let dir = scratch("missing");
        let mut out = String::new();
        let err = vmap_lines_from_library_dir(&mut out, &dir).unwrap_err();
        assert!(err.to_string().contains("library.cfg"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }
}
