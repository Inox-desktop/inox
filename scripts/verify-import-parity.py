#!/usr/bin/env python3
"""Check that nothing in the component repositories is missing from this workspace.

Migration tooling for the single-repository workspace, not a product or CI check. Run it before
archiving the component repositories. Once they are archived this file has no
purpose and should be deleted with them.

It answers one question: *is anything left in the component repositories that
this workspace does not have, and is everything here safely committed?*

For every source file it compares the workspace copy against two things:

  A. the component repository's committed ref  (`--ref`, default `main`)
  B. this repository's committed ref            (`--committed-ref`, default HEAD)

and classifies exactly:

  MISSING A has it, the workspace does not     -> real loss, stop
  UNCOMMITTED HERE differs from A and is not in B       -> at risk: only this
                                                             disk holds it
  SUPERSEDED HERE differs from A but is committed in B -> fine; the file was
                                                             changed on purpose
                                                             (imported uncommitted
                                                             work, or a rewrite)
  matches A committed work preserved

Pass `--committed-ref origin/main` to prove the workspace copies are pushed, not
merely committed locally: "committed on this disk" and "on GitHub" are different
claims, and conflating them is the mistake this tool exists to catch.

An earlier version used the local clones under `repos/` as the second source.
That worked only while the clones existed and it classified by "matches the
clone's working tree", which is not the property that matters. `repos/` is
optional here; when present its uncommitted state is reported as a diagnostic.

Not compared, and listed as dropped so the omissions stay visible: `LICENSE`,
`.gitignore`, `deny.toml`, `rust-toolchain.toml`, `Cargo.lock` (the single-repository workspace kept one
workspace copy each), `Cargo.toml` / `AGENTS.md` / `README.md` / `CLAUDE.md`
(rewritten for workspace inheritance), `.github/`.

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


def gh(path):
    return json.loads(subprocess.check_output(["gh", "api", path]))


def sha_of_file(path):
    if not os.path.isfile(path):
        return None
    return subprocess.check_output(["git", "hash-object", path]).decode().strip()


def sha_in_ref(root, ref, path):
    """Blob hash of `path` at `ref`, or None when absent / unknown ref."""
    try:
        return subprocess.check_output(
            ["git", "-C", root, "rev-parse", f"{ref}:{path}"],
            stderr=subprocess.DEVNULL).decode().strip()
    except subprocess.CalledProcessError:
        return None


def keep(path):
    return (os.path.basename(path) not in DROPPED
            and not path.startswith(".github/"))


def main():
    ap = argparse.ArgumentParser(description="see the module docstring")
    ap.add_argument("--ref", default="main",
                    help="component-repository ref to compare against")
    ap.add_argument("--committed-ref", default="HEAD",
                    help="this repository's ref to prove the copies are committed")
    ap.add_argument("--org", default="Inox-desktop")
    args = ap.parse_args()

    root = subprocess.check_output(
        ["git", "rev-parse", "--show-toplevel"]).decode().strip()

    if sha_in_ref(root, args.committed_ref, "Cargo.toml") is None:
        print(f"cannot resolve --committed-ref `{args.committed_ref}`", file=sys.stderr)
        sys.exit(2)

    lost, at_risk, superseded, unreadable = [], [], [], []
    clones_dirty = []

    for crate in CRATES:
        try:
            tree = gh(f"repos/{args.org}/{crate}/git/trees/{args.ref}?recursive=1")["tree"]
        except subprocess.CalledProcessError:
            print(f"{crate:<22} !! cannot read `{args.ref}` (archived?)")
            unreadable.append(crate)
            continue

        repo = {t["path"]: t["sha"] for t in tree
                if t["type"] == "blob" and keep(t["path"])}

        c_lost, c_risk, c_superseded, ok = [], [], [], 0
        for path, repo_sha in sorted(repo.items()):
            rel = f"crates/{crate}/{path}"
            ws_sha = sha_of_file(os.path.join(root, rel))
            if ws_sha is None:
                c_lost.append(path)
            elif ws_sha == repo_sha:
                ok += 1
            elif sha_in_ref(root, args.committed_ref, rel) == ws_sha:
                c_superseded.append(path)
            else:
                c_risk.append(path)

        verdict = "OK" if not (c_lost or c_risk) else "CHECK"
        print(f"{crate:<22} {len(repo):>3} files  {verdict}  ({ok} match `{args.ref}`)")
        for p in c_lost:
            print(f"    MISSING: repo has it, workspace does not        {p}")
        for p in c_risk:
            print(f"    UNCOMMITTED HERE: not in `{args.committed_ref}`  {p}")
        for p in c_superseded:
            print(f"    superseded here: differs from `{args.ref}`, committed in "
                  f"`{args.committed_ref}`   {p}")

        lost += [f"{crate}/{p}" for p in c_lost]
        at_risk += [f"{crate}/{p}" for p in c_risk]
        superseded += [f"{crate}/{p}" for p in c_superseded]

        clone = os.path.join(root, "repos", crate)
        if os.path.isdir(os.path.join(clone, ".git")):
            status = subprocess.check_output(
                ["git", "-C", clone, "status", "--porcelain"]).decode()
            for line in status.splitlines():
                if not line.startswith("??"):
                    clones_dirty.append(f"{crate}: {line.strip()}")

    print()
    if superseded:
        print(f"{len(superseded)} file(s) intentionally differ from `{args.ref}` and are "
              f"committed in `{args.committed_ref}`.")
        print("That is the expected shape for imported uncommitted work and for files")
        print("whose workspace form was rewritten; the full list is printed above.\n")
    if clones_dirty:
        print("Diagnostic — uncommitted changes in the optional local clones under repos/:")
        for line in clones_dirty:
            print(f"  {line}")
        print("  Not a problem by itself: check these files appear as `superseded here`")
        print("  above, which is what proves their content is committed in this repo.\n")

    if lost or at_risk or unreadable:
        print(f"RESULT: NOT SAFE — {len(lost)} missing, {len(at_risk)} uncommitted here, "
              f"{len(unreadable)} repos unreadable")
        sys.exit(1)
    print(f"RESULT: nothing in the component repositories is missing from this workspace, ")
    print(f"        and every workspace copy is committed in `{args.committed_ref}`.")
    print(f"        ({len(superseded)} files intentionally differ from `{args.ref}` — "
          f"they are not losses.)")
    sys.exit(0)


if __name__ == "__main__":
    main()
