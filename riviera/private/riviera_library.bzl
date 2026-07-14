"""The `riviera_library` rule implementation."""

load("@rules_verilog//verilog:defs.bzl", "VerilogInfo")
load("@rules_vhdl//vhdl:defs.bzl", "VhdlInfo")
load(":providers.bzl", "RivieraLibraryInfo")
load(":resource_sets.bzl", "cpu_resource_set")
load(
    ":utils.bzl",
    "HDL_FILE_EXTENSIONS",
    "HDL_SOURCE_EXTENSIONS",
    "TOOLCHAIN_TYPE",
    "collect_hdl_sources",
    "determine_language",
    "include_dirs",
    "resolve_jobs",
    "riviera_action_env",
    "riviera_execution_requirements",
)

def _args_map_verilog_parts(value):
    lib_name, alog_opts, headers, includes, verilog = value
    parts = ["alog", "-work", lib_name]
    parts.extend(alog_opts)
    for inc in include_dirs(headers, includes):
        parts.append("+incdir+{}".format(inc))
    parts.extend([f.path for f in verilog])
    return " ".join(parts)

def _args_map_vhdl_parts(value):
    lib_name, acom_opts, vhdl = value
    parts = ["acom", "-work", lib_name]
    parts.extend(acom_opts)
    parts.extend([f.path for f in vhdl])
    return " ".join(parts)

def _build_do_script(ctx, output, lib_name, library_dir, verilog, vhdl, headers, includes, alog_opts, acom_opts):
    """Write the `.do` that creates, maps and compiles the library.

    Pre-compiled `link_libraries` are not mapped here; the wrapper
    prepends those (`--link-library-dir`).

    Args:
        ctx: ctx; The rule's context object
        output: File; The output path of the `.do` script.
        lib_name: string; logical Riviera library name.
        library_dir: File; The output library dir.
        verilog: list of Verilog/SystemVerilog source Files.
        vhdl: list of VHDL source Files.
        headers: list of header Files (paths contribute to `+incdir+`).
        includes: list of str; explicit `+incdir+` search paths.
        alog_opts: list of extra flags forwarded to `alog`.
        acom_opts: list of extra flags forwarded to `acom`.

    Returns:
        File; `output`.
    """
    args = ctx.actions.args()
    args.set_param_file_format("multiline")
    args.add("onerror { quit -code 1 }")

    # alib and vmap must name the same storage subdir,
    # `<library_dir>/<lib_name>.lib`, which is where the wrapper's
    # `library.cfg` points. Mapping `library_dir` itself fails ("Cannot
    # add mapping"): its cfg only exists after a successful compile.
    storage = "%s/" + lib_name + ".lib"
    args.add_all([library_dir], expand_directories = False, format_each = "alib " + lib_name + " " + storage)
    args.add_all([library_dir], expand_directories = False, format_each = "vmap " + lib_name + " " + storage)

    if verilog:
        args.add_all([(lib_name, alog_opts, headers, includes, verilog)], map_each = _args_map_verilog_parts)
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
        includes = srcs.includes,
        alog_opts = alog_opts,
        acom_opts = acom_opts,
    )

    inputs = depset(
        srcs.verilog + srcs.vhdl + srcs.headers + [do_file] + link_library_dirs,
        transitive = [srcs.data],
    )

    jobs = resolve_jobs(ctx.attr.jobs, tc.jobs)

    # Paths go in as Files (via `add_all`, since `add` refuses
    # TreeArtifacts) so they stay path-mapped like the `.do` contents.
    args = ctx.actions.args()
    args.add("--vsimsa", tc.vsimsa.files_to_run.executable)
    args.add("--do", do_file)
    args.add("--library-name", lib_name)
    args.add_all([library_dir], before_each = "--library-dir", expand_directories = False)
    args.add_all(link_library_dirs, before_each = "--link-library-dir", expand_directories = False)

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
        resource_set = cpu_resource_set(jobs),
        toolchain = TOOLCHAIN_TYPE,
    )

    return [
        DefaultInfo(
            files = depset([library_dir]),
            # `riviera_sim_test` merges each dep's runfiles, so `data`
            # reaches the simulation this way.
            runfiles = ctx.runfiles(transitive_files = srcs.data),
        ),
        RivieraLibraryInfo(
            name = lib_name,
            library_dir = library_dir,
            language = determine_language(srcs.verilog, srcs.vhdl),
        ),
        coverage_common.instrumented_files_info(
            ctx,
            source_attributes = ["srcs"],
            dependency_attributes = ["deps"],
            extensions = HDL_SOURCE_EXTENSIONS,
        ),
    ]

riviera_library = rule(
    implementation = _riviera_library_impl,
    doc = """Compile the full transitive HDL DAG into ONE self-contained Aldec Riviera library.

`deps` takes `verilog_library` / `vhdl_library` targets (anything
providing `VerilogInfo` or `VhdlInfo`). The rule walks the whole
reachable source graph — own srcs + hdrs, transitive `.deps`, and
cross-language `.vhdl_deps` / `.verilog_deps`, through any number of
language boundaries — and compiles it in one `alib`/`alog`/`acom`
invocation via a generated `vsimsa` `.do` script. `includes` become
`+incdir+` paths; `data` files are compile inputs and are forwarded as
runfiles to `riviera_sim_test`. Output is one TreeArtifact directory; `riviera_sim_test`
`vmap`s it by the logical library name.

`srcs` is an optional escape hatch for HDL files that only make sense
inside this Riviera library (a Riviera-specific stub, a compile-time
wrapper) and don't warrant their own HDL-library target.

Runs through `//tools/riviera_wrapper`, which isolates vsimsa's HOME,
maps `link_libraries`, cleans up the session `library.cfg`, and only
prints the transcript when the compile fails.""",
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
            doc = "`verilog_library` / `vhdl_library` targets; their full transitive source graph is compiled into this library.",
            # OR semantics — each dep must provide EITHER VerilogInfo OR VhdlInfo.
            providers = [[VerilogInfo], [VhdlInfo]],
        ),
        "exec_requirements": attr.string_dict(
            doc = "Extra execution_requirements merged over the defaults (`resources:riviera_license=1`, plus `requires-network` when the toolchain sets it).",
            default = {},
        ),
        "jobs": attr.int(
            doc = "CPU slots to request from Bazel's scheduler. `-1` inherits the toolchain's `jobs`; `N > 0` requests N; anything else is no hint. Compiler parallelism flags go in `alog_opts` / `acom_opts`.",
            default = -1,
        ),
        "library_name": attr.string(
            doc = "Logical Riviera library name. Defaults to the target's label name.",
            default = "",
        ),
        "link_libraries": attr.label_list(
            doc = "Pre-compiled `RivieraLibraryInfo` targets `vmap`ped before this compile, so `library <name>; use <name>.<pkg>.all;` clauses resolve. Unlike `deps`, their sources are not compiled into this library.",
            providers = [[RivieraLibraryInfo]],
            default = [],
        ),
        "srcs": attr.label_list(
            doc = "HDL files compiled directly into this library, after everything from `deps`. Prefer `deps` for anything another target might also want.",
            allow_files = ["." + ext for ext in HDL_FILE_EXTENSIONS],
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
