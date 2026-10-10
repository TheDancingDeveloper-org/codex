"""Selection semantics for fork-ci's changed-package scripts."""

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import importlib.util


def load(name, filename):
    spec = importlib.util.spec_from_file_location(
        name, os.path.join(os.path.dirname(os.path.abspath(__file__)), filename)
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


fork_changed_packages = load("fork_changed_packages", "fork-changed-packages.py")
fork_lock_affected = load("fork_lock_affected", "fork-lock-affected.py")


def package(name, directory, deps):
    return {
        "name": name,
        "id": f"{directory}-id",
        "manifest_path": os.path.join("/work/codex-rs", directory, "Cargo.toml"),
        "dependencies": [{"pkg": dep} for dep in deps],
    }


META = {
    "workspace_members": ["core-id", "cli-id", "protocol-id"],
    "packages": [
        package("codex-core", "core", ["protocol-id"]),
        package("codex-cli", "cli", ["core-id"]),
        package("codex-protocol", "protocol", []),
    ],
}


class SelectTest(unittest.TestCase):
    def select(self, paths):
        return fork_changed_packages.select(META, paths, root="/work/codex-rs")

    def test_source_file_selects_its_package(self):
        self.assertEqual(self.select(["codex-rs/core/src/lib.rs"]), ["codex-core"])

    def test_workspace_manifest_selects_everything(self):
        self.assertEqual(
            self.select(["codex-rs/Cargo.toml"]),
            ["codex-core", "codex-cli", "codex-protocol"],
        )

    def test_member_manifest_selects_reverse_dependencies(self):
        selected = self.select(["codex-rs/protocol/Cargo.toml"])
        self.assertEqual(selected, ["codex-core", "codex-cli", "codex-protocol"])

    def test_member_manifest_without_dependents_selects_itself(self):
        self.assertEqual(self.select(["codex-rs/cli/Cargo.toml"]), ["codex-cli"])


class LockTest(unittest.TestCase):
    def test_lock_only_change_selects_the_affected_subset(self):
        base = {
            "workspace_members": ["core-id", "cli-id", "proto-id"],
            "packages": [
                {"name": "serde", "id": "serde-1", "dependencies": []},
                {"name": "codex-protocol", "id": "proto-id", "dependencies": []},
                {"name": "codex-core", "id": "core-id", "dependencies": [{"pkg": "serde-1"}, {"pkg": "proto-id"}]},
                {"name": "codex-cli", "id": "cli-id", "dependencies": [{"pkg": "core-id"}]},
            ],
        }
        head = json.loads(json.dumps(base))
        for package in head["packages"]:
            if package["id"] == "serde-1":
                package["id"] = "serde-2"
            for dep in package["dependencies"]:
                if dep["pkg"] == "serde-1":
                    dep["pkg"] = "serde-2"
        self.assertEqual(fork_lock_affected.affected(base, head), ["codex-core", "codex-cli"])


if __name__ == "__main__":
    unittest.main()
