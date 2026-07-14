#!/bin/sh
# Default `riviera` shim (Aldec's full Riviera-PRO IDE). No rule in
# this repo launches it; the toolchain attr exists so a rule of your own
# can. See vsimsa.sh for the pattern.
exec riviera "$@"
