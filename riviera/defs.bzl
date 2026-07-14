"""Bulk re-export of the public rules_riviera API."""

load(
    "//riviera/private:providers.bzl",
    _RivieraLibraryInfo = "RivieraLibraryInfo",
    _RivieraToolInfo = "RivieraToolInfo",
    _RivieraToolchainInfo = "RivieraToolchainInfo",
)
load(
    "//riviera/private:riviera_library.bzl",
    _riviera_library = "riviera_library",
)
load(
    "//riviera/private:riviera_sim_test.bzl",
    _riviera_sim_test = "riviera_sim_test",
)
load(
    "//riviera/private:riviera_toolchain.bzl",
    _riviera_toolchain = "riviera_toolchain",
)
load("//riviera/private:utils.bzl", _TOOLCHAIN_TYPE = "TOOLCHAIN_TYPE")

RivieraToolchainInfo = _RivieraToolchainInfo
RivieraToolInfo = _RivieraToolInfo
RivieraLibraryInfo = _RivieraLibraryInfo
riviera_toolchain = _riviera_toolchain
riviera_library = _riviera_library
riviera_sim_test = _riviera_sim_test
TOOLCHAIN_TYPE = _TOOLCHAIN_TYPE
