#!/usr/bin/env python3
"""Check the L3 lab documents for ASCII text, relative links and vectors.

The main-worktree proposal and review files are merge inputs that are not in
this worktree at the L3 base. They are reported as expected-missing rather than
as failures; every other relative link must resolve. The arithmetic checks are
independent Python integer/Fraction calculations for the chosen reference
target, the window target, the multiplier stepdown and the compact decoder.
"""

from __future__ import annotations

import hashlib
import re
import sys
from fractions import Fraction
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FILES = [
    ROOT / "docs/REGTEST-LAB-IMPLEMENTATION.md",
    ROOT / "docs/decisions/2026-11-05-lab-share-work-units.md",
    ROOT / "docs/decisions/2026-11-05-window-network-block-work.md",
    ROOT / "docs/changelog/2026-11-05-lab-share-work-units.md",
    ROOT / "docs/changelog/2026-11-05-window-network-block-work.md",
    ROOT / "docs/reviews/2026-11-05-lab-l3-contracts.md",
]

# Files introduced on main after the L3 base; they resolve after merge.
MERGE_INPUTS = {
    "REGTEST-LAB.md",
    "REGTEST-LAB-STATUS.md",
    "reviews/2026-11-05-regtest-lab-review-summary.md",
}

LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")


def check_ascii(path: Path) -> list[str]:
    errors: list[str] = []
    raw = path.read_bytes()
    text = raw.decode("utf-8")
    for number, line in enumerate(text.splitlines(), 1):
        for column, char in enumerate(line, 1):
            if ord(char) > 127:
                errors.append(f"{path}:{number}:{column}: non-ASCII U+{ord(char):04X}")
    return errors


def check_links(path: Path) -> tuple[list[str], list[str]]:
    errors: list[str] = []
    expected: list[str] = []
    text = path.read_text(encoding="utf-8")
    for target in LINK.findall(text):
        target = target.strip().strip("<>")
        if not target or target.startswith(("#", "http://", "https://", "mailto:")):
            continue
        target = target.split("#", 1)[0].split("?", 1)[0]
        if not target:
            continue
        resolved = (path.parent / target).resolve()
        if resolved.exists():
            continue
        try:
            normalized = resolved.relative_to(ROOT / "docs").as_posix()
        except ValueError:
            normalized = target
        if normalized in MERGE_INPUTS or target in MERGE_INPUTS:
            expected.append(f"{path}: expected merge input {target}")
        else:
            errors.append(f"{path}: missing link target {target}")
    return errors, expected


def decode_compact(bits: int) -> int:
    """Consensus-style SetCompact decode used by the vectors."""
    size = bits >> 24
    word = bits & 0x007FFFFF
    if word != 0 and (bits & 0x00800000):
        raise ValueError("sign bit")
    if word != 0 and (
        size > 34 or (word > 0xFF and size > 33) or (word > 0xFFFF and size > 32)
    ):
        raise ValueError("overflow")
    if size <= 3:
        target = word >> (8 * (3 - size))
    else:
        target = word << (8 * (size - 3))
    if target == 0:
        raise ValueError("zero target")
    return target


def check_vectors() -> list[str]:
    errors: list[str] = []

    def require(condition: bool, message: str) -> None:
        if not condition:
            errors.append(message)

    # Compact decoder vectors from the review.
    accepted = {
        0x2100FFFF: True,
        0x220000FF: True,
        0x02123456: True,
        0x207FFFFF: True,
    }
    rejected = [0x21010000, 0x22000100, 0x00123456, 0x20800000]
    for bits in accepted:
        try:
            decode_compact(bits)
        except ValueError as exc:
            errors.append(f"compact {bits:#010x} should decode: {exc}")
    for bits in rejected:
        try:
            decode_compact(bits)
        except ValueError:
            pass
        else:
            errors.append(f"compact {bits:#010x} should be rejected")
    try:
        require(
            decode_compact(0x02123456) == 0x1234,
            "compact 0x02123456 should decode to 0x1234",
        )
        require(
            decode_compact(0x207FFFFF) == (0x7FFFFF << 232),
            "compact 0x207fffff should decode to the pow limit",
        )
    except ValueError as exc:
        errors.append(f"compact vector decode failed: {exc}")

    reference = (1 << 255) - 1
    t_overlay = decode_compact(0x200FFFFF)
    t_powlimit = decode_compact(0x207FFFFF)
    unit = Fraction(1 << 256, reference + 1)
    require(unit == 2, "E(R) must be 2 for R = 2^255 - 1")
    require(reference >= t_powlimit, "R must be >= the regtest pow limit")

    k = 255
    require(reference + 1 == 1 << k, "R+1 must be an exact power of two")
    require((reference + 1) & reference == 0, "power-of-two check failed")

    family = {1: 2, 2: 4, 4: 8, 8: 16, 16: 32}
    for multiplier, expected_work in family.items():
        require(
            multiplier > 0 and multiplier & (multiplier - 1) == 0,
            f"m={multiplier} must be a power of two",
        )
        require(multiplier <= 1 << (k - 1), f"m={multiplier} must have j <= k-1")
        target = (reference + 1) // multiplier - 1
        require(target >= 1, f"m={multiplier} produced a zero target")
        require(
            Fraction(1 << 256, target + 1) == expected_work,
            f"m={multiplier} expected work must be {expected_work}",
        )

    require((1 << 254) + 1 != (1 << 254), "non-power vector setup failed")
    require(
        ((1 << 254) + 1) & (1 << 254) != 0,
        "R=2^254 should have a non-power R+1",
    )
    require(3 & (3 - 1) != 0, "m=3 should be non-power")

    def ceil_log2(n: int) -> int:
        return (n - 1).bit_length()

    def issuance(j_cfg: int, target: int) -> int:
        return min(j_cfg, k - ceil_log2(target + 1))

    require(issuance(3, t_overlay) == 3, "j_cfg=3 should be feasible on 0x200fffff")
    require(issuance(3, t_powlimit) == 0, "j_cfg=3 should step down to 0 at pow limit")
    require(issuance(3, (1 << 252) - 1) == 3, "exact-power boundary should keep j=3")
    require(issuance(3, 1 << 252) == 2, "just above exact power should step to j=2")

    q = ((8 * (reference + 1)) + t_overlay) // (t_overlay + 1)
    require(q == 65, f"Q(0x200fffff) must be 65, got {q}")
    require(
        (8 * (reference + 1)) // (t_overlay + 1) == 64,
        "floor Q(0x200fffff) must be 64",
    )
    error = q * unit - 8 * Fraction(1 << 256, t_overlay + 1)
    require(0 <= error < unit, f"ceiling error must be in [0, E(R)), got {error}")

    q_powlimit = ((8 * (reference + 1)) + t_powlimit) // (t_powlimit + 1)
    require(q_powlimit == 9, f"Q(0x207fffff) must be 9, got {q_powlimit}")
    require(
        (8 * (reference + 1)) // (t_powlimit + 1) == 8,
        "floor Q(0x207fffff) must be 8",
    )

    fingerprint = hashlib.sha256(
        b"spc-lab/work-unit/v2" + b"\x00" + reference.to_bytes(32, "big")
    ).hexdigest()
    require(
        fingerprint
        == "7fe9ed1a4e2d4279b302cbb0bcf709758f03dc44db6dab82ac40c04eb0d77c6b",
        f"unexpected fingerprint {fingerprint}",
    )
    old_reference = (1 << 254) - 1
    old_fingerprint = hashlib.sha256(
        b"spc-lab/work-unit/v2" + b"\x00" + old_reference.to_bytes(32, "big")
    ).hexdigest()
    require(
        old_fingerprint
        == "2fc4270b351c943cc49909e369f72df47e38ed48ee0f69bdf0bc1b2cb76a1859",
        f"unexpected old fingerprint {old_fingerprint}",
    )

    # Sufficient per-slice/current-limit widths used by the decision records.
    require(65000 * (1 << 32) < (1 << 48), "slice sum(d) bound failed")
    require(65000 * (1 << 32) * (1 << 51) < (1 << 99), "slice sum(d*f) bound failed")
    require((1 << 64) + (1 << 48) < (1 << 65), "D_win bound failed")
    require((1 << 51) * (1 << 65) < (1 << 128), "subsidy product bound failed")

    # Anchors that keep the prose tied to the checked vectors.
    decision_one = (ROOT / "docs/decisions/2026-11-05-lab-share-work-units.md").read_text(
        encoding="utf-8"
    )
    decision_two = (
        ROOT / "docs/decisions/2026-11-05-window-network-block-work.md"
    ).read_text(encoding="utf-8")
    for needle in (
        "R = 2^255 - 1",
        "reference_below_pow_limit",
        "reference_not_power_of_two",
        "multiplier_not_in_family",
        "ceil_log2",
        "multiplier_stepdown",
        "7fe9ed1a4e2d4279b302cbb0bcf709758f03dc44db6dab82ac40c04eb0d77c6b",
        "slice_fee_work_overflow",
    ):
        require(needle in decision_one, f"work-unit decision missing anchor {needle!r}")
    for needle in ("R = 2^255 - 1", "Q(T)", "underfilled", "window_rule"):
        require(needle in decision_two, f"window decision missing anchor {needle!r}")

    return errors


def main() -> int:
    errors: list[str] = []
    expected: list[str] = []
    for path in FILES:
        if not path.exists():
            errors.append(f"missing file {path}")
            continue
        errors.extend(check_ascii(path))
        link_errors, link_expected = check_links(path)
        errors.extend(link_errors)
        expected.extend(link_expected)
    errors.extend(check_vectors())

    for line in expected:
        print(f"EXPECTED: {line}")
    for line in errors:
        print(f"ERROR: {line}")
    if errors:
        print(f"FAIL: {len(errors)} error(s)")
        return 1
    print(f"OK: {len(FILES)} files, ASCII clean, links resolve, vectors pass")
    return 0


if __name__ == "__main__":
    sys.exit(main())
