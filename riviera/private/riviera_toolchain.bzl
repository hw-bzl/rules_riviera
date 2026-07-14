"""The `riviera_toolchain` rule implementation."""

load(":providers.bzl", "RivieraToolInfo", "RivieraToolchainInfo")
load(":tools.bzl", "MANDATORY_TOOLS", "TOOL_NAMES")

def _tool_info(ctx, name):
    """Bundle one tool attr into a `RivieraToolInfo`.

    Both fields stage the executable itself, so no consumer has to pair
    one with a bare `File` — see the `RivieraToolInfo` doc.

    Args:
        ctx: rule context.
        name: string; the tool attr name.

    Returns:
        RivieraToolInfo.
    """
    target = getattr(ctx.attr, name)
    return RivieraToolInfo(
        files_to_run = target[DefaultInfo].files_to_run,
        runfiles = ctx.runfiles(files = [getattr(ctx.executable, name)]).merge(
            target[DefaultInfo].default_runfiles,
        ),
    )

def _riviera_toolchain_impl(ctx):
    tools = {name: _tool_info(ctx, name) for name in TOOL_NAMES}
    toolchain_info = RivieraToolchainInfo(
        env = ctx.attr.env,
        requires_network = ctx.attr.requires_network,
        version = ctx.attr.version,
        jobs = ctx.attr.jobs,
        **tools
    )
    return [
        platform_common.ToolchainInfo(
            riviera_info = toolchain_info,
            label = ctx.label,
        ),
    ]

_MANDATORY_DOC = "Executable invoked as `{}`. MANDATORY — the rules call this tool directly."

_OPTIONAL_DOC = "Executable invoked as `{}`. Optional — no rule in this repo invokes it across the Bazel action boundary. Bind it if your own rules need a Bazel-visible path, or to override the default PATH-resolving shim."

def _tool_attr(name, mandatory):
    """One tool attr: mandatory with no default, or optional with a shim.

    Note the absence of `allow_single_file`: it would restrict every
    binding to a single-output target, rejecting exactly the `sh_binary`
    / `py_binary` wrappers-with-`data` that `RivieraToolInfo` exists to
    carry the runfiles of.

    Args:
        name: string; the tool attr name.
        mandatory: bool; whether the user must bind it explicitly.

    Returns:
        attr.label.
    """
    return attr.label(
        executable = True,
        cfg = "exec",
        mandatory = mandatory,
        default = None if mandatory else Label("//riviera/private/shims:{}".format(name)),
        doc = (_MANDATORY_DOC if mandatory else _OPTIONAL_DOC).format(name),
    )

_TOOL_ATTR_DEFS = {name: _tool_attr(name, name in MANDATORY_TOOLS) for name in TOOL_NAMES}

riviera_toolchain = rule(
    implementation = _riviera_toolchain_impl,
    doc = """Declares an Aldec Riviera-PRO toolchain by binding one executable per Riviera tool.

Each tool attr takes an executable target that resolves to the
corresponding Riviera binary. In the ideal case that's a hermetic
toolchain that vendors the whole install into the Bazel action graph;
in practice it can be anything Bazel can `exec` — a `sh_binary`
wrapper that sources a license env file and execs a site install, a
container launcher, a PATH shim. The toolchain only cares that each
attr resolves to something that behaves like the named tool.

Tools bound to a wrapper keep their runfiles: each attr is exposed to
the rules as a `RivieraToolInfo` carrying the target's
`files_to_run` + `default_runfiles`, not just the bare executable
`File`.

**`vsimsa` is the only mandatory attr.** It is the one binary the rules
invoke across the Bazel action boundary — every `riviera_library`
compile action and every `riviera_sim_test` launcher call. Coverage
report generation (`acdb report -txt/-html`) runs as TCL inside that
same `vsimsa` invocation.

**Everything else is optional** and defaults to a shipped
`exec <tool> "$@"` PATH-resolving shim:

- `acdb`, `vsim`, `alib`, `alog`, `acom`, `asim`, `vmap` — run inside
  vsimsa's TCL interpreter today and never cross the Bazel boundary.
- `riviera` — the interactive Riviera-PRO IDE. No rule here launches
  it; the attr exists so a rule of your own can.

They're exposed so downstream consumers have a place to bind if they
ever need a Bazel-visible path to those binaries.""",
    attrs = dict(
        _TOOL_ATTR_DEFS,
        env = attr.string_dict(
            doc = "Extra env vars applied to every action that resolves this toolchain.",
            default = {},
        ),
        requires_network = attr.bool(
            doc = "Set True when Riviera actions need network access (FlexLM license server).",
            default = False,
        ),
        version = attr.string(
            doc = "Toolchain version identifier for diagnostics.",
            default = "",
        ),
        jobs = attr.int(
            doc = "Default CPU-count hint for actions and tests that resolve this toolchain. `-1` (default) means no hint — Bazel schedules however it likes; `0` also means no hint; `N > 0` requests N CPUs. Per-target `jobs = -1` (the rule default) inherits this value.",
            default = -1,
        ),
    ),
)
