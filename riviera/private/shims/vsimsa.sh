#!/bin/sh
# Default `vsimsa` shim. Assumes vsimsa is on PATH — e.g. inside a
# container whose image installs Riviera-PRO under /opt/aldec/Riviera-PRO
# and pre-sets PATH. Override per-toolchain to hard-code an install path.
exec vsimsa "$@"
