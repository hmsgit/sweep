#!/usr/bin/env python3
"""Version bump helper for sweep releases.

Cargo.toml is the single source of truth and must hold a SemVer version
(cargo rejects PEP 440 spellings); PyPI and git tags use the PEP 440
form, which maturin derives from SemVer automatically:

    Cargo.toml: 0.1.0-beta.1   ->   PyPI: 0.1.0b1   ->   tag: v0.1.0b1

Usage:
    bump.py [major|minor|patch] [--beta | --rc] [--git] [--dry-run]

    bump.py --beta          0.1.0        -> 0.1.0-beta.1  (pre-first-release)
    bump.py --beta          0.1.0-beta.1 -> 0.1.0-beta.2
    bump.py --rc            0.1.0-beta.2 -> 0.1.0-rc.1
    bump.py                 0.1.0-rc.1   -> 0.1.0         (finalize)
    bump.py patch           0.1.0-beta.4 -> 0.1.0         (finalize too)
    bump.py patch           0.1.0        -> 0.1.1
    bump.py minor --beta    0.1.1        -> 0.2.0-beta.1

Rewrites Cargo.toml, Cargo.lock and the README `rev:` example. With
--git it also commits and creates the annotated tag (never pushes).
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT / "Cargo.toml"
README = ROOT / "README.md"

SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-(beta|rc)\.(\d+))?$")


def parse(version: str) -> tuple[int, int, int, str | None, int]:
    m = SEMVER.match(version)
    if not m:
        sys.exit(f"error: unsupported version in Cargo.toml: {version!r} "
                 "(expected MAJOR.MINOR.PATCH[-beta.N|-rc.N])")
    major, minor, patch, pre, n = m.groups()
    return int(major), int(minor), int(patch), pre, int(n or 0)


def compute(version: str, level: str | None, pre: str | None) -> str:
    major, minor, patch, cur_pre, cur_n = parse(version)

    if level == "major":
        major, minor, patch, cur_pre = major + 1, 0, 0, None
    elif level == "minor":
        minor, patch, cur_pre = minor + 1, 0, None
    elif level == "patch":
        # A pre-release's release version IS its base: `patch` from
        # 0.1.0-beta.4 finalizes to 0.1.0 rather than skipping to
        # 0.1.1 (cargo-release / poetry convention).
        if cur_pre is None:
            patch += 1
        cur_pre = None

    base = f"{major}.{minor}.{patch}"
    if pre is None:
        if level is None and cur_pre is None:
            sys.exit(f"error: {version} is already final; pass a bump "
                     "level or --beta/--rc")
        # A level bump gives a plain final version; no level means
        # finalizing the current pre-release.
        return base

    if level is None and cur_pre == pre:
        return f"{base}-{pre}.{cur_n + 1}"
    if level is None and cur_pre == "rc" and pre == "beta":
        sys.exit("error: refusing to go back from rc to beta; "
                 "bump a level instead")
    return f"{base}-{pre}.1"


def pep440(version: str) -> str:
    major, minor, patch, pre, n = parse(version)
    base = f"{major}.{minor}.{patch}"
    if pre is None:
        return base
    return f"{base}{'b' if pre == 'beta' else 'rc'}{n}"


def current_version() -> str:
    m = re.search(r'^version = "([^"]+)"$', CARGO_TOML.read_text(), re.M)
    if not m:
        sys.exit("error: no version line found in Cargo.toml")
    return m.group(1)


def apply(old: str, new: str, tag: str) -> None:
    CARGO_TOML.write_text(
        CARGO_TOML.read_text().replace(f'version = "{old}"', f'version = "{new}"', 1)
    )
    subprocess.run(["cargo", "update", "-w", "-q"], cwd=ROOT, check=True)
    readme = README.read_text()
    updated = re.sub(r"rev: v[0-9][\w.\-]*", f"rev: {tag}", readme)
    if updated != readme:
        README.write_text(updated)


def git(new: str, tag: str) -> None:
    subprocess.run(
        ["git", "add", "Cargo.toml", "Cargo.lock", "README.md"], cwd=ROOT, check=True
    )
    subprocess.run(["git", "commit", "-m", f"Release {pep440(new)}"], cwd=ROOT, check=True)
    subprocess.run(
        ["git", "tag", "-a", tag, "-m", f"Release {pep440(new)}"], cwd=ROOT, check=True
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("level", nargs="?", choices=["major", "minor", "patch"])
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--beta", action="store_const", dest="pre", const="beta")
    group.add_argument("--rc", action="store_const", dest="pre", const="rc")
    parser.add_argument("--git", action="store_true", help="commit and tag (no push)")
    parser.add_argument("--dry-run", action="store_true", help="print only")
    args = parser.parse_args()

    old = current_version()
    new = compute(old, args.level, args.pre)
    tag = f"v{pep440(new)}"

    print(f"cargo:  {old} -> {new}")
    print(f"pypi:   {pep440(new)}")
    print(f"tag:    {tag}")
    if args.dry_run:
        return

    apply(old, new, tag)
    if args.git:
        git(new, tag)
        print(f"committed and tagged; publish with: git push --follow-tags")
    else:
        print("files updated; commit, tag and push to release")


if __name__ == "__main__":
    main()
