"""Map changed file paths to their cargo package names.

Reads `cargo metadata --no-deps --format-version 1` on stdin and file paths,
one per line, on argv. A file belongs to the package whose manifest directory
is the longest prefix of its path, so a nested crate wins over its parent and
the directory name never has to equal the package name.

A change to a member's Cargo.toml also selects the workspace packages that
depend on it, directly or transitively (reverse dependencies). A change to the
workspace Cargo.toml selects every package. Cargo.lock is not handled here:
the caller compares the resolve graph and passes the affected members as
ordinary paths, or prints them itself.
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


def reverse_closure(meta, names):
    """Workspace members that depend on `names`, directly or transitively.

    The selection step runs `cargo metadata --no-deps`, whose `resolve` is null,
    so the only edges available are the declared requirements. A declared
    dependency names a workspace member when it has a `path` or its `name` is
    itself a member; registry requirements match neither.
    """
    members = set(meta.get("workspace_members", []))
    member_names = {
        package["name"]
        for package in meta["packages"]
        if package["id"] in members
    }
    # name -> packages in the workspace that list it as a normal/dev/build dep
    dependents = {}
    for package in meta["packages"]:
        if package["id"] not in members:
            continue
        for dep in package.get("dependencies", []):
            dep_name = dep.get("name")
            if dep.get("path") or dep_name in member_names:
                dependents.setdefault(dep_name, set()).add(package["name"])
    selected = set(names)
    frontier = list(names)
    while frontier:
        name = frontier.pop()
        for dependent in dependents.get(name, ()):
            if dependent not in selected:
                selected.add(dependent)
                frontier.append(dependent)
    return selected


def select(meta, paths, root=None):
    """Return workspace package names selected by `paths`, in first-seen order.

    `paths` are repo-relative. The workspace Cargo.toml selects everything.
    A member Cargo.toml selects that member and its reverse dependencies.
    Any other file selects only its owning package.
    """
    root = root or os.getcwd()
    dirs = package_dirs(meta, root)
    member_names = []
    for package in meta["packages"]:
        if package["id"] in set(meta.get("workspace_members", [])):
            member_names.append(package["name"])
    manifest_dir = {}
    for package in meta["packages"]:
        manifest = package["manifest_path"]
        if os.path.isabs(manifest):
            manifest = os.path.relpath(manifest, root)
        manifest = strip_workspace_prefix(manifest)
        manifest_dir[package["name"]] = os.path.dirname(manifest)

    seen = []

    def add(name):
        if name and name not in seen:
            seen.append(name)

    manifest_hits = []
    for argument in paths:
        path = os.path.normpath(argument)
        stripped = strip_workspace_prefix(path)
        if stripped in ("Cargo.toml", "./Cargo.toml"):
            return member_names
        owner = owner_of(path, dirs)
        if owner is None:
            continue
        if os.path.basename(stripped) == "Cargo.toml" and manifest_dir.get(owner) == os.path.dirname(stripped):
            manifest_hits.append(owner)
        else:
            add(owner)
    if manifest_hits:
        closure = reverse_closure(meta, manifest_hits)
        for name in member_names:
            if name in closure:
                add(name)
        for name in manifest_hits:
            add(name)
    return seen


def main(argv):
    meta = json.load(sys.stdin)
    print("\n".join(select(meta, argv[1:])))


if __name__ == "__main__":
    main(sys.argv)
