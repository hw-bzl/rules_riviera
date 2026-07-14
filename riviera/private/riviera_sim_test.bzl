"""The `riviera_sim_test` rule implementation."""

load(":providers.bzl", "RivieraLibraryInfo")
load(
    ":utils.bzl",
    "resolve_jobs",
    "riviera_action_env",
    "riviera_execution_requirements",
    "rlocationpath",
)

TOOLCHAIN_TYPE = str(Label("//riviera:toolchain_type"))

# Sentinel the rule prepends to `--asim-arg` values that were expanded
# from a Bazel location template (`$(location ...)`, `$(rlocationpath
# ...)`, etc.). The launcher recognizes it and runfiles-resolves the
# rest of the string to an absolute path.
_RLOC_PREFIX = "@rloc:"

def _riviera_sim_test_impl(ctx):
    tc = ctx.toolchains[TOOLCHAIN_TYPE].riviera_info

    # `link_libraries` (external precompiled bundles — Xilinx simlibs,
    # DUT-precompiled export bundles) are vmap'd BEFORE `deps` (the
    # primary libraries under test) so that when two contributors expose
    # the same logical library name — a vendor bundle shipping the DUT
    # under some name, and the testbench's own `riviera_library`
    # compiling into that same name — the `deps`-side mapping wins.
    # Aldec's vmap is last-write-wins; hierarchical control via distinct
    # attrs, rather than label-order-dependent shuffling, keeps caller
    # BUILD files order-independent.
    link_libs = [d[RivieraLibraryInfo] for d in ctx.attr.link_libraries]
    dep_libs = [d[RivieraLibraryInfo] for d in ctx.attr.deps]
    if not dep_libs:
        fail("riviera_sim_test {}: `deps` must include at least one riviera_library.".format(ctx.label))
    libs = link_libs + dep_libs

    # `ctx.coverage_instrumented()` on a test target is gated on
    # `--instrument_test_targets` (off by default), so it hides the
    # coverage run when the user just wants HDL-level coverage. Use
    # the config-level flag instead — it flips True the moment
    # `bazel coverage` is running.
    coverage_on = ctx.configuration.coverage_enabled

    args = ctx.actions.args()
    args.set_param_file_format("multiline")

    args.add("--vsimsa-rloc", rlocationpath(tc.vsimsa.files_to_run.executable, ctx.workspace_name))
    # Emit `--library <name>:<rloc>` per dep — the launcher uses these
    # to generate `vmap <name> <abs_path>` commands with a proper
    # absolute path. Using `vmap -link <library.cfg>` alone doesn't
    # work: the linked .cfg's relative paths (`./<name>.lib`) resolve
    # against the SESSION's CWD (`$TEST_TMPDIR`), not the .cfg's
    # directory — so asim can't find the library at elaboration.
    for lib in libs:
        args.add("--library", "{}:{}".format(
            lib.name,
            rlocationpath(lib.library_dir, ctx.workspace_name),
        ))
    args.add("--top", ctx.attr.top)
    for extra_top in ctx.attr.extra_tops:
        args.add("--extra-top", extra_top)
    for k, v in ctx.attr.generics.items():
        args.add("--generic", "{}={}".format(k, v))

    # asim_opts: expand location templates against `data`. When a value
    # changed under expansion it referenced a runfile, so tag it with
    # `@rloc:` for the launcher to resolve at test time. Literal
    # strings pass through untouched.
    for opt in ctx.attr.asim_opts:
        expanded = ctx.expand_location(opt, targets = ctx.attr.data)
        if expanded != opt:
            args.add("--asim-arg", _RLOC_PREFIX + expanded)
        else:
            args.add("--asim-arg", opt)

    if coverage_on:
        args.add("--coverage")
    if ctx.attr.pre_processor:
        args.add("--pre-process-rloc", rlocationpath(ctx.executable.pre_processor, ctx.workspace_name))
    if ctx.attr.post_processor:
        args.add("--post-process-rloc", rlocationpath(ctx.executable.post_processor, ctx.workspace_name))

    args_file = ctx.actions.declare_file("{}_launcher.args".format(ctx.label.name))
    ctx.actions.write(args_file, content = args)

    extra_files = [args_file, ctx.executable._launcher] + [lib.library_dir for lib in libs]
    if ctx.attr.pre_processor:
        extra_files.append(ctx.executable.pre_processor)
    if ctx.attr.post_processor:
        extra_files.append(ctx.executable.post_processor)

    runfile_deps = list(ctx.attr.link_libraries) + list(ctx.attr.deps) + list(ctx.attr.data)
    if ctx.attr.pre_processor:
        runfile_deps.append(ctx.attr.pre_processor)
    if ctx.attr.post_processor:
        runfile_deps.append(ctx.attr.post_processor)

    runfiles = ctx.runfiles(files = extra_files + ctx.files.data).merge_all(
        [
            tc.vsimsa.runfiles,
            ctx.attr._launcher[DefaultInfo].default_runfiles,
        ] +
        [t[DefaultInfo].default_runfiles for t in runfile_deps],
    )

    launcher = ctx.actions.declare_file(ctx.label.name)
    ctx.actions.symlink(
        output = launcher,
        target_file = ctx.executable._launcher,
        is_executable = True,
    )

    # env: same location-template expansion as asim_opts. Values that
    # changed under expansion get the `@rloc:` sentinel; the launcher
    # resolves those to absolute paths and re-`setenv`s them before
    # spawning vsimsa. Literal strings pass through untouched.
    expanded_env = {}
    for k, v in ctx.attr.env.items():
        expanded = ctx.expand_location(v, targets = ctx.attr.data)
        if expanded != v:
            expanded_env[k] = _RLOC_PREFIX + expanded
        else:
            expanded_env[k] = v
    env = riviera_action_env(ctx, tc, expanded_env)
    env["RIVIERA_ARGS_FILE_RLOC"] = rlocationpath(args_file, ctx.workspace_name)

    jobs = resolve_jobs(ctx.attr.jobs, tc.jobs)

    return [
        DefaultInfo(
            executable = launcher,
            runfiles = runfiles,
        ),
        testing.ExecutionInfo(
            requirements = riviera_execution_requirements(tc, ctx.attr.exec_requirements, jobs),
        ),
        testing.TestEnvironment(env),
        coverage_common.instrumented_files_info(
            ctx,
            dependency_attributes = ["deps"],
            extensions = ["v", "sv", "vhd", "vhdl"],
        ),
    ]

riviera_sim_test = rule(
    implementation = _riviera_sim_test_impl,
    doc = """Elaborate + simulate a Riviera library under `vsimsa` as a Bazel test.

Takes one or more `deps` — each a `riviera_library` — and a fully-qualified
`top` (e.g. `"my_lib.my_tb"`) that `asim` elaborates. Every dep's library
directory is `vmap -link`ed before elaboration, so cross-library
instantiation works out of the box.

`asim_opts` supports Bazel's `$(location ...)` / `$(rlocationpath ...)`
templates against targets in `data`. Values with a template are
runfiles-resolved to absolute paths at test time — useful for wiring
`-f <file>`, `-pli <shared_lib>`, or any other flag whose value is a
Bazel-tracked runfile. Literal strings pass through verbatim.

Rule ↔ launcher hand-off:

1. The rule builds a `ctx.actions.args()` bundle describing everything
   the launcher needs (tool rlocation keys, per-library rlocs, top,
   generics, asim args, coverage flag) and writes it to a multiline
   param file via `ctx.actions.write(content = args)`.
2. It exposes exactly one env var — `RIVIERA_ARGS_FILE_RLOC` — whose
   value is the args file's runfiles-library key.
3. The launcher (`//tools/riviera_launcher`) reads the env, resolves
   the args file via the runfiles library, argparse-parses it,
   resolves every `*_rloc` field to an absolute path, constructs the
   `.do` at runtime, and hands vsimsa the absolute path.

Coverage kicks in automatically under `bazel coverage`: the launcher
adds `-acdb -acdb_cov sbceam` to the `.do`, renders html + text into
`$TEST_UNDECLARED_OUTPUTS_DIR`, and converts the text report to lcov
via the linked-in `acdb2lcov` for `$COVERAGE_OUTPUT_FILE`.

Seeding — `RIVIERA_SEED` carries the simulation seed in both
directions. Set it (`--test_env=RIVIERA_SEED=42`, or from a
`pre_processor` env fragment) to pin the run; leave it unset and the
launcher picks a fresh value per run. Either way the resolved seed
is passed to `asim` as `-random_seed`/`-sv_seed`, printed to the test
log as `riviera_sim_test seed: <n>`, and re-exported as
`RIVIERA_SEED` so the simulation and anything it spawns can read it.
No other seed variable is set — a harness wanting its framework's own
variable should set it alongside `RIVIERA_SEED` from a single value in
its `pre_processor`.""",
    attrs = {
        "asim_opts": attr.string_list(
            doc = "Extra arguments forwarded to `asim`. Supports `$(location ...)` / `$(rlocationpath ...)` expansion against targets in `data`; expanded values are runfiles-resolved at test time.",
            default = [],
        ),
        "data": attr.label_list(
            doc = "Runtime data (memory init files, custom .f lists, VPI shared libs, etc.) staged into runfiles. Targets referenced from `asim_opts` templates must appear here.",
            allow_files = True,
        ),
        "deps": attr.label_list(
            doc = "One or more `riviera_library` targets whose logical libraries hold the design under test (including the `top`). Vmap'd AFTER `link_libraries`, so when a library name collides between the two, the `deps`-side mapping wins. Order within this list is not semantically meaningful — if you find yourself relying on it, split the collision into a separate `link_libraries` entry.",
            providers = [RivieraLibraryInfo],
            mandatory = True,
        ),
        "link_libraries": attr.label_list(
            doc = "Targets providing `RivieraLibraryInfo` that are vmap'd BEFORE `deps` — a `riviera_library`, or any rule of your own wrapping an already-compiled bundle. Use for external, pre-built libraries (Xilinx simlibs, precompiled IP/BD exports) the design under test references but doesn't own. Placing them here (instead of alongside `deps`) makes ordering hierarchical: the `deps` mapping always wins any name collision, so caller BUILDs never have to remember which label comes first in a list.",
            providers = [RivieraLibraryInfo],
            default = [],
        ),
        "env": attr.string_dict(
            doc = "Extra environment variables for the test action. Values support `$(location ...)` / `$(rlocationpath ...)` expansion against targets in `data`; expanded values are runfiles-resolved at test time. Literal strings pass through verbatim.",
            default = {},
        ),
        "exec_requirements": attr.string_dict(
            doc = "Extra execution_requirements merged on top of the defaults (`resources:riviera_license=1`, plus `requires-network` when the toolchain says so). Use to override license slot count, force `no-remote-exec`, request a named resource pool, etc.",
            default = {},
        ),
        "generics": attr.string_dict(
            doc = "Elaboration-time parameters / generics, bound by name at every level of the hierarchy (mapped to `-G<name>=<value>`).",
            default = {},
        ),
        "jobs": attr.int(
            doc = "CPU slots requested for the test. `-1` (default) inherits the toolchain's `jobs` field; `0` explicitly means no hint; `N > 0` adds `cpu:N` to `testing.ExecutionInfo.requirements`. Only hints the scheduler — pass any Riviera-specific parallel simulator flags (`-j`, `-threads`, `-mfcu`, etc.) through `asim_opts`.",
            default = -1,
        ),
        "post_processor": attr.label(
            doc = "Optional executable invoked after a successful simulation, before the test returns. Runs in the test's working directory — the runfiles root, like any other Bazel executable — so relative paths reach data deps the usual way.\n\nEverything the simulation produced is reachable through the environment rather than the CWD: vsimsa runs with its CWD set to `$TEST_TMPDIR`, so `coverage.acdb`, `coverage.txt`, `coverage_html/`, and anything the testbench itself wrote all land there. `$COVERAGE_OUTPUT_FILE` and `$TEST_UNDECLARED_OUTPUTS_DIR` are set as usual, alongside any `env` from the rule.\n\nA non-zero exit fails the test.",
            executable = True,
            cfg = "target",
        ),
        "pre_processor": attr.label(
            doc = "Optional executable target invoked before vsimsa, in `$TEST_TMPDIR` with the test's runfiles + env. Any executable label works (`py_binary`, `sh_binary`, `py_venv_binary`, custom rules setting `DefaultInfo(executable = ...)`, etc.).\n\n**Env-fragment contract:** the launcher exports `RIVIERA_PRE_PROCESSOR_ENV_FILE=<path>` when invoking the pre_processor. If the pre_processor writes `K=V\\n`-per-line contents to that file, the launcher parses each line and `setenv`s it before spawning vsimsa. Blank lines and lines starting with `#` are skipped; malformed lines fail the test loudly. This is the intended channel for pre_processor-computed values (venv paths, per-run tmpdirs, etc.) that the sim needs in env.\n\nA non-zero exit fails the test.",
            executable = True,
            cfg = "target",
        ),
        "top": attr.string(
            doc = "Fully-qualified HDL top-level unit passed to `asim`, e.g. `\"my_lib.my_tb\"`.",
            mandatory = True,
        ),
        "extra_tops": attr.string_list(
            doc = "Additional fully-qualified top-level units appended to `top` on the asim command line. asim instantiates each at simulator root — use for co-elaborating helper modules like Xilinx `xil_defaultlib.glbl` whose internal signals (`glbl.GSR`, `glbl.GTS`) must exist for hierarchical references from user HDL (`xpm_cdc`, `xpm_fifo`, …) to resolve at elaboration.",
            default = [],
        ),
        "_launcher": attr.label(
            default = Label("//tools/riviera_launcher"),
            executable = True,
            cfg = "target",
        ),
    },
    toolchains = [TOOLCHAIN_TYPE],
    test = True,
)
