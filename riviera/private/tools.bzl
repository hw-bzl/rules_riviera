"""The set of Riviera-PRO tools `riviera_toolchain` binds.

Single source of truth, loaded by both the toolchain rule and the shims
package. The two are coupled by a computed label
(`//riviera/private/shims:{name}`), so a drift between separate copies
would surface only as an unresolved-label error in a downstream repo's
toolchain, not at the edit site.
"""

# `vsimsa` is the only load-bearing tool: it is the one binary the rules
# invoke across the Bazel action boundary, from `riviera_library`'s
# compile action and from `riviera_sim_test` at test time. Everything
# else — `acdb` and the `riviera` IDE included — either runs inside
# vsimsa's TCL interpreter or is bound purely so consumer-authored rules
# have somewhere to reach.
MANDATORY_TOOLS = ["vsimsa"]

OPTIONAL_TOOLS = ["riviera", "vsim", "alib", "alog", "acom", "acdb", "asim", "vmap"]

TOOL_NAMES = MANDATORY_TOOLS + OPTIONAL_TOOLS
