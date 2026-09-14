#!/usr/bin/env python3
"""Check the mechanically checkable rules in docs/agents/voice.md.

Most of that file's rules are about judgement and cannot be automated. Two are
not, and both are ones it records as slipping in practice: em dashes, and
references to this repo's own issue numbers from source.

Usage:
  tools/voice/check.py           # check tracked files, exit 1 on any finding
  tools/voice/check.py --fix-em-dashes
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

EM_DASH = "—"

# Files that may legitimately contain either pattern. `voice.md` itself has to
# quote the characters it bans in order to ban them, and this checker has to
# contain them in order to look for them.
EXEMPT = {
    "docs/agents/voice.md",
    "tools/voice/check.py",
}

# Binary and generated files, where a match means nothing.
SKIP_SUFFIXES = {".lock", ".png", ".jpg", ".gif", ".ico", ".pdf", ".bin"}

# `rust-lang/rust#143874`: a qualified reference to another project's tracker.
# Stable, unambiguous, and resolvable without access to this repo, which is the
# opposite of the problem the rule exists for.
QUALIFIED_REF = re.compile(r"[\w.-]+/[\w.-]+#\d+")

# `#113`, `(#54)`, `see #26`.
BARE_REF = re.compile(r"#\d+")

# Inline code spans. Chess writes mate-in-N as `#N`, and this codebase quotes
# engine output that way, so a backticked `#110` is notation rather than a
# tracker reference.
CODE_SPAN = re.compile(r"`[^`\n]*`")


def tracked_files() -> list[Path]:
    out = subprocess.run(
        ["git", "ls-files", "-z"], cwd=REPO, capture_output=True, text=True, check=True
    ).stdout
    return [REPO / p for p in out.split("\0") if p]


def is_exempt(path: Path) -> bool:
    rel = path.relative_to(REPO).as_posix()
    return rel in EXEMPT or path.suffix in SKIP_SUFFIXES


def read_lines(path: Path) -> list[str] | None:
    try:
        return path.read_text(encoding="utf-8").split("\n")
    except (UnicodeDecodeError, FileNotFoundError, IsADirectoryError):
        return None


def check_em_dashes(path: Path, lines: list[str]) -> list[tuple[int, str, str]]:
    return [
        (
            n,
            line.strip(),
            "em dash; use a colon, semicolon, comma, parentheses, or restructure",
        )
        for n, line in enumerate(lines, 1)
        if EM_DASH in line
    ]


def check_issue_refs(path: Path, lines: list[str]) -> list[tuple[int, str, str]]:
    # Scoped to Rust source: the rule is about references rotting where a reader
    # has only the code in front of them. Commit messages, PR bodies and the ADRs
    # under `docs/` are where a tracker reference belongs and stays useful.
    if path.suffix != ".rs":
        return []

    findings = []
    for n, line in enumerate(lines, 1):
        stripped = QUALIFIED_REF.sub("", CODE_SPAN.sub("", line))
        if BARE_REF.search(stripped):
            findings.append(
                (
                    n,
                    line.strip(),
                    "issue or PR number; state the reasoning inline instead",
                )
            )
    return findings


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--fix-em-dashes",
        action="store_true",
        help="rewrite em dashes to ', ' and report what changed; review the result",
    )
    args = parser.parse_args()

    findings: list[tuple[Path, int, str, str]] = []
    fixed = 0

    for path in tracked_files():
        if is_exempt(path):
            continue
        lines = read_lines(path)
        if lines is None:
            continue

        if args.fix_em_dashes and any(EM_DASH in line for line in lines):
            path.write_text("\n".join(lines).replace(EM_DASH, ", "), encoding="utf-8")
            fixed += 1
            lines = read_lines(path) or []

        for n, text, why in check_em_dashes(path, lines) + check_issue_refs(
            path, lines
        ):
            findings.append((path.relative_to(REPO), n, text, why))

    if args.fix_em_dashes:
        print(
            f"rewrote em dashes in {fixed} file(s); review the wording, a comma is not always right"
        )

    for rel, n, text, why in findings:
        print(f"{rel}:{n}: {why}\n    {text}")

    if findings:
        print(f"\n{len(findings)} finding(s). See docs/agents/voice.md.")
        return 1

    print("voice: no findings")
    return 0


if __name__ == "__main__":
    sys.exit(main())
