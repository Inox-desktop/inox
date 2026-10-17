#!/usr/bin/env python3
"""Check whether any component-repository source is missing from this workspace.

Migration tooling for ADR 0016, not a product or CI check. Run it before
archiving the component repositories. Once they are archived this file has no
purpose and should be deleted with them.

Why it exists: this migration's first import read the local clones under
`repos/`, and both failure modes bit.

  1. Five clones were stale. `inox-compositor` was six commits behind, so a
     *merged* change (the compositor consuming `inox-protocols`, plus its
     `ipc_global` test) was silently dropped.
  2. `repos/inox-compositor` had an uncommitted working tree — the M2.5 backend
     scaffold, 231 changed lines plus an untracked `src/backend/`. That work was
     never on GitHub at all, so "the repositories can be deleted" was never the
     question for it.

So for every source file this compares the workspace copy against **two**
sources — the repository's committed ref and the local clone's working tree —
and classifies by exact content equality:

  matches repo ref        committed work preserved
  matches clone working    imported from an uncommitted tree; exists only on
    tree (repo differs)    this disk. Push it, or decide to discard it.
  matches neither          merged by hand from both (review the file)
  in repo, not workspace   real loss — do not archive yet
  in clone, not repo       untracked in the clone; exists only on this disk

Committed refs and clones are both checked because either can be the only
holder: the clone can be behind (problem 1) or ahead of it (problem 2).

Byte comparison uses `git hash-object`, so `rustfmt` reflowing counts as a
difference. That is deliberate: a file the workspace silently reformatted is a
file whose diff against the repo is not reviewable.

Not compared, and listed as dropped so the omissions stay visible: `LICENSE`,
`.gitignore`, `deny.toml`, `rust-toolchain.toml`, `Cargo.lock` (ADR 0016 kept
one workspace copy each), `Cargo.toml` / `AGENTS.md` / `README.md` /
`CLAUDE.md` (rewritten for workspace inheritance), `.github/`.

Requires an authenticated `gh` CLI and network access.
"""

import argparse
import json
import os
import subprocess
import sys

DROPPED = {"LICENSE", ".gitignore", "deny.toml", "rust-toolchain.toml",
           "Cargo.lock", "Cargo.toml", "AGENTS.md", "README.md", "CLAUDE.md"}

CRATES = ["inox-common", "inox-config", "inox-protocols", "inox-compositor",
          "inox-xwayland", "inox-shell", "inox-panel", "inox-launcher",
          "inox-notifications", "inox-settings", "inox-lock", "inox-wallpaper",
          "inox-session", "inox-idle", "inox-greet", "inox-integration"]

# Versions of these files that the workspace deliberately differs from.
MERGED_BY_HAND = "merged-by-hand"


def gh(path):
    return json.loads(subprocess.check_output(["gh", "api", path]))


def blob_sha(path):
    if not os.path.isfile(path):
        return None
    return subprocess.check_output(["git", "hash-object", path]).decode().strip()


def clone_files(clone):
    """Tracked + untracked files in the clone's working tree, relative paths."""
    if not os.path.isdir(os.path.join(clone, ".git")):
        return None
    out = subprocess.check_output(
        ["git", "-C", clone, "ls-files", "--cached", "--others", "--exclude-standard"])
    return set(out.decode().split("\n")) - {""}


def main():
    ap = argparse.ArgumentParser(description="see the module docstring")
    ap.add_argument("--ref", default="main", help="committed ref to compare against")
    ap.add_argument("--org", default="Inox-desktop")
    args = ap.parse_args()

    root = subprocess.check_output(
        ["git", "rev-parse", "--show-toplevel"]).decode().strip()

    lost, uncommitted_only, merged, unreadable, clone_only = [], [], [], [], []

    for crate in CRATES:
        try:
            tree = gh(f"repos/{args.org}/{crate}/git/trees/{args.ref}?recursive=1")["tree"]
        except subprocess.CalledProcessError:
            print(f"{crate:<22} !! cannot read `{args.ref}` (archived?)")
            unreadable.append(crate)
            continue

        def keep(p):
            return (os.path.basename(p) not in DROPPED
                    and not p.startswith(".github/"))

        repo = {t["path"]: t["sha"] for t in tree
                if t["type"] == "blob" and keep(t["path"])}
        clone = clone_files(os.path.join(root, "repos", crate))
        paths = sorted(set(repo) | (clone or set()))

        ok, c_lost, c_uncommitted, c_merged, c_clone_only = 0, [], [], [], []
        for path in paths:
            if not keep(path):
                continue
            ws = blob_sha(os.path.join(root, "crates", crate, path))
            repo_sha = repo.get(path)
            clone_sha = blob_sha(os.path.join(root, "repos", crate, path)) if clone else None

            if ws is None:
                (c_clone_only if repo_sha is None else c_lost).append(path)
            elif repo_sha is not None and ws == repo_sha:
                ok += 1
            elif clone_sha is not None and ws == clone_sha:
                c_uncommitted.append(path)
            elif repo_sha is None and clone_sha is None:
                c_merged.append(path)
            else:
                c_merged.append(path)

        verdict = "OK" if not (c_lost or c_merged) else "CHECK"
        print(f"{crate:<22} {len(paths):>3} paths  {verdict}  ({ok} match ref)")
        for p in c_lost:
            print(f"    LOST: repo has it, workspace does not   {p}")
        for p in c_uncommitted:
            print(f"    UNCOMMITTED-ONLY: workspace == clone working tree, "
                  f"not on GitHub   {p}")
        for p in c_merged:
            print(f"    REVIEW: matches neither ref nor clone   {p}")
        for p in c_clone_only:
            print(f"    CLONE-ONLY: untracked in clone          {p}")

        lost += [f"{crate}/{p}" for p in c_lost]
        uncommitted_only += [f"{crate}/{p}" for p in c_uncommitted]
        merged += [f"{crate}/{p}" for p in c_merged]
        clone_only += [f"{crate}/{p}" for p in c_clone_only]

    print()
    if uncommitted_only:
        print("These workspace files came from a clone's uncommitted working tree.")
        print("That content is on this disk only. Push it somewhere, or decide")
        print("explicitly to discard it, before archiving:")
        for p in uncommitted_only:
            print(f"  {p}")
        print()
    if merged:
        print("These workspace files match neither their committed ref nor the clone")
        print("working tree, so they were reconciled by hand and need a human read")
        print("(the expected case is a file both sides had changed):")
        for p in merged:
            print(f"  {p}")
        print()

    if lost or unreadable:
        print(f"RESULT: NOT SAFE TO ARCHIVE — {len(lost)} files lost, "
              f"{len(unreadable)} repos unreadable")
        sys.exit(1)
    print("RESULT: every repository source file is present in the workspace, "
          "either from the committed ref or from the clone's working tree.")
    if merged:
        print(f"NOTE: {len(merged)} file(s) still need the human read above.")
    sys.exit(0)


if __name__ == "__main__":
    main()
