"""The `riviera_sim_test` rule implementation."""

load(":providers.bzl", "RivieraLibraryInfo")
load(
    ":utils.bzl",
    "HDL_SOURCE_EXTENSIONS",
    "TOOLCHAIN_TYPE",
    "expand_rlocation_templates",
    "resolve_jobs",
    "riviera_action_env",
    "riviera_execution_requirements",
    "rlocationpath",
)

def _riviera_sim_test_impl(ctx):
    tc = ctx.toolchains[TOOLCHAIN_TYPE].riviera_info

    if not ctx.attr.deps:
        fail("riviera_sim_test {}: `deps` must include at least one riviera_library.".format(ctx.label))

    # link_libraries first: vmap is last-write-wins, so `deps` take
    # precedence on a name collision.
    libs = [d[RivieraLibraryInfo] for d in ctx.attr.link_libraries + ctx.attr.deps]
    processors = [t for t in (ctx.attr.pre_processor, ctx.attr.post_processor) if t]

    # `ctx.coverage_instrumented()` on a test target is gated on
    # `--instrument_test_targets` (off by default), so it hides the
    # coverage run when the user just wants HDL-level coverage. Use
    # the config-level flag instead — it flips True the moment
    # `bazel coverage` is running.
    coverage_on = ctx.configuration.coverage_enabled

    args = ctx.actions.args()
    args.set_param_file_format("multiline")

    args.add("--vsimsa-rloc", rlocationpath(tc.vsimsa.files_to_run.executable, ctx.workspace_name))
    for lib in libs:
        args.add("--library", rlocationpath(lib.library_dir, ctx.workspace_name))
    args.add("--top", ctx.attr.top)
    for extra_top in ctx.attr.extra_tops:
        args.add("--extra-top", extra_top)
    for k, v in ctx.attr.generics.items():
        args.add("--generic", "{}={}".format(k, v))

    # Location templates become `@rloc{...}` tags the launcher resolves.
    for opt in ctx.attr.asim_opts:
        args.add("--asim-arg", expand_rlocation_templates(ctx, opt, ctx.attr.data))

    if coverage_on:
        args.add("--coverage")
    if ctx.attr.pre_processor:
        args.add("--pre-process-rloc", rlocationpath(ctx.executable.pre_processor, ctx.workspace_name))
    if ctx.attr.post_processor:
        args.add("--post-process-rloc", rlocationpath(ctx.executable.post_processor, ctx.workspace_name))

    args_file = ctx.actions.declare_file("{}_launcher.args".format(ctx.label.name))
    ctx.actions.write(args_file, content = args)

    runfile_deps = ctx.attr.link_libraries + ctx.attr.deps + ctx.attr.data + processors
    runfiles = ctx.runfiles(
        files = [args_file, ctx.executable._launcher] + [lib.library_dir for lib in libs] +
                [t[DefaultInfo].files_to_run.executable for t in processors] + ctx.files.data,
    ).merge_all(
        [tc.vsimsa.runfiles, ctx.attr._launcher[DefaultInfo].default_runfiles] +
        [t[DefaultInfo].default_runfiles for t in runfile_deps],
    )

    launcher = ctx.actions.declare_file(ctx.label.name)
    ctx.actions.symlink(
        output = launcher,
        target_file = ctx.executable._launcher,
        is_executable = True,
    )

    expanded_env = {
        k: expand_rlocation_templates(ctx, v, ctx.attr.data)
        for k, v in ctx.attr.env.items()
    }
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
            extensions = HDL_SOURCE_EXTENSIONS,
        ),
    ]

riviera_sim_test = rule(
    implementation = _riviera_sim_test_impl,
    doc = """Elaborate + simulate a Riviera library under `vsimsa` as a Bazel test.

Takes one or more `deps` — each a `riviera_library` — and a fully-qualified
`top` (e.g. `"my_lib.my_tb"`) that `asim` elaborates. Every library a
dep's `library.cfg` declares is `vmap`ped by absolute path before
elaboration, so cross-library instantiation works out of the box.

`asim_opts` and `env` support Bazel's location templates
(`$(location ...)`, `$(rlocationpath ...)`, `execpath` / `rootpath` and
their plural forms) against targets in `data`. Each template is
runfiles-resolved to an absolute path at test time, in place, so
`-f $(location :f)` or `+define+HEX=$(rlocationpath :rom)` work as
written.

Coverage kicks in automatically under `bazel coverage`: the launcher
adds Riviera's ACDB coverage flags, renders html + text reports into
`$TEST_UNDECLARED_OUTPUTS_DIR`, and converts the text report to lcov
via `acdb2lcov` for `$COVERAGE_OUTPUT_FILE`.

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
            doc = "Extra arguments forwarded to `asim`. Location templates expand against `data` (see the rule doc).",
            default = [],
        ),
        "data": attr.label_list(
            doc = "Runtime data (memory init files, `.f` lists, VPI shared libs, ...) staged into runfiles. Targets referenced from `asim_opts` / `env` templates must appear here.",
            allow_files = True,
        ),
        "deps": attr.label_list(
            doc = "`riviera_library` targets holding the design under test, including `top`.",
            providers = [RivieraLibraryInfo],
            mandatory = True,
        ),
        "link_libraries": attr.label_list(
            doc = "`RivieraLibraryInfo` targets for pre-built libraries the design references but doesn't own (Xilinx simlibs, precompiled IP). Vmapped before `deps`, so a `deps` library wins any name collision.",
            providers = [RivieraLibraryInfo],
            default = [],
        ),
        "env": attr.string_dict(
            doc = "Extra environment variables for the test. Location templates expand against `data` (see the rule doc).",
            default = {},
        ),
        "exec_requirements": attr.string_dict(
            doc = "Extra execution_requirements merged over the defaults (`resources:riviera_license=1`, plus `requires-network` when the toolchain sets it).",
            default = {},
        ),
        "generics": attr.string_dict(
            doc = "Elaboration-time parameters / generics, bound by name at every level of the hierarchy (mapped to `-G<name>=<value>`).",
            default = {},
        ),
        "jobs": attr.int(
            doc = "CPU slots to request from Bazel's scheduler. `-1` inherits the toolchain's `jobs`; `N > 0` adds `cpu:N`; anything else is no hint. Simulator parallelism flags go in `asim_opts`.",
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
