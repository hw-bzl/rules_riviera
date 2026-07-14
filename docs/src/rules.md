# Rules

Three public rules over the providers in `//riviera:defs.bzl`.

| Rule | Produces | Consumes |
|---|---|---|
| `riviera_library` | `RivieraLibraryInfo` | `VerilogInfo` / `VhdlInfo` |
| `riviera_sim_test` | (a bazel test) | `RivieraLibraryInfo` |
| `riviera_toolchain` | `RivieraToolchainInfo` (one `RivieraToolInfo` per tool) | one executable per Riviera tool; only `vsimsa` is mandatory |

Anything that produces `RivieraLibraryInfo` (a pre-built vendor library, an imported UVM archive, etc.) is a drop-in for `riviera_sim_test.deps` — no sim-rule surgery required.

- [`riviera_library`](./riviera_library.md) — compile a `verilog_library` / `vhdl_library` DAG into one Riviera library.
- [`riviera_sim_test`](./riviera_sim_test.md) — `vmap` one or more `riviera_library` deps and elaborate + simulate a `top` under `vsimsa`. Automatic coverage under `bazel coverage`.
- [`riviera_toolchain`](./riviera_toolchain.md) — bind Riviera executables to the toolchain type consumed by the rules above.
