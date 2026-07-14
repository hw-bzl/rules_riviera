# rules_riviera

Bazel rules for [Aldec Riviera-PRO](https://www.aldec.com/en/products/functional_verification/riviera-pro), a mixed-language (Verilog / SystemVerilog / VHDL) HDL simulator.

Full documentation — setup, per-rule reference, coverage flow — is hosted at:

**<https://hw-bzl.github.io/rules_riviera/>**

## Quickstart

```python
# MODULE.bazel
bazel_dep(name = "rules_riviera", version = "0.1.0")
register_toolchains("//tools/riviera:my_riviera_toolchain")
```

See [the docs](https://hw-bzl.github.io/rules_riviera/) for the toolchain declaration.
