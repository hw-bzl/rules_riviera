//! Helpers shared between the RivieraCompile action wrapper and the
//! `riviera_sim_test` launcher.
//!
//! Both drivers spawn `vsimsa` and inspect its output for two things
//! Riviera doesn't reliably signal via exit code: error / fatal
//! transcript lines, and stderr contents intermixed with stdout. Both
//! also need a value that is distinct per concurrent invocation.

use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Concatenate `output.stdout + output.stderr` into a single buffer,
/// mirroring how vsimsa's users typically consume the log.
pub fn merge_output(output: &Output) -> Vec<u8> {
    let mut log = output.stdout.clone();
    log.extend_from_slice(&output.stderr);
    log
}

/// Severity keywords that mark a transcript line as a failure, longest
/// first. Matched case-insensitively, and the trailing `:` is
/// load-bearing: it is the only thing separating a real diagnostic
/// (`Error: VCP2000 ... Syntax error.`) from a testbench that happens
/// to print the word (`$display("Error count = 0")`, which reaches the
/// transcript as `# KERNEL: Error count = 0`).
const ERROR_SEVERITIES: [&[u8]; 3] = [b"fatal error:", b"fatal:", b"error:"];

/// True when any line in `log` is a Riviera error / fatal diagnostic.
/// Riviera can emit those without a non-zero exit — a testbench calling
/// `$fatal` still lets the `.do` reach `quit -code 0` — so the drivers
/// scan the log defensively.
///
/// Diagnostics look like `# [<TOOL>: ]<Severity>: <message>`:
///
/// ```text
/// # ALOG: Error: VCP2000 /path/foo.sv : (2, 9): Syntax error. ...
/// # VSIM: Error: Unknown library unit "no_such_module" specified.
/// # KERNEL: Fatal Error: /path/fatal_tb.sv (6): deliberate failure
/// # Error: ELBREAD_0081 unit not found
/// ```
///
/// The leading `# ` is required — vsimsa echoes the `.do` script's own
/// commands unprefixed, and those routinely embed paths and messages
/// we must not scan. `<TOOL>` is optional and matched narrowly (see
/// [`strip_tool_tag`]).
pub fn has_error_marker(log: &[u8]) -> bool {
    log.split(|&b| b == b'\n').any(line_has_error_marker)
}

fn line_has_error_marker(raw: &[u8]) -> bool {
    let Some(body) = strip_transcript_prefix(raw) else {
        return false;
    };
    // Check before AND after stripping the tool tag: an all-caps
    // severity (`# ERROR: ...`) is itself tag-shaped, so stripping
    // first would swallow it.
    if starts_with_severity(body) {
        return true;
    }
    strip_tool_tag(body).is_some_and(starts_with_severity)
}

/// Strip the `# ` Riviera prefixes every transcript line with, plus any
/// leading whitespace. Everything downstream is prefix-oriented, so a
/// trailing `\r` (Windows vsimsa) needs no trimming.
fn strip_transcript_prefix(raw: &[u8]) -> Option<&[u8]> {
    Some(
        raw.trim_ascii_start()
            .strip_prefix(b"#")?
            .trim_ascii_start(),
    )
}

/// Strip a leading `<TAG>: ` tool prefix (`VSIM`, `KERNEL`, `ALOG`,
/// `COMP96`, `ELBREAD`, `SCRIPTER`, ...) and return what follows.
///
/// The tag charset is deliberately narrow — upper-case ASCII, digits
/// and `_` — so a severity word, which also carries a colon, is never
/// mistaken for a tag. `None` when the line doesn't open with one.
fn strip_tool_tag(line: &[u8]) -> Option<&[u8]> {
    let colon = line.iter().position(|&b| b == b':')?;
    let tag = &line[..colon];
    if tag.is_empty() {
        return None;
    }
    if !tag
        .iter()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
    {
        return None;
    }
    Some(line[colon + 1..].trim_ascii_start())
}

fn starts_with_severity(line: &[u8]) -> bool {
    ERROR_SEVERITIES.iter().any(|sev| {
        line.get(..sev.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(sev))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    fn output(stdout: &str, stderr: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn merge_puts_stderr_after_stdout() {
        let merged = merge_output(&output("# transcript\n", "# warning\n"));
        assert_eq!(merged, b"# transcript\n# warning\n");
    }

    #[test]
    fn merge_handles_empty_streams() {
        assert_eq!(merge_output(&output("", "")), b"");
        assert_eq!(merge_output(&output("out", "")), b"out");
        assert_eq!(merge_output(&output("", "err")), b"err");
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
}
