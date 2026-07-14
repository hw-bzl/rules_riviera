"""Shared helpers for rules_riviera rule implementations."""

load("@bazel_skylib//lib:collections.bzl", "collections")
load("@rules_verilog//verilog:defs.bzl", "VerilogInfo")
load("@rules_vhdl//vhdl:defs.bzl", "VhdlInfo")
load(":resource_sets.bzl", "cpu_resource_set")

_VERILOG_EXTS = ("v", "sv")
_VERILOG_HEADER_EXTS = ("vh", "svh")
_VHDL_EXTS = ("vhd", "vhdl")

def collect_hdl_sources(deps, srcs):
    """Collect HDL sources for compilation into one Riviera library.

    Each target in either list is one of:
    - A `verilog_library` (or anything providing `VerilogInfo`): pulls
      its own `srcs`+`hdrs`, its transitive `.deps` srcs+hdrs, and any
      cross-language `.vhdl_deps` VHDL srcs.
    - A `vhdl_library` (or anything providing `VhdlInfo`): same idea
      the other way round.
    - A raw HDL source file (`.v`/`.sv`/`.vh`/`.svh`/`.vhd`/`.vhdl`), used
      directly.

    The whole reachable graph is compiled into one Riviera library —
    provider walking is transitive, not direct-only.

    The two lists are separate parameters rather than one concatenated
    list because the split is semantic, not cosmetic: everything reached
    through a provider is compiled before any raw file, since alog/acom
    need packages compiled before their users.

    Args:
        deps: list of Target; compiled first, transitively.
        srcs: list of Target; raw HDL files, compiled last.

    Returns:
        struct(
            verilog = list[File],
            vhdl = list[File],
            headers = list[File],
        )
    """

    # Depsets dedup — flatten once at the end.
    src_depsets = []
    hdr_depsets = []
    raw_files = []

    for t in deps + srcs:
        if VerilogInfo in t:
            info = t[VerilogInfo]

            # Deps first (postorder: leaves precede parents) so alog/acom
            # compiles packages/entities before their users. Own srcs
            # last — this target is the "root" of its subgraph.
            for dep in info.deps.to_list():
                src_depsets.append(dep.srcs)
                hdr_depsets.append(dep.hdrs)
            for dep in info.vhdl_deps.to_list():
                src_depsets.append(dep.srcs)
            src_depsets.append(info.srcs)
            hdr_depsets.append(info.hdrs)
        elif VhdlInfo in t:
            info = t[VhdlInfo]
            for dep in info.deps.to_list():
                src_depsets.append(dep.srcs)
            for dep in info.verilog_deps.to_list():
                src_depsets.append(dep.srcs)
                hdr_depsets.append(dep.hdrs)
            src_depsets.append(info.srcs)
        else:
            # Raw file(s). `allow_files` on the attr already filters to
            # known HDL extensions.
            raw_files.extend(t.files.to_list())

    # Raw files are appended rather than passed as the depset's `direct`
    # list: depset iteration order for `default` is deliberately
    # unspecified, so only concatenation keeps them strictly last.
    all_files = depset(transitive = src_depsets + hdr_depsets).to_list()
    if raw_files:
        # Drops any file a dep already contributed, keeping first-seen
        # order. Skipped entirely when `srcs` is empty — the common case —
        # to avoid hashing the whole transitive graph for nothing.
        all_files = collections.uniq(all_files + raw_files)

    verilog = []
    vhdl = []
    headers = []
    for f in all_files:
        ext = f.extension.lower()
        if ext in _VERILOG_EXTS:
            verilog.append(f)
        elif ext in _VHDL_EXTS:
            vhdl.append(f)
        elif ext in _VERILOG_HEADER_EXTS:
            headers.append(f)
        else:
            fail("Unsupported source extension for {}: .{}".format(f.short_path, ext))

    return struct(verilog = verilog, vhdl = vhdl, headers = headers)

def determine_language(verilog, vhdl):
    """Classify a source set by which language lists are populated.

    Args:
        verilog: list of Verilog/SystemVerilog source Files.
        vhdl: list of VHDL source Files.

    Returns:
        One of "verilog", "vhdl", "mixed".
    """
    if verilog and vhdl:
        return "mixed"
    if vhdl:
        return "vhdl"
    return "verilog"

def include_dirs(headers):
    """Deduplicated list of directories containing the given header Files.

    Args:
        headers: list of header Files (`.vh`/`.svh`).

    Returns:
        list of distinct directory paths, in first-seen order.
    """
    return collections.uniq([h.dirname for h in headers])

def riviera_execution_requirements(tc_info, extra = None, jobs = 0):
    """Compose execution_requirements for actions/tests that invoke Riviera tools.

    Precedence (lowest -> highest): defaults, then `jobs`, then `extra`.
    Defaults are:

      - `resources:riviera_license=1` — participates in Bazel's local
        resource pool so concurrent Riviera actions stay under the
        configured license count. Users tune with
        `--local_resources=riviera_license=<N>`.
      - `requires-network` — when the toolchain says the licence server
        is remote (FlexLM over the network).
      - `cpu:N` — when `jobs > 0`, requests N CPU slots. Rules pass the
        already-resolved value (see `resolve_jobs`).

    Anything in `extra` overrides these — pass
    `{"resources:riviera_license": "2"}` to bump a specific action's
    license slot count, or `{"no-remote-exec": ""}` to force local.

    NOTE: `requires-riviera` is a *tag* (target `tags` attr for
    `--build_tag_filters=-requires-riviera`), NOT an execution
    requirement. Users tag their own targets — this rule set doesn't
    inject the tag for them.

    Args:
        tc_info: RivieraToolchainInfo from the resolved toolchain.
        extra: optional dict[str, str] merged on top of the defaults.
        jobs: int; resolved CPU-count request. `> 0` adds `cpu:N`;
            `<= 0` adds nothing.

    Returns:
        dict[str, str] suitable for passing as `execution_requirements`
        on an action or as `testing.ExecutionInfo(requirements=...)`.
    """
    reqs = {"resources:riviera_license": "1"}
    if tc_info.requires_network:
        reqs["requires-network"] = ""
    if jobs > 0:
        reqs["cpu:{}".format(jobs)] = ""
    if extra:
        for k, v in extra.items():
            reqs[k] = v
    return reqs

def resolve_jobs(rule_jobs, toolchain_jobs):
    """Two-level `jobs` resolution.

    Rule-level `-1` (the default) means "inherit from the toolchain."
    Any other value is used directly — including `0` (explicit "no
    hint"). Toolchain-level `-1` means "no hint" at the toolchain
    level too. The final resolved value flows into
    `riviera_resource_set` and `cpu:N` execution requirements.

    Args:
        rule_jobs: int; the rule's `jobs` attr (default -1).
        toolchain_jobs: int; the resolved toolchain's `jobs` field.

    Returns:
        int; the effective CPU-count request. -1 or 0 both mean
        "no hint"; positive values propagate through.
    """
    if rule_jobs == -1:
        return toolchain_jobs
    return rule_jobs

def riviera_resource_set(jobs):
    """Return a `resource_set` callable for `ctx.actions.run(...)`.

    Delegates to the static `cpu_resource_set` table in `resource_sets.bzl`
    so we hand Bazel identity-comparable callables (a closure per action
    breaks action-graph determinism — see rules_rust's
    `rustc_resource_set.bzl` for the same pattern).

    Args:
        jobs: int; requested CPU slots (after `resolve_jobs`). Values
            <= 0 mean "let Bazel decide" and return None; values above
            the max clamp to the max.

    Returns:
        A pre-defined `def(os_name, num_inputs) -> {'cpu': N}` callable,
        or None.
    """
    return cpu_resource_set(jobs)

def riviera_action_env(ctx, tc_info, extra = None):
    """Compose an env dict for a Riviera action.

    Precedence (lowest -> highest): `ctx.configuration.default_shell_env`
    (the user's `--action_env` values), then `tc_info.env`, then `extra`.

    Seeding from `default_shell_env` is what makes `--action_env=FOO=bar`
    actually reach the action under strict spawn strategies — sandboxes
    only forward vars explicitly listed in the action's `env` dict and
    drop everything else.

    Args:
        ctx: rule context; used for `configuration.default_shell_env`.
        tc_info: RivieraToolchainInfo from the resolved toolchain.
        extra: optional dict[str, str] merged on top of the toolchain env.

    Returns:
        dict[str, str] of environment variables.
    """
    env = dict(ctx.configuration.default_shell_env)
    for k, v in tc_info.env.items():
        env[k] = v
    if extra:
        for k, v in extra.items():
            env[k] = v
    return env

def rlocationpath(file, workspace_name):
    """Return the runfiles-library rlocation key for a File.

    The key is what a Bazel runfiles library (`@rules_rust//tools/runfiles`,
    rules_python's `python.runfiles`, ...) resolves to an absolute
    path at test time. For files in the main repo this is
    `<workspace_name>/<short_path>`; for files
    in an external repo `short_path` already starts with `../<repo>/`,
    so we strip the leading `../`.

    Args:
        file: a File.
        workspace_name: string; typically `ctx.workspace_name` from the
            caller.

    Returns:
        string; the rlocation key suitable for passing to
        `runfiles.Rlocation(...)` in the launcher.
    """
    if file.short_path.startswith("../"):
        return file.short_path[len("../"):]
    return "{}/{}".format(workspace_name, file.short_path)
