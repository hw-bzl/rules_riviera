"""The `riviera_library` rule implementation."""

load("@rules_verilog//verilog:defs.bzl", "VerilogInfo")
load("@rules_vhdl//vhdl:defs.bzl", "VhdlInfo")
load(":providers.bzl", "RivieraLibraryInfo")
load(
    ":utils.bzl",
    "collect_hdl_sources",
    "determine_language",
    "include_dirs",
    "resolve_jobs",
    "riviera_action_env",
    "riviera_execution_requirements",
    "riviera_resource_set",
)

TOOLCHAIN_TYPE = str(Label("//riviera:toolchain_type"))

def _args_map_path(value):
    return value.path

def _args_map_verilog_parts(value):
    lib_name, alog_opts, headers, verilog = value
    parts = ["alog", "-work", lib_name]
    parts.extend(alog_opts)
    for inc in include_dirs(headers):
        parts.append("+incdir+{}".format(inc))
    parts.extend([f.path for f in verilog])
    return " ".join(parts)

def _args_map_vhdl_parts(value):
    lib_name, acom_opts, vhdl = value
    parts = ["acom", "-work", lib_name]
    parts.extend(acom_opts)
    parts.extend([f.path for f in vhdl])
    return " ".join(parts)

def _build_do_script(ctx, output, lib_name, library_dir, verilog, vhdl, headers, alog_opts, acom_opts, link_library_dirs):
    """Compose the `.do` script vsimsa runs for this library.

    Args:
        ctx: ctx; The rule's context object
        output: File; The output path of the `.do` script.
        lib_name: string; logical Riviera library name.
        library_dir: File; The output library dir.
        verilog: list of Verilog/SystemVerilog source Files.
        vhdl: list of VHDL source Files.
        headers: list of header Files (paths contribute to `+incdir+`).
        alog_opts: list of extra flags forwarded to `alog`.
        acom_opts: list of extra flags forwarded to `acom`.
        link_library_dirs: list of File (TreeArtifacts); pre-compiled
            Riviera library dirs to `vmap -link` before compiling this
            target's own sources. Each dir carries a `library.cfg`
            that maps one or more logical library names to physical
            paths — `vmap -link <dir>` merges every entry in. Used to
            make Xilinx simlibs (`xpm`, `unisim`, etc.) visible when
            user HDL references them via `library xpm; use xpm.foo.all;`
            style clauses.

    Returns:
        string; the complete `.do` script text (newline-terminated).
    """
    args = ctx.actions.args()
    args.set_param_file_format("multiline")
    args.add("onerror { quit -code 1 }")
    # Aldec's library storage convention: `library.cfg` maps a name to
    # a subdirectory `./<name>.lib/` (relative to the .cfg file), and
    # that subdir holds the library's `.lib` + `.mgf` files. Both alib
    # and vmap have to agree on this layout — otherwise vmap writes a
    # `./<name>.lib` reference but alib puts files directly under
    # library_dir with basename-prefixed names, and asim later can't
    # find the library ("Unknown library unit ...").
    #
    # Passing `alib <lib_name> <library_dir>/<lib_name>.lib` tells alib
    # to create the storage subdir at exactly the path vmap will
    # reference. `_args_map_path_subdir` composes the subdir path.
    args.add_all(
        [library_dir],
        expand_directories = False,
        format_each = "alib " + lib_name + " %s/" + lib_name + ".lib",
        map_each = _args_map_path,
    )
    # `%s` is the library_dir TreeArtifact, so the mapping has to name the
    # `<lib_name>.lib` subdir alib just created inside it — pointing amap at
    # the parent fails with `Cannot add mapping "<dir>"`, because the
    # `library.cfg` describing that dir is only written once the compile
    # succeeds (see tools/riviera_wrapper).
    args.add_all(
        [library_dir],
        expand_directories = False,
        format_each = "vmap " + lib_name + " %s/" + lib_name + ".lib",
        map_each = _args_map_path,
    )
    # `vmap -link <dir>` merges the .cfg file inside <dir> into the
    # current session's library.cfg — so link_libraries' entries (xpm,
    # unisim, whatever else) become visible to acom/alog below.
    args.add_all(link_library_dirs, expand_directories = False, format_each = "vmap -link %s", map_each = _args_map_path)

    if verilog:
        args.add_all([(lib_name, alog_opts, headers, verilog)], map_each = _args_map_verilog_parts)
    if vhdl:
        args.add_all([(lib_name, acom_opts, vhdl)], map_each = _args_map_vhdl_parts)

    args.add("quit -code 0")
    args.add("")

    ctx.actions.write(
        output = output,
        content = args,
        mnemonic = "RivieraDoScript",
        execution_requirements = {"supports-path-mapping": ""},
    )

    return output

def _riviera_library_impl(ctx):
    tc = ctx.toolchains[TOOLCHAIN_TYPE].riviera_info
    lib_name = ctx.attr.library_name or ctx.label.name

    srcs = collect_hdl_sources(ctx.attr.deps, ctx.attr.srcs)

    if not (srcs.verilog or srcs.vhdl):
        fail("riviera_library {}: no HDL sources found in deps or srcs.".format(ctx.label))

    # Under `bazel coverage`, add Riviera's `-coverage sbceam` flag at
    # compile time so alog/acom bake statement/branch/condition/expression/
    # assertion/FSM instrumentation into the library. Without this the
    # `-acdb_cov` at simulation time has nothing to record against.
    alog_opts = list(ctx.attr.alog_opts)
    acom_opts = list(ctx.attr.acom_opts)
    if ctx.coverage_instrumented():
        alog_opts.extend(["-coverage", "sbceam"])
        acom_opts.extend(["-coverage", "sbceam"])

    link_library_dirs = [t[RivieraLibraryInfo].library_dir for t in ctx.attr.link_libraries]

    # The on-disk name only has to be unique within this package; Riviera
    # identifies libraries by their vmap logical name, not the dir name.
    library_dir = ctx.actions.declare_directory(ctx.label.name)
    do_file = _build_do_script(
        ctx = ctx,
        output = ctx.actions.declare_file("{}_build.do".format(ctx.label.name)),
        lib_name = lib_name,
        library_dir = library_dir,
        verilog = srcs.verilog,
        vhdl = srcs.vhdl,
        headers = srcs.headers,
        alog_opts = alog_opts,
        acom_opts = acom_opts,
        link_library_dirs = link_library_dirs,
    )

    inputs = depset(srcs.verilog + srcs.vhdl + srcs.headers + [do_file] + link_library_dirs)

    jobs = resolve_jobs(ctx.attr.jobs, tc.jobs)

    args = ctx.actions.args()
    args.add("--vsimsa", tc.vsimsa.files_to_run.executable)
    args.add("--do", do_file)
    args.add("--library-name", lib_name)
    args.add("--library-dir", library_dir.path)

    ctx.actions.run(
        outputs = [library_dir],
        inputs = inputs,
        tools = [tc.vsimsa.files_to_run],
        executable = ctx.executable._wrapper,
        arguments = [args],
        mnemonic = "RivieraCompile",
        progress_message = "RivieraCompile %{label}",
        env = riviera_action_env(ctx, tc),
        execution_requirements = riviera_execution_requirements(tc, ctx.attr.exec_requirements, jobs) | {"supports-path-mapping": ""},
        resource_set = riviera_resource_set(jobs),
        toolchain = TOOLCHAIN_TYPE,
    )

    return [
        DefaultInfo(files = depset([library_dir])),
        RivieraLibraryInfo(
            name = lib_name,
            library_dir = library_dir,
            language = determine_language(srcs.verilog, srcs.vhdl),
        ),
        coverage_common.instrumented_files_info(
            ctx,
            source_attributes = ["srcs"],
            dependency_attributes = ["deps"],
            extensions = ["v", "sv", "vhd", "vhdl"],
        ),
    ]

riviera_library = rule(
    implementation = _riviera_library_impl,
    doc = """Compile the full transitive HDL DAG into ONE self-contained Aldec Riviera library.

`deps` takes `verilog_library` / `vhdl_library` targets (anything
providing `VerilogInfo` or `VhdlInfo`). The rule walks the whole
reachable source graph — own srcs + hdrs, transitive `.deps`, and
cross-language `.vhdl_deps` / `.verilog_deps` — and compiles it in
one `alib`/`alog`/`acom` invocation via a generated `vsimsa` `.do`
script. Output is one TreeArtifact directory; `riviera_sim_test`
`vmap`s it by the logical library name.

`srcs` is an optional escape hatch for HDL files that only make sense
inside this Riviera library (a Riviera-specific stub, a compile-time
wrapper) and don't warrant their own HDL-library target.

Runs through `//tools/riviera_wrapper` so vsimsa's HOME is isolated
per-action (`~/.aldec/` doesn't bleed into the user's real home) and
stdout is buffered — nothing streams unless the action fails or a
`# Error`/`# Fatal` slips past vsimsa's exit code.

Users compose library boundaries by wiring multiple `riviera_library`
targets together via other consumers, not by cross-linking `riviera_library` deps.""",
    attrs = {
        "acom_opts": attr.string_list(
            doc = "Extra flags forwarded to VHDL compilation (`acom`).",
            default = [],
        ),
        "alog_opts": attr.string_list(
            doc = "Extra flags forwarded to Verilog compilation (`alog`)",
            default = [],
        ),
        "deps": attr.label_list(
            doc = "verilog_library / vhdl_library targets. The full reachable source graph (own srcs+hdrs, transitive deps, cross-language deps) is compiled into this Riviera library.",
            # OR semantics — each dep must provide EITHER VerilogInfo OR VhdlInfo.
            providers = [[VerilogInfo], [VhdlInfo]],
        ),
        "exec_requirements": attr.string_dict(
            doc = "Extra execution_requirements merged on top of the defaults (which are `resources:riviera_license=1` and — when the toolchain has `requires_network=True` — `requires-network`). Use to override the license slot count, force `no-remote-exec`, tag a specific resource pool, etc.",
            default = {},
        ),
        "jobs": attr.int(
            doc = "CPU slots to request from Bazel's local scheduler. `-1` (default) inherits the toolchain's `jobs` field; `0` explicitly means no hint; `N > 0` maps to `resource_set = {'cpu': N}`. Only hints the scheduler — pass any Riviera-specific parallel flags (e.g. `-j`, `-mfcu`) via `alog_opts` / `acom_opts`.",
            default = -1,
        ),
        "library_name": attr.string(
            doc = "Logical Riviera library name. Defaults to the target's label name.",
            default = "",
        ),
        "link_libraries": attr.label_list(
            doc = "Pre-compiled `RivieraLibraryInfo`-providing targets whose library dirs are `vmap -link`ed BEFORE this target's own compile. Use to make Xilinx simlibs (`xpm`, `unisim`, `secureip`) or other externally-compiled libraries visible when this target's HDL references them via `library <name>; use <name>.<pkg>.all;` clauses that acom must resolve at compile time. `deps` is for source-level dependencies (their sources get compiled INTO this library); `link_libraries` is for already-compiled deps compiled INTO other libraries (just linked as external references).",
            providers = [[RivieraLibraryInfo]],
            default = [],
        ),
        "srcs": attr.label_list(
            doc = "HDL files compiled directly into this Riviera library, after everything reached through `deps`. An escape hatch for sources that only make sense inside this library — a Riviera-specific stub, a compile-time wrapper — and don't warrant their own `verilog_library` / `vhdl_library` target. Prefer `deps` for anything another target might also want.",
            allow_files = [".v", ".sv", ".vh", ".svh", ".vhd", ".vhdl"],
            default = [],
        ),
        "_wrapper": attr.label(
            default = Label("//tools/riviera_wrapper"),
            executable = True,
            cfg = "exec",
        ),
    },
    toolchains = [TOOLCHAIN_TYPE],
    provides = [RivieraLibraryInfo],
)
