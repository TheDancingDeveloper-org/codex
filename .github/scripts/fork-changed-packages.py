"""Map changed file paths to their cargo package names.

Reads `cargo metadata --no-deps --format-version 1` on stdin and file paths,
one per line, on argv. A file belongs to the package whose manifest directory
is the longest prefix of its path, so a nested crate wins over its parent and
the directory name never has to equal the package name.
"""

import json
import os
import sys

meta = json.load(sys.stdin)
root = os.getcwd()
dirs = []
for package in meta["packages"]:
    manifest = os.path.relpath(package["manifest_path"], root)
    dirs.append((os.path.dirname(manifest), package["name"]))
dirs.sort(key=lambda entry: len(entry[0]), reverse=True)

seen = []
for argument in sys.argv[1:]:
    path = os.path.normpath(argument.removeprefix("codex-rs/"))
    for directory, name in dirs:
        if path == directory or path.startswith(directory + os.sep):
            if name not in seen:
                seen.append(name)
            break
print("\n".join(seen))
