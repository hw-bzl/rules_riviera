# acdb2lcov

A small Rust tool that converts Aldec's `acdb report -txt` output to lcov `.info` format. Same code powers `riviera_sim_test`'s coverage post-processing.

## Where it lives

- **Binary**: `//tools/acdb2lcov:acdb2lcov_cli` — a `rust_binary` CLI wrapper around the `acdb2lcov` library.
- **Library**: `//tools/acdb2lcov:acdb2lcov` — the parser + emitter, exposed as a `rust_library` for anyone who wants to embed it.
- **Tests**: `//tools/acdb2lcov:acdb2lcov_test` — a `rust_test` over the library, runnable without a Riviera install.

## CLI usage

```
bazel run @rules_riviera//tools/acdb2lcov:acdb2lcov_cli -- \
    --input coverage.txt --output coverage.info
```

Both flags take filesystem paths. `//tools/acdb2lcov:cli` is an alias for the same binary; `:acdb2lcov` is the library and is not runnable.

## Format notes

The launcher runs `acdb report -txt -i coverage.acdb -o coverage.txt` inside vsimsa after the sim ends, producing a human-readable table report. `acdb2lcov` walks that table:

- **Statement Coverage** tables carry `| <line> | <hits> | ... |` rows keyed by a `| Line | Hits | Source: /path/to/foo.sv |` header. Rows with an empty `hits` cell are treated as non-coverable and skipped so they don't count as missed lines.
- **Branch Coverage** tables carry `IF branch#<line>#<idx>#` parent rows followed by one arm-name row per branch arm; each arm becomes a `BRDA` entry.

The report states the same coverage twice — once under a `DESIGN HIERARCHY` banner (one block per elaborated instance) and again under `DESIGN UNITS` (one block per module / entity). Only the instance-based part is counted; it is the finer-grained of the two, since a module instantiated N times contributes N tables whose hits sum to the real execution count. Reports generated without it fall back to the design-unit part. Within the part that is counted, repeated tables for the same source are additive, and each branch arm stays a single `BRDA` entry.

Source paths in the report are absolute (`/execroot/<workspace>/bazel-out/<config>/bin/<pkg>/<pkg>/<workspace-relative>`); the parser normalizes them back to a workspace-relative form so Bazel's lcov merger matches them against the baseline report.

Coverage types beyond Statement and Branch (Covergroup, Expression, Condition, FSM, Toggle, Assertion, Path) are silently skipped — they're not representable in lcov and `acdb report -txt` doesn't include them by default.
