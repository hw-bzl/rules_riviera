"""Shared helpers for rules_riviera rule implementations."""

load("@bazel_skylib//lib:collections.bzl", "collections")
load("@rules_verilog//verilog:defs.bzl", "VerilogInfo")
load("@rules_vhdl//vhdl:defs.bzl", "VhdlInfo")

TOOLCHAIN_TYPE = str(Label("//riviera:toolchain_type"))

VERILOG_EXTENSIONS = ["v", "sv"]
VERILOG_HEADER_EXTENSIONS = ["vh", "svh"]
VHDL_EXTENSIONS = ["vhd", "vhdl"]

# Extensions `riviera_library` compiles and coverage instruments.
HDL_SOURCE_EXTENSIONS = VERILOG_EXTENSIONS + VHDL_EXTENSIONS

# Everything `riviera_library.srcs` accepts.
HDL_FILE_EXTENSIONS = HDL_SOURCE_EXTENSIONS + VERILOG_HEADER_EXTENSIONS

# Deepest Verilog <-> VHDL alternation `_hdl_closure` follows before
# failing loudly instead of silently dropping sources.
_MAX_LANGUAGE_HOPS = 64

def _closed(info):
    """Postorder depset of one provider plus its same-language closure."""
    return depset([info], order = "postorder", transitive = [info.deps])

def _hdl_closure(verilog_roots, vhdl_roots):
    """Full transitive closure of `VerilogInfo` / `VhdlInfo` providers.

    Upstream `.deps` is closed within one language and `.vhdl_deps` /
    `.verilog_deps` one hop across, but a provider reached across the
    boundary is not expanded into *its* cross-language deps, so an SV
    testbench over a VHDL wrapper over an SV core would lose the core.
    Iterating to a fixed point follows every alternation. (Starlark has
    no recursion and providers are not hashable, so a worklist with a
    visited set is not an option.)

    Every depset unioned here is transitively closed in postorder, so
    first-occurrence dedup keeps dependencies before dependents whatever
    the concatenation order. Newly reached providers go first so leaves
    still precede the roots.

    Args:
        verilog_roots: list of VerilogInfo.
        vhdl_roots: list of VhdlInfo.

    Returns:
        (list[VerilogInfo], list[VhdlInfo]) in compile order.
    """
    verilog = depset(order = "postorder", transitive = [_closed(i) for i in verilog_roots])
    vhdl = depset(order = "postorder", transitive = [_closed(i) for i in vhdl_roots])
    verilog_list = verilog.to_list()
    vhdl_list = vhdl.to_list()
    for _ in range(_MAX_LANGUAGE_HOPS):
        # Depset truthiness is an emptiness test without flattening, so
        # the single-language case exits here with no extra work.
        cross_verilog = [i.verilog_deps for i in vhdl_list if i.verilog_deps]
        cross_vhdl = [i.vhdl_deps for i in verilog_list if i.vhdl_deps]
        if not cross_verilog and not cross_vhdl:
            return verilog_list, vhdl_list
        next_verilog = depset(order = "postorder", transitive = cross_verilog + [verilog])
        next_vhdl = depset(order = "postorder", transitive = cross_vhdl + [vhdl])
        next_verilog_list = next_verilog.to_list()
        next_vhdl_list = next_vhdl.to_list()
        if len(next_verilog_list) == len(verilog_list) and len(next_vhdl_list) == len(vhdl_list):
            return verilog_list, vhdl_list
        verilog, vhdl = next_verilog, next_vhdl
        verilog_list, vhdl_list = next_verilog_list, next_vhdl_list
    fail("HDL dependency graph alternates languages more than {} times.".format(_MAX_LANGUAGE_HOPS))

def collect_hdl_sources(deps, srcs):
    """Collect HDL sources for compilation into one Riviera library.

    Each target is a `VerilogInfo` / `VhdlInfo` provider (walked
    transitively across any number of language boundaries, pulling
    `srcs`, `hdrs`, `data` and `includes`) or a raw HDL file. Everything
    reached through a provider precedes every raw file.

    Args:
        deps: list of Target; provider targets, compiled first.
        srcs: list of Target; raw HDL files, compiled last.

    Returns:
        struct(
            verilog = list[File], in compile order,
            vhdl = list[File], in compile order,
            headers = list[File],
            data = depset[File]; `data` from anywhere in the graph,
            includes = list[str]; `includes` from anywhere in the graph,
        )
    """
    verilog_roots = []
    vhdl_roots = []
    raw_files = []

    for t in deps + srcs:
        if VerilogInfo in t:
            verilog_roots.append(t[VerilogInfo])
        elif VhdlInfo in t:
            vhdl_roots.append(t[VhdlInfo])
        else:
            raw_files.extend(t.files.to_list())

    verilog_infos, vhdl_infos = _hdl_closure(verilog_roots, vhdl_roots)
    infos = verilog_infos + vhdl_infos

    # Each provider's `srcs` is a flat depset, so unioning them in
    # provider order yields files in compile order. Raw files are
    # appended afterwards rather than passed as `direct`, which would
    # not keep them last; `uniq` drops any a provider already supplied.
    all_files = depset(
        transitive = [i.srcs for i in infos] + [i.hdrs for i in verilog_infos],
    ).to_list()
    if raw_files:
        all_files = collections.uniq(all_files + raw_files)

    verilog = []
    vhdl = []
    headers = []
    for f in all_files:
        ext = f.extension.lower()
        if ext in VERILOG_EXTENSIONS:
            verilog.append(f)
        elif ext in VHDL_EXTENSIONS:
            vhdl.append(f)
        elif ext in VERILOG_HEADER_EXTENSIONS:
            headers.append(f)
        else:
            fail("Unsupported source extension for {}: .{}".format(f.short_path, ext))

    return struct(
        verilog = verilog,
        vhdl = vhdl,
        headers = headers,
        data = depset(transitive = [i.data for i in infos]),
        includes = depset(transitive = [i.includes for i in verilog_infos]).to_list(),
    )

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

def include_dirs(headers, includes):
    """Deduplicated `+incdir+` paths: header dirnames, then explicit includes.

    Args:
        headers: list of header Files (`.vh`/`.svh`).
        includes: list of str; `VerilogInfo.includes` entries.

    Returns:
        list of distinct directory paths, in first-seen order.
    """
    return collections.uniq([h.dirname for h in headers] + includes)

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
    reqs.update(extra or {})
    return reqs

def resolve_jobs(rule_jobs, toolchain_jobs):
    """Rule-level `jobs = -1` inherits the toolchain's value; anything else wins.

    Args:
        rule_jobs: int; the rule's `jobs` attr.
        toolchain_jobs: int; the toolchain's `jobs` field.

    Returns:
        int; `> 0` requests that many CPUs, anything else is no hint.
    """
    if rule_jobs == -1:
        return toolchain_jobs
    return rule_jobs

def riviera_action_env(ctx, tc_info, extra = None):
    """Env for a Riviera action: `--action_env` values, then toolchain env, then `extra`.

    Seeding from `default_shell_env` is what lets `--action_env=FOO=bar`
    reach a sandboxed action, which only sees vars listed in its `env`.

    Args:
        ctx: rule context.
        tc_info: RivieraToolchainInfo from the resolved toolchain.
        extra: optional dict[str, str] with the highest precedence.

    Returns:
        dict[str, str] of environment variables.
    """
    env = dict(ctx.configuration.default_shell_env)
    env.update(tc_info.env)
    env.update(extra or {})
    return env

def rlocationpath(file, workspace_name):
    """Runfiles-library key for a File.

    `<workspace>/<short_path>` for main-repo files; external files'
    `short_path` already starts `../<repo>/`, which becomes `<repo>/`.

    Args:
        file: a File.
        workspace_name: string; `ctx.workspace_name`.

    Returns:
        string; the key a runfiles library resolves to an absolute path.
    """
    if file.short_path.startswith("../"):
        return file.short_path.removeprefix("../")
    return "{}/{}".format(workspace_name, file.short_path)

# Location-template keywords, mapped to the rlocation form that means
# something at test time (`location` -> `rlocationpath`, plural likewise).
_RLOCATION_KEYWORDS = {
    "location": "rlocationpath",
    "execpath": "rlocationpath",
    "rootpath": "rlocationpath",
    "rlocationpath": "rlocationpath",
    "locations": "rlocationpaths",
    "execpaths": "rlocationpaths",
    "rootpaths": "rlocationpaths",
    "rlocationpaths": "rlocationpaths",
}

# Wraps each expanded template. The launcher (`tools/riviera_launcher`)
# resolves every whitespace-separated key inside `@rloc{...}` to an
# absolute path, leaving the surrounding literal text alone. Keys never
# contain `}` or whitespace.
RLOC_TAG_OPEN = "@rloc{"
RLOC_TAG_CLOSE = "}"

def expand_rlocation_templates(ctx, value, targets):
    """Expand location templates in `value` into launcher-resolvable tags.

    Each `$(<keyword> <label>)` becomes `@rloc{$(rlocationpath <label>)}`
    (plural keywords use `rlocationpaths`) *before* `ctx.expand_location`
    runs, so the result carries one tag per template with the rlocation
    key inside it and literal text (`-f `, `+define+X=`) intact. Other
    `$(...)` text is left for `ctx.expand_location` to handle as usual.

    Args:
        ctx: rule context.
        value: string possibly containing `$(...)` templates.
        targets: list of Target to expand labels against (the `data` attr).

    Returns:
        string; `value` with every location template replaced by a tag.
    """
    if "$(" not in value:
        return value
    out = ""
    rest = value
    for _ in range(len(value)):
        start = rest.find("$(")
        end = rest.find(")", start)
        if start < 0 or end < 0:
            break
        out += rest[:start]
        parts = rest[start + 2:end].split(" ", 1)
        keyword = _RLOCATION_KEYWORDS.get(parts[0].strip()) if len(parts) == 2 else None
        if keyword:
            out += "{}$({} {}){}".format(RLOC_TAG_OPEN, keyword, parts[1].strip(), RLOC_TAG_CLOSE)
        else:
            out += rest[start:end + 1]
        rest = rest[end + 1:]
    return ctx.expand_location(out + rest, targets = targets)
