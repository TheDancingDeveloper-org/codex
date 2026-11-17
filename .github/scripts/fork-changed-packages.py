"""Map changed file paths to their cargo package names.

Reads `cargo metadata --no-deps --format-version 1` on stdin and file paths,
one per line, on argv. A file belongs to the package whose manifest directory
is the longest prefix of its path, so a nested crate wins over its parent and
the directory name never has to equal the package name.

A member's Cargo.toml is a file of that member like any other: it selects the
member and nothing else. It used to select the member's reverse dependencies
too, but a fork-owned manifest hunk is a dependency the fork's patch added
(codex-home gained dirs and sha2), and codex-home's dependents are the whole
workspace, which is what the selection exists to avoid (WI-4161). Dependents
compile against the same API either way; a fork patch that changes an API is a
change to .rs files, and dependents the fork itself edits are selected by
those edits. A change to the workspace Cargo.toml selects every package.
Cargo.lock is not handled here: the caller compares the resolve graph and
passes the affected members as ordinary paths, or prints them itself.
"""

import json
import os
import sys


def strip_workspace_prefix(path):
    path = os.path.normpath(path)
    prefix = "codex-rs" + os.sep
    marker = os.sep + "codex-rs" + os.sep
    if path == "codex-rs" or path.endswith(os.sep + "codex-rs"):
        return ""
    if marker in path:
        path = path.split(marker, 1)[1]
    if path.startswith(prefix):
        return path[len(prefix):]
    return path


def package_dirs(meta, root):
    dirs = []
    for package in meta["packages"]:
        manifest = package["manifest_path"]
        if os.path.isabs(manifest):
            manifest = os.path.relpath(manifest, root)
        manifest = strip_workspace_prefix(manifest)
        dirs.append((os.path.dirname(manifest), package["name"]))
    dirs.sort(key=lambda entry: len(entry[0]), reverse=True)
    return dirs


def owner_of(path, dirs):
    path = strip_workspace_prefix(path)
    for directory, name in dirs:
        if path == directory or path.startswith(directory + os.sep):
            return name
    return None



def select(meta, paths, root=None):
    """Return workspace package names selected by `paths`, in first-seen order.

    `paths` are repo-relative. The workspace Cargo.toml selects everything.
    Any other file, a member's Cargo.toml included, selects only its owning
    package.
    """
    root = root or os.getcwd()
    dirs = package_dirs(meta, root)
    member_names = []
    for package in meta["packages"]:
        if package["id"] in set(meta.get("workspace_members", [])):
            member_names.append(package["name"])
    seen = []

    def add(name):
        if name and name not in seen:
            seen.append(name)

    for argument in paths:
        path = os.path.normpath(argument)
        stripped = strip_workspace_prefix(path)
        if stripped in ("Cargo.toml", "./Cargo.toml"):
            return member_names
        add(owner_of(path, dirs))
    return seen


def main(argv):
    meta = json.load(sys.stdin)
    print("\n".join(select(meta, argv[1:])))


if __name__ == "__main__":
    main(sys.argv)
