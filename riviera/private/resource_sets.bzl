"""Static `ctx.actions.run.resource_set` callables per CPU-slot count.

Follows the pattern from `~/Code/rules_rust/rust/private/rustc_resource_set.bzl`:
Bazel's resource_set callable is identity-compared for action-graph
determinism, so each supported CPU count gets its own top-level
`def`, and consumers look them up in `_RESOURCE_SETS` rather than
constructing new closures per call.

Supports `1..=64` CPU slots. Requests above the max clamp to 64.
"""

def _cpu_1(_os_name, _inputs):
    return {"cpu": 1}  # buildifier: disable=one-line-def

def _cpu_2(_os_name, _inputs):
    return {"cpu": 2}  # buildifier: disable=one-line-def

def _cpu_3(_os_name, _inputs):
    return {"cpu": 3}  # buildifier: disable=one-line-def

def _cpu_4(_os_name, _inputs):
    return {"cpu": 4}  # buildifier: disable=one-line-def

def _cpu_5(_os_name, _inputs):
    return {"cpu": 5}  # buildifier: disable=one-line-def

def _cpu_6(_os_name, _inputs):
    return {"cpu": 6}  # buildifier: disable=one-line-def

def _cpu_7(_os_name, _inputs):
    return {"cpu": 7}  # buildifier: disable=one-line-def

def _cpu_8(_os_name, _inputs):
    return {"cpu": 8}  # buildifier: disable=one-line-def

def _cpu_9(_os_name, _inputs):
    return {"cpu": 9}  # buildifier: disable=one-line-def

def _cpu_10(_os_name, _inputs):
    return {"cpu": 10}  # buildifier: disable=one-line-def

def _cpu_11(_os_name, _inputs):
    return {"cpu": 11}  # buildifier: disable=one-line-def

def _cpu_12(_os_name, _inputs):
    return {"cpu": 12}  # buildifier: disable=one-line-def

def _cpu_13(_os_name, _inputs):
    return {"cpu": 13}  # buildifier: disable=one-line-def

def _cpu_14(_os_name, _inputs):
    return {"cpu": 14}  # buildifier: disable=one-line-def

def _cpu_15(_os_name, _inputs):
    return {"cpu": 15}  # buildifier: disable=one-line-def

def _cpu_16(_os_name, _inputs):
    return {"cpu": 16}  # buildifier: disable=one-line-def

def _cpu_17(_os_name, _inputs):
    return {"cpu": 17}  # buildifier: disable=one-line-def

def _cpu_18(_os_name, _inputs):
    return {"cpu": 18}  # buildifier: disable=one-line-def

def _cpu_19(_os_name, _inputs):
    return {"cpu": 19}  # buildifier: disable=one-line-def

def _cpu_20(_os_name, _inputs):
    return {"cpu": 20}  # buildifier: disable=one-line-def

def _cpu_21(_os_name, _inputs):
    return {"cpu": 21}  # buildifier: disable=one-line-def

def _cpu_22(_os_name, _inputs):
    return {"cpu": 22}  # buildifier: disable=one-line-def

def _cpu_23(_os_name, _inputs):
    return {"cpu": 23}  # buildifier: disable=one-line-def

def _cpu_24(_os_name, _inputs):
    return {"cpu": 24}  # buildifier: disable=one-line-def

def _cpu_25(_os_name, _inputs):
    return {"cpu": 25}  # buildifier: disable=one-line-def

def _cpu_26(_os_name, _inputs):
    return {"cpu": 26}  # buildifier: disable=one-line-def

def _cpu_27(_os_name, _inputs):
    return {"cpu": 27}  # buildifier: disable=one-line-def

def _cpu_28(_os_name, _inputs):
    return {"cpu": 28}  # buildifier: disable=one-line-def

def _cpu_29(_os_name, _inputs):
    return {"cpu": 29}  # buildifier: disable=one-line-def

def _cpu_30(_os_name, _inputs):
    return {"cpu": 30}  # buildifier: disable=one-line-def

def _cpu_31(_os_name, _inputs):
    return {"cpu": 31}  # buildifier: disable=one-line-def

def _cpu_32(_os_name, _inputs):
    return {"cpu": 32}  # buildifier: disable=one-line-def

def _cpu_33(_os_name, _inputs):
    return {"cpu": 33}  # buildifier: disable=one-line-def

def _cpu_34(_os_name, _inputs):
    return {"cpu": 34}  # buildifier: disable=one-line-def

def _cpu_35(_os_name, _inputs):
    return {"cpu": 35}  # buildifier: disable=one-line-def

def _cpu_36(_os_name, _inputs):
    return {"cpu": 36}  # buildifier: disable=one-line-def

def _cpu_37(_os_name, _inputs):
    return {"cpu": 37}  # buildifier: disable=one-line-def

def _cpu_38(_os_name, _inputs):
    return {"cpu": 38}  # buildifier: disable=one-line-def

def _cpu_39(_os_name, _inputs):
    return {"cpu": 39}  # buildifier: disable=one-line-def

def _cpu_40(_os_name, _inputs):
    return {"cpu": 40}  # buildifier: disable=one-line-def

def _cpu_41(_os_name, _inputs):
    return {"cpu": 41}  # buildifier: disable=one-line-def

def _cpu_42(_os_name, _inputs):
    return {"cpu": 42}  # buildifier: disable=one-line-def

def _cpu_43(_os_name, _inputs):
    return {"cpu": 43}  # buildifier: disable=one-line-def

def _cpu_44(_os_name, _inputs):
    return {"cpu": 44}  # buildifier: disable=one-line-def

def _cpu_45(_os_name, _inputs):
    return {"cpu": 45}  # buildifier: disable=one-line-def

def _cpu_46(_os_name, _inputs):
    return {"cpu": 46}  # buildifier: disable=one-line-def

def _cpu_47(_os_name, _inputs):
    return {"cpu": 47}  # buildifier: disable=one-line-def

def _cpu_48(_os_name, _inputs):
    return {"cpu": 48}  # buildifier: disable=one-line-def

def _cpu_49(_os_name, _inputs):
    return {"cpu": 49}  # buildifier: disable=one-line-def

def _cpu_50(_os_name, _inputs):
    return {"cpu": 50}  # buildifier: disable=one-line-def

def _cpu_51(_os_name, _inputs):
    return {"cpu": 51}  # buildifier: disable=one-line-def

def _cpu_52(_os_name, _inputs):
    return {"cpu": 52}  # buildifier: disable=one-line-def

def _cpu_53(_os_name, _inputs):
    return {"cpu": 53}  # buildifier: disable=one-line-def

def _cpu_54(_os_name, _inputs):
    return {"cpu": 54}  # buildifier: disable=one-line-def

def _cpu_55(_os_name, _inputs):
    return {"cpu": 55}  # buildifier: disable=one-line-def

def _cpu_56(_os_name, _inputs):
    return {"cpu": 56}  # buildifier: disable=one-line-def

def _cpu_57(_os_name, _inputs):
    return {"cpu": 57}  # buildifier: disable=one-line-def

def _cpu_58(_os_name, _inputs):
    return {"cpu": 58}  # buildifier: disable=one-line-def

def _cpu_59(_os_name, _inputs):
    return {"cpu": 59}  # buildifier: disable=one-line-def

def _cpu_60(_os_name, _inputs):
    return {"cpu": 60}  # buildifier: disable=one-line-def

def _cpu_61(_os_name, _inputs):
    return {"cpu": 61}  # buildifier: disable=one-line-def

def _cpu_62(_os_name, _inputs):
    return {"cpu": 62}  # buildifier: disable=one-line-def

def _cpu_63(_os_name, _inputs):
    return {"cpu": 63}  # buildifier: disable=one-line-def

def _cpu_64(_os_name, _inputs):
    return {"cpu": 64}  # buildifier: disable=one-line-def

_RESOURCE_SETS = {
    1: _cpu_1,
    2: _cpu_2,
    3: _cpu_3,
    4: _cpu_4,
    5: _cpu_5,
    6: _cpu_6,
    7: _cpu_7,
    8: _cpu_8,
    9: _cpu_9,
    10: _cpu_10,
    11: _cpu_11,
    12: _cpu_12,
    13: _cpu_13,
    14: _cpu_14,
    15: _cpu_15,
    16: _cpu_16,
    17: _cpu_17,
    18: _cpu_18,
    19: _cpu_19,
    20: _cpu_20,
    21: _cpu_21,
    22: _cpu_22,
    23: _cpu_23,
    24: _cpu_24,
    25: _cpu_25,
    26: _cpu_26,
    27: _cpu_27,
    28: _cpu_28,
    29: _cpu_29,
    30: _cpu_30,
    31: _cpu_31,
    32: _cpu_32,
    33: _cpu_33,
    34: _cpu_34,
    35: _cpu_35,
    36: _cpu_36,
    37: _cpu_37,
    38: _cpu_38,
    39: _cpu_39,
    40: _cpu_40,
    41: _cpu_41,
    42: _cpu_42,
    43: _cpu_43,
    44: _cpu_44,
    45: _cpu_45,
    46: _cpu_46,
    47: _cpu_47,
    48: _cpu_48,
    49: _cpu_49,
    50: _cpu_50,
    51: _cpu_51,
    52: _cpu_52,
    53: _cpu_53,
    54: _cpu_54,
    55: _cpu_55,
    56: _cpu_56,
    57: _cpu_57,
    58: _cpu_58,
    59: _cpu_59,
    60: _cpu_60,
    61: _cpu_61,
    62: _cpu_62,
    63: _cpu_63,
    64: _cpu_64,
}

MAX_CPUS = 64

def cpu_resource_set(cpus):
    """Return a `resource_set` callable requesting `cpus` CPUs.

    Args:
        cpus: int; requested CPU slots. Values <= 0 return None (no
            resource_set — Bazel decides). Values > MAX_CPUS clamp
            down to MAX_CPUS.

    Returns:
        A static `def(os_name, num_inputs) -> {'cpu': N}` callable,
        or None.
    """
    if cpus <= 0:
        return None
    if cpus > MAX_CPUS:
        cpus = MAX_CPUS
    return _RESOURCE_SETS[cpus]
