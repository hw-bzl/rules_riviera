"""The set of Riviera-PRO tools `riviera_toolchain` binds.

Single source of truth, loaded by both the toolchain rule and the shims
package. The two are coupled by a computed label
(`//riviera/private/shims:{name}`), so a drift between separate copies
would surface only as an unresolved-label error in a downstream repo's
toolchain, not at the edit site.
"""

# `vsimsa` is the only tool the rules invoke; see `RivieraToolchainInfo`.
MANDATORY_TOOLS = ["vsimsa"]

OPTIONAL_TOOLS = ["riviera", "vsim", "alib", "alog", "acom", "acdb", "asim", "vmap"]

TOOL_NAMES = MANDATORY_TOOLS + OPTIONAL_TOOLS
