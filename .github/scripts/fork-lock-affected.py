"""Workspace packages whose resolved dependency set changed in Cargo.lock.

Compares two `cargo metadata --format-version 1` documents (the full resolve
graph, not `--no-deps`). A workspace member is affected when the set of
package ids reachable from it through normal, dev and build dependencies
differs between the two resolves. Prints one package name per line.

Usage: fork-lock-affected.py BASE.json HEAD.json
"""

import json
import sys


def reachable(meta):
    """name -> frozenset of resolved package ids reachable from that member."""
    by_id = {package["id"]: package for package in meta["packages"]}
    members = [
        (package["id"], package["name"])
        for package in meta["packages"]
        if package["id"] in set(meta.get("workspace_members", []))
    ]
    cache = {}

    def walk(package_id, stack):
        if package_id in cache:
            return cache[package_id]
        if package_id in stack or package_id not in by_id:
            return frozenset()
        stack.add(package_id)
        found = {package_id}
        for dep in by_id[package_id].get("dependencies", []):
            dep_id = dep.get("pkg") or dep.get("id")
            if dep_id:
                found |= walk(dep_id, stack)
        stack.remove(package_id)
        cache[package_id] = frozenset(found)
        return cache[package_id]

    return {name: walk(package_id, set()) for package_id, name in members}


def affected(base, head):
    base_sets = reachable(base)
    head_sets = reachable(head)
    names = []
    for name in head_sets:
        if base_sets.get(name) != head_sets[name]:
            names.append(name)
    for name in base_sets:
        if name not in head_sets and name not in names:
            names.append(name)
    return names


def main(argv):
    with open(argv[1], encoding="utf-8") as handle:
        base = json.load(handle)
    with open(argv[2], encoding="utf-8") as handle:
        head = json.load(handle)
    print("\n".join(affected(base, head)))


if __name__ == "__main__":
    main(sys.argv)
