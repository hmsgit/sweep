#!/usr/bin/env python3
"""Generate release notes for a tag from the commits since the last
release of equal-or-higher maturity.

Maturity ranks: beta < rc < stable. A beta release diffs against the
previous release of any kind; an rc folds in all betas since the last
rc/stable; a stable folds in every pre-release since the last stable.

Output: a Summary section bulleting every commit subject, then a
Details section with each commit's body under its subject. `Release
x.y.z` bump commits and merge commits are skipped.

Usage: release_notes.py <tag>       # e.g. release_notes.py v0.1.0b2
"""

from __future__ import annotations

import re
import subprocess
import sys

TAG = re.compile(r"^v(\d+)\.(\d+)\.(\d+)(?:(b|rc)(\d+))?$")
RANKS = {"b": 0, "rc": 1, None: 2}
SKIP_SUBJECT = re.compile(r"^(Release \S+$|Merge )")


def tag_key(tag: str) -> tuple[int, int, int, int, int] | None:
    m = TAG.match(tag)
    if not m:
        return None
    major, minor, patch, pre, n = m.groups()
    return (int(major), int(minor), int(patch), RANKS[pre], int(n or 0))


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], check=True, capture_output=True, text=True
    ).stdout


def baseline_for(tag: str) -> str | None:
    current = tag_key(tag)
    if current is None:
        sys.exit(f"error: unrecognized tag {tag!r} (expected vX.Y.Z[bN|rcN])")
    rank = current[3]

    candidates = []
    for other in git("tag", "--list", "v*").split():
        key = tag_key(other)
        if key is None or other == tag:
            continue
        if key < current and key[3] >= rank:
            candidates.append((key, other))
    if not candidates:
        return None
    return max(candidates)[1]


def commits(baseline: str | None, tag: str) -> list[tuple[str, str]]:
    span = f"{baseline}..{tag}" if baseline else tag
    raw = git("log", "--reverse", "--format=%s%x1f%b%x00", span)
    result = []
    for record in raw.split("\x00"):
        record = record.strip("\n")
        if not record:
            continue
        subject, _, body = record.partition("\x1f")
        subject = subject.strip()
        if not subject or SKIP_SUBJECT.match(subject):
            continue
        result.append((subject, body.strip()))
    return result


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    tag = sys.argv[1]
    baseline = baseline_for(tag)
    entries = commits(baseline, tag)

    lines = []
    if baseline:
        lines.append(f"Changes since {baseline}:")
        lines.append("")
    lines.append("## Summary")
    lines.append("")
    for subject, _ in entries:
        lines.append(f"- {subject}")

    detailed = [(s, b) for s, b in entries if b]
    if detailed:
        lines.append("")
        lines.append("## Details")
        for subject, body in detailed:
            lines.append("")
            lines.append(f"### {subject}")
            lines.append("")
            lines.append(body)

    print("\n".join(lines))


if __name__ == "__main__":
    main()
