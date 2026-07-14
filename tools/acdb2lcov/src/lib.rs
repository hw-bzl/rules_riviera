//! Parse Riviera's `acdb report -txt` output and emit lcov.
//!
//! The text format is table-based; per-file coverage lives under
//! block headers (`INSTANCE - ...`, `MODULE - ...`) that contain
//! `STATEMENT COVERAGE` and/or `BRANCH COVERAGE` tables. Both tables
//! start with a header row naming the source file
//! (`| Line | Hits | Source: /path/to/foo.sv |` for statements,
//! `|  Source: /path/to/foo.sv   |` for branches).
//!
//! Statement rows: `| <line> | <hits> | ... |`. Empty hit cells mean
//! the line is non-coverable (whitespace, comment) and should be
//! skipped entirely — not `DA:<line>,0`.
//!
//! Branch rows: `| IF branch#<line>#<idx># | ... | <taken>/<total> |`
//! then sub-rows `|     <label> | <hits> |` for each individual arm.
//! We emit one BRDA per arm keyed on line + branch idx + arm ordinal.
//!
//! Report parts: the report covers the same data TWICE — once under a
//! `DESIGN HIERARCHY` banner (one block per elaborated instance) and
//! once under `DESIGN UNITS` (one block per module / entity). Parsing
//! both would double every `DA` count and duplicate every `BRDA`
//! entry, so [`parse_acdb_text`] keeps them apart and returns one.
//!
//! Path normalization: sources compiled through
//! `riviera_library` end up with paths like
//! `/execroot/_main/bazel-out/k8-fastbuild/bin/tests/tests/foo/bar.sv`
//! (execroot + runfiles-workspace-root + compile-time workspace-relative
//! path). The parser strips the execroot + first-bazel-out prefix so
//! Bazel's coverage merger sees workspace-relative paths.

use std::collections::BTreeMap;
use std::fmt::Write;

use lcov::Record;

/// Identity of a single branch arm, as lcov keys them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BranchKey {
    pub line: u32,
    pub block: u32,
    pub branch: u32,
}

#[derive(Debug, Default, Clone)]
pub struct FileCoverage {
    pub path: String,
    pub statements: BTreeMap<u32, u64>,
    /// Hit count per arm. A map rather than a list so a module
    /// instantiated more than once accumulates into one `BRDA` per arm
    /// instead of emitting the arm repeatedly.
    pub branches: BTreeMap<BranchKey, u64>,
}

#[derive(Debug, Default, Clone)]
pub struct CoverageReport {
    pub files: BTreeMap<String, FileCoverage>,
}

/// Parse a text report into per-file line + branch coverage.
///
/// The instance-based (`DESIGN HIERARCHY`) part wins when present: it
/// is the finer-grained of the two, since a module instantiated N times
/// contributes N tables whose hits sum to the real execution count.
/// Reports generated without it fall back to the design-unit part.
pub fn parse_acdb_text(text: &str) -> CoverageReport {
    let mut hierarchy = CoverageReport::default();
    let mut design_units = CoverageReport::default();
    let mut part = ReportPart::Hierarchy;
    let mut section = Section::None;
    let mut current_path: Option<String> = None;
    let mut current_branch_line: Option<u32> = None;
    let mut current_branch_idx: u32 = 0;
    let mut branch_arm_idx: u32 = 0;

    for raw in text.lines() {
        let line = raw.trim();

        // Part banners (`++++++++++    DESIGN UNITS    ++++++++++`) come
        // first: they also match the `+++` reset below.
        let banner = if line.contains("DESIGN HIERARCHY") {
            Some(ReportPart::Hierarchy)
        } else if line.contains("DESIGN UNITS") {
            Some(ReportPart::DesignUnits)
        } else {
            None
        };
        if let Some(next) = banner {
            part = next;
            section = Section::None;
            current_path = None;
            continue;
        }

        if line.contains("STATEMENT COVERAGE") {
            section = Section::Statement;
            current_path = None;
            continue;
        }
        if line.contains("BRANCH COVERAGE") {
            section = Section::Branch;
            current_path = None;
            current_branch_line = None;
            continue;
        }
        if is_block_header(line) || line.starts_with("SUMMARY") || line.starts_with("+++") {
            section = Section::None;
            continue;
        }

        // Header row for either table names the source file.
        if let Some(idx) = line.find("Source:") {
            let after = line[idx + "Source:".len()..].trim_end_matches('|').trim();
            current_path = Some(normalize_path(after));
            continue;
        }

        if section == Section::None || current_path.is_none() {
            continue;
        }
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        // Separator rows like |------|------|------|
        if cells
            .iter()
            .all(|c| c.chars().all(|ch| ch == '-' || ch == ' '))
        {
            continue;
        }

        // Skip the design-unit part outright once the hierarchy part has
        // produced anything — it restates the same coverage, and parsing
        // it would double the work and peak heap for a result the tail of
        // this function discards. Guarded rather than `break`ing at the
        // banner so a report that somehow ordered the parts the other way
        // still yields the hierarchy data.
        let report = match part {
            ReportPart::Hierarchy => &mut hierarchy,
            ReportPart::DesignUnits if hierarchy.files.is_empty() => &mut design_units,
            ReportPart::DesignUnits => continue,
        };
        let path = current_path.as_deref().unwrap();
        let file = report
            .files
            .entry(path.to_string())
            .or_insert_with(|| FileCoverage {
                path: path.to_string(),
                ..Default::default()
            });

        match section {
            Section::Statement => {
                // Expect at least [line, hits, source-preview...].
                if cells.len() < 2 {
                    continue;
                }
                let Ok(line_no) = cells[0].parse::<u32>() else {
                    continue;
                };
                let hits_cell = cells[1];
                if hits_cell.is_empty() {
                    // Non-coverable (comment / whitespace) — skip so lcov
                    // doesn't count it as a missed line.
                    continue;
                }
                let Ok(hits) = hits_cell.parse::<u64>() else {
                    continue;
                };
                // `+=`, not `=`: within one part, repeated tables are
                // distinct instances of the same module and their hits
                // are additive.
                *file.statements.entry(line_no).or_insert(0) += hits;
            }
            Section::Branch => {
                if cells.len() < 2 {
                    continue;
                }
                let head = cells[0];
                // Parent row: `IF branch#<line>#<idx>#`
                if let Some(rest) = head.strip_prefix("IF branch#") {
                    let mut parts = rest.split('#');
                    if let (Some(l), Some(i)) = (parts.next(), parts.next()) {
                        if let (Ok(ln), Ok(idx)) = (l.parse::<u32>(), i.parse::<u32>()) {
                            current_branch_line = Some(ln);
                            current_branch_idx = idx;
                            branch_arm_idx = 0;
                        }
                    }
                    continue;
                }
                // Arm sub-row: `<label>` + hits count. Only counted if
                // we're already inside an `IF branch#...` parent row.
                let Some(line_no) = current_branch_line else {
                    continue;
                };
                // Arm rows in this format have a label + integer hit count.
                let Some(hits_cell) = cells.get(1) else {
                    continue;
                };
                let Ok(hits) = hits_cell.parse::<u64>() else {
                    continue;
                };
                let key = BranchKey {
                    line: line_no,
                    block: current_branch_idx,
                    branch: branch_arm_idx,
                };
                *file.branches.entry(key).or_insert(0) += hits;
                branch_arm_idx += 1;
            }
            Section::None => {}
        }
    }

    if hierarchy.files.is_empty() {
        design_units
    } else {
        hierarchy
    }
}

/// True for the block headers that introduce a new coverage subject:
/// `INSTANCE - /tb : lib.tb` in the hierarchy part, `MODULE - lib.tb` /
/// `ENTITY - ...` / `ARCHITECTURE - ...` in the design-units part. Any
/// of them ends the table we were reading.
fn is_block_header(line: &str) -> bool {
    let Some((head, _)) = line.split_once(" - ") else {
        return false;
    };
    !head.is_empty() && head.chars().all(|c| c.is_ascii_uppercase() || c == ' ')
}

/// Bazel's coverage merger keys files by workspace-relative path.
/// Strip `/execroot/<workspace>/` and the runfiles-workspace-root
/// bazel-out prefix so the path we emit matches what Bazel expects.
fn normalize_path(p: &str) -> String {
    let p = p.trim();
    let mut s = p.to_string();
    if let Some(after) = s.strip_prefix("/execroot/") {
        // /execroot/<workspace>/rest → rest
        if let Some(slash) = after.find('/') {
            s = after[slash + 1..].to_string();
        }
    }
    // A common shape after the strip is:
    //   bazel-out/<config>/bin/<pkg-of-test>/<workspace-relative-src>
    // The compile-time workspace-relative path shows up doubled
    // (`<pkg-of-test>/<workspace-relative-src>`) because the runfiles
    // workspace root ends at `<pkg-of-test>` while the source ref
    // stored in the compiled library is the compile-time workspace
    // path. Trim to the last known-source root when present.
    if let Some(rest) = s.strip_prefix("bazel-out/") {
        // bazel-out/<config>/bin/... — strip 3 segments.
        let mut it = rest.splitn(3, '/');
        it.next();
        it.next();
        if let Some(tail) = it.next() {
            // `<pkg-of-test>/<workspace-relative-src>` → keep just the
            // workspace-relative tail. We do this by scanning for the
            // rightmost occurrence of the first path segment repeated.
            if let Some(dedup) = dedupe_leading_segment(tail) {
                s = dedup;
            } else {
                s = tail.to_string();
            }
        }
    }
    s
}

/// If `p` has the shape `<seg>/<seg>/rest`, return `<seg>/rest`.
/// Otherwise return None.
fn dedupe_leading_segment(p: &str) -> Option<String> {
    let mut it = p.splitn(3, '/');
    let a = it.next()?;
    let b = it.next()?;
    let rest = it.next()?;
    if a == b {
        Some(format!("{a}/{rest}"))
    } else {
        None
    }
}

pub fn emit_lcov(report: &CoverageReport) -> String {
    let mut out = String::new();
    for file in report.files.values() {
        let mut push = |r: Record| writeln!(out, "{r}").unwrap();

        push(Record::SourceFile {
            path: file.path.as_str().into(),
        });

        let mut lh: u32 = 0;
        for (&line, &hits) in &file.statements {
            push(Record::LineData {
                line,
                count: hits,
                checksum: None,
            });
            if hits > 0 {
                lh += 1;
            }
        }
        push(Record::LinesFound {
            found: file.statements.len() as u32,
        });
        push(Record::LinesHit { hit: lh });

        let mut brh: u32 = 0;
        for (key, &hits) in &file.branches {
            let taken = if hits == 0 {
                None
            } else {
                brh += 1;
                Some(hits)
            };
            push(Record::BranchData {
                line: key.line,
                block: key.block,
                branch: key.branch,
                taken,
            });
        }
        push(Record::BranchesFound {
            found: file.branches.len() as u32,
        });
        push(Record::BranchesHit { hit: brh });

        push(Record::EndOfRecord);
    }
    out
}

/// Which top-level pass over the design a block belongs to. The two
/// carry identical tables; see [`parse_acdb_text`].
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum ReportPart {
    Hierarchy,
    DesignUnits,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Section {
    None,
    Statement,
    Branch,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim structure of a real Riviera-PRO 2025.04
    /// `acdb report -txt` run, trimmed to two source files. Note the
    /// two banners: every table below `DESIGN HIERARCHY` repeats below
    /// `DESIGN UNITS`.
    const REAL_REPORT: &str = r#"
+++++++++++++++++++++++++++++++++++++++++++++
++++++++++     DESIGN HIERARCHY    ++++++++++
+++++++++++++++++++++++++++++++++++++++++++++


CUMULATIVE SUMMARY
============================================
|   Coverage Type    | Weight | Hits/Total |
============================================
| Statement Coverage |      1 |    73.333% |
============================================


INSTANCE - /adder_tb : adder_lib.adder_tb


    STATEMENT COVERAGE
    ==========================================================================================================
    | Line | Hits | Source: /execroot/_main/bazel-out/k8-fastbuild/bin/tests/tests/smoke_verilog/adder_tb.sv |
    |------|------|------------------------------------------------------------------------------------------|
    | 9    |      |                                                                                          |
    | 11   |  1   |                                                                                          |
    | 15   |  0   |                                                                                          |
    ==========================================================================================================


    BRANCH COVERAGE
    ===============================================================================================
    |  Source: /execroot/_main/bazel-out/k8-fastbuild/bin/tests/tests/smoke_verilog/adder_tb.sv   |
    ===============================================================================================
    | Branch/Line                                  |                     Hits                     |
    ===============================================================================================
    | IF branch#14#1#                              |                                          1/2 |
    |     if_branch                                |                                            0 |
    |     all_false_branch                         |                                            1 |
    ===============================================================================================


INSTANCE - /adder_tb/dut : adder_lib.adder


    STATEMENT COVERAGE
    =======================================================================================================
    | Line | Hits | Source: /execroot/_main/bazel-out/k8-fastbuild/bin/tests/tests/smoke_verilog/adder.sv |
    |------|------|---------------------------------------------------------------------------------------|
    | 8    |  1   |                                                                                       |
    =======================================================================================================


+++++++++++++++++++++++++++++++++++++++++++++
++++++++++       DESIGN UNITS      ++++++++++
+++++++++++++++++++++++++++++++++++++++++++++


MODULE - adder_lib.adder_tb


    STATEMENT COVERAGE
    ==========================================================================================================
    | Line | Hits | Source: /execroot/_main/bazel-out/k8-fastbuild/bin/tests/tests/smoke_verilog/adder_tb.sv |
    |------|------|------------------------------------------------------------------------------------------|
    | 9    |      |                                                                                          |
    | 11   |  1   |                                                                                          |
    | 15   |  0   |                                                                                          |
    ==========================================================================================================


    BRANCH COVERAGE
    ===============================================================================================
    |  Source: /execroot/_main/bazel-out/k8-fastbuild/bin/tests/tests/smoke_verilog/adder_tb.sv   |
    ===============================================================================================
    | Branch/Line                                  |                     Hits                     |
    ===============================================================================================
    | IF branch#14#1#                              |                                          1/2 |
    |     if_branch                                |                                            0 |
    |     all_false_branch                         |                                            1 |
    ===============================================================================================


MODULE - adder_lib.adder


    STATEMENT COVERAGE
    =======================================================================================================
    | Line | Hits | Source: /execroot/_main/bazel-out/k8-fastbuild/bin/tests/tests/smoke_verilog/adder.sv |
    |------|------|---------------------------------------------------------------------------------------|
    | 8    |  1   |                                                                                       |
    =======================================================================================================
"#;

    #[test]
    fn parses_real_adder_report() {
        let r = parse_acdb_text(REAL_REPORT);
        assert_eq!(r.files.len(), 2);

        let tb = r
            .files
            .get("tests/smoke_verilog/adder_tb.sv")
            .expect("adder_tb.sv");
        assert_eq!(tb.statements.len(), 2);
        assert_eq!(*tb.statements.get(&11).unwrap(), 1);
        assert_eq!(*tb.statements.get(&15).unwrap(), 0);
        assert!(!tb.statements.contains_key(&9)); // non-coverable line skipped
        assert_eq!(tb.branches.len(), 2);
        assert_eq!(
            tb.branches[&BranchKey {
                line: 14,
                block: 1,
                branch: 0
            }],
            0
        );
        assert_eq!(
            tb.branches[&BranchKey {
                line: 14,
                block: 1,
                branch: 1
            }],
            1
        );

        let dut = r
            .files
            .get("tests/smoke_verilog/adder.sv")
            .expect("adder.sv");
        assert_eq!(*dut.statements.get(&8).unwrap(), 1);
    }

    /// The `DESIGN UNITS` part restates every table from `DESIGN
    /// HIERARCHY`. Counting both doubled `DA`/`BRF`/`BRH` and emitted
    /// each `BRDA` twice.
    #[test]
    fn design_units_part_is_not_counted_twice() {
        let lcov = emit_lcov(&parse_acdb_text(REAL_REPORT));
        assert!(lcov.contains("DA:11,1"), "{lcov}");
        assert!(lcov.contains("DA:15,0"), "{lcov}");
        assert!(lcov.contains("DA:8,1"), "{lcov}");
        assert!(!lcov.contains("DA:11,2"), "{lcov}");
        assert!(!lcov.contains("DA:8,2"), "{lcov}");
        assert_eq!(lcov.matches("BRDA:14,1,0,-").count(), 1, "{lcov}");
        assert_eq!(lcov.matches("BRDA:14,1,1,1").count(), 1, "{lcov}");
        assert!(lcov.contains("BRF:2"), "{lcov}");
        assert!(lcov.contains("BRH:1"), "{lcov}");
        assert!(lcov.contains("LF:2"), "{lcov}");
        assert!(lcov.contains("LH:1"), "{lcov}");
        assert!(
            lcov.contains("SF:tests/smoke_verilog/adder_tb.sv"),
            "{lcov}"
        );
        assert!(!lcov.contains("DA:9,"), "{lcov}");
    }

    /// A report with only the design-unit pass (`acdb report -du`) must
    /// still yield coverage rather than an empty result.
    #[test]
    fn falls_back_to_design_units_when_hierarchy_absent() {
        let banner = REAL_REPORT.find("DESIGN UNITS").unwrap();
        let r = parse_acdb_text(&REAL_REPORT[banner..]);
        assert_eq!(r.files.len(), 2);
        assert_eq!(
            *r.files["tests/smoke_verilog/adder_tb.sv"]
                .statements
                .get(&11)
                .unwrap(),
            1
        );
    }

    /// Two instances of the same module: hits are additive and each arm
    /// stays a single BRDA entry.
    #[test]
    fn repeated_instances_accumulate_into_one_entry() {
        let text = r#"
++++++++++     DESIGN HIERARCHY    ++++++++++

INSTANCE - /tb/a : lib.dut

    STATEMENT COVERAGE
    | Line | Hits | Source: /work/dut.sv |
    | 8    |  3   |                      |

    BRANCH COVERAGE
    |  Source: /work/dut.sv |
    | IF branch#9#1#        |     1/2 |
    |     if_branch         |       2 |
    |     all_false_branch  |       0 |

INSTANCE - /tb/b : lib.dut

    STATEMENT COVERAGE
    | Line | Hits | Source: /work/dut.sv |
    | 8    |  4   |                      |

    BRANCH COVERAGE
    |  Source: /work/dut.sv |
    | IF branch#9#1#        |     1/2 |
    |     if_branch         |       5 |
    |     all_false_branch  |       0 |
"#;
        let r = parse_acdb_text(text);
        let f = &r.files["/work/dut.sv"];
        assert_eq!(*f.statements.get(&8).unwrap(), 7);
        assert_eq!(f.branches.len(), 2);
        let lcov = emit_lcov(&r);
        assert!(lcov.contains("BRDA:9,1,0,7"), "{lcov}");
        assert!(lcov.contains("BRDA:9,1,1,-"), "{lcov}");
        assert!(lcov.contains("BRF:2"), "{lcov}");
    }

    #[test]
    fn normalize_strips_execroot_and_bazel_out_dedupe() {
        assert_eq!(
            normalize_path("/execroot/_main/bazel-out/k8-fastbuild/bin/tests/tests/foo/bar.sv"),
            "tests/foo/bar.sv"
        );
        assert_eq!(
            normalize_path("bazel-out/k8-fastbuild/bin/pkg/pkg/x.sv"),
            "pkg/x.sv"
        );
        assert_eq!(normalize_path("plain/file.sv"), "plain/file.sv");
    }

    #[test]
    fn block_headers_end_the_current_table() {
        assert!(is_block_header("INSTANCE - /adder_tb : adder_lib.adder_tb"));
        assert!(is_block_header("MODULE - adder_lib.adder"));
        assert!(is_block_header("ENTITY - lib.counter"));
        assert!(is_block_header("ARCHITECTURE - lib.counter(rtl)"));
        assert!(!is_block_header("| 11   |  1   |   |"));
        assert!(!is_block_header(
            "CUMULATIVE INSTANCE-BASED COVERAGE: 61.666%"
        ));
    }
}
