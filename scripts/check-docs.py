#!/usr/bin/env python3
"""Check that the documentation site's structure and links resolve.

`mdbook build` alone is a weak gate: it succeeds when a chapter listed in
SUMMARY.md does not exist (it emits a placeholder page with the chapter title and
exits 0), and it says nothing about relative links written in prose. Both classes
of breakage therefore reach the published site as 404s. This script makes them
CI failures instead.

Three checks, each of which has caught a real defect in this repository:

  1. **SUMMARY.md structure** — every chapter file a summary entry names exists,
     and every Markdown file under `src/` is reachable from the summary. A chapter
     that exists but is unlisted is invisible on the site; a listed chapter that
     does not exist is a 404 with a title.

  2. **Relative links in prose** — every `[text](target)` whose target is a
     relative path resolves to a file or directory. Links starting with a scheme
     (`https:`, `mailto:`) or with `#` are skipped: checking those needs the
     network, and a link checker that cannot run offline is not a gate.

  3. **Files the pages link to outside the book** — the docs point at crate source,
     acceptance specs and packaging scripts by relative path. Those are checked by
     (2) as well, because the target is resolved from the page's own directory.

It exits non-zero and prints every finding, not just the first: a broken link
usually comes in a batch with its siblings.

Usage:
    scripts/check-docs.py [--book DIR]   # default: docs/book
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# `[text](target)` and `[text](<target>)`, but not `![image](...)` handling: an
# image link is a link, and a missing image is just as broken as a missing page.
LINK = re.compile(r"(?<!\!)\[[^\]]*\]\((?:<([^>]+)>|([^)\s]+))(?:\s+\"[^\"]*\")?\)")
# `![alt](src)` — the same resolution rules, reported separately so the message
# says "image" rather than "link".
IMAGE = re.compile(r"!\[[^\]]*\]\((?:<([^>]+)>|([^)\s]+))(?:\s+\"[^\"]*\")?\)")
# A summary entry: `- [Title](./chapter.md)` or `- [Title](chapter.md)`, with or
# without a part heading block. Nested entries are matched too.
SUMMARY_ENTRY = re.compile(r"^\s*[-*]\s*\[[^\]]*\]\(([^)]+)\)\s*$")
# Any scheme, including `https:`, `mailto:` and `file:`. A target starting with
# `#` is an anchor in the current page.
EXTERNAL = re.compile(r"^[a-zA-Z][a-zA-Z0-9+.-]*:")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--book",
        default="docs/book",
        help="the mdBook directory (the one with book.toml) [default: docs/book]",
    )
    return parser.parse_args()


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def summary_entries(book: Path) -> list[tuple[str, Path]]:
    """Return (raw target, resolved path) for every chapter in SUMMARY.md."""
    summary = book / "src" / "SUMMARY.md"
    if not summary.is_file():
        raise SystemExit(f"no {summary}: this is not an mdBook directory")
    entries = []
    for line in read(summary).splitlines():
        match = SUMMARY_ENTRY.match(line)
        if not match:
            continue
        target = match.group(1)
        if EXTERNAL.match(target) or target.startswith("#"):
            continue
        entries.append((target, (summary.parent / target).resolve()))
    return entries


def markdown_files(root: Path) -> list[Path]:
    return sorted(root.rglob("*.md"))


def check_summary(book: Path, problems: list[str]) -> None:
    """Every summary entry exists, and every chapter is in the summary."""
    src = book / "src"
    entries = summary_entries(book)

    listed: set[Path] = set()
    for target, resolved in entries:
        if not resolved.is_file():
            problems.append(f"SUMMARY.md lists a chapter that does not exist: {target}")
            continue
        listed.add(resolved)

    for path in markdown_files(src):
        if path.name == "SUMMARY.md":
            continue
        if path.resolve() not in listed:
            problems.append(
                f"a chapter exists but no SUMMARY.md entry reaches it: "
                f"{path.relative_to(book.parent)}"
            )


def documentation_roots(book: Path) -> list[Path]:
    """The directories whose Markdown is part of the published documentation.

    Scoped deliberately: the book's own `src/`, plus the sibling `docs/` tree when
    the book lives in one (acceptance specs and quality records are linked from the
    site and published on GitHub, so a broken link there is still a broken link).
    Scanning further would drag in vendored READMEs — a checker that fails on
    `node_modules` is a checker somebody disables.
    """
    roots = [book / "src"]
    parent = book.parent
    if parent.name == "docs" and parent.is_dir():
        roots.append(parent)
    return roots


def check_links(book: Path, problems: list[str]) -> None:
    """Resolve every relative link and image target from the page's directory."""
    seen: set[Path] = set()
    for root in documentation_roots(book):
        for path in markdown_files(root):
            if path in seen:
                continue
            seen.add(path)
            # SUMMARY.md entries are checked by check_summary, which reports a
            # missing chapter once. Running the generic link check over it too
            # would report every such chapter twice.
            if path.name == "SUMMARY.md":
                continue
            text = read(path)
            for pattern, kind in ((LINK, "link"), (IMAGE, "image")):
                for match in pattern.finditer(text):
                    target = match.group(1) or match.group(2) or ""
                    target = target.strip()
                    if not target or EXTERNAL.match(target) or target.startswith("#"):
                        continue
                    # An anchor attached to a path is not part of the path.
                    path_part = target.split("#", 1)[0]
                    if not path_part:
                        continue
                    resolved = (path.parent / path_part).resolve()
                    if not resolved.exists():
                        problems.append(
                            f"{path.relative_to(book.parent.parent)}: {kind} target "
                            f"does not exist: {target}"
                        )


def main() -> int:
    args = parse_args()
    book = Path(args.book).resolve()
    problems: list[str] = []

    check_summary(book, problems)
    check_links(book, problems)

    if problems:
        print(f"check-docs: {len(problems)} problem(s)", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1

    entries = len(summary_entries(book))
    pages = len(markdown_files(book / "src"))
    print(f"check-docs: ok ({entries} chapters in the summary, {pages} pages)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
