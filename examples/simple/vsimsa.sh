#!/bin/sh
# Resolve `vsimsa` from PATH.
#
# Appropriate when Riviera-PRO is already installed in whatever
# environment the Bazel actions execute in — a container image, a build
# host with the tools on PATH. Replace with an absolute install path, or
# with a wrapper that sources a license env file first, as your
# deployment requires.
#
# rules_riviera ships an identical script it uses as the default for the
# optional tool attrs. This copy is deliberate: `vsimsa` is the one attr
# you must bind yourself, and authoring the binding is what this example
# is demonstrating.
exec vsimsa "$@"
