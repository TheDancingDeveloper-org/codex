"""Selection semantics for fork-ci's changed-package scripts.

The fixtures are real `cargo metadata --format-version 1` output from a
three-member scratch workspace (codex-protocol, codex-core, codex-cli), trimmed
to the packages the graph walk reaches. `nodeps.json` was generated with
`--no-deps` (resolve is null); `lock-base.json` and `lock-head.json` are full
resolves whose only difference is the pinned serde version.
"""

import json
import os
import subprocess
import sys
import tempfile
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


HERE = os.path.dirname(os.path.abspath(__file__))
fork_changed_packages = load("fork_changed_packages", "fork-changed-packages.py")
fork_lock_affected = load("fork_lock_affected", "fork-lock-affected.py")


def fixture(name):
    with open(os.path.join(HERE, "testdata", name), encoding="utf-8") as handle:
        return json.load(handle)


NODEPS = fixture("nodeps.json")
ROOT = NODEPS["workspace_root"]


class SelectTest(unittest.TestCase):
    def select(self, paths):
        return fork_changed_packages.select(NODEPS, paths, root=ROOT)

    def test_source_file_selects_its_package(self):
        self.assertEqual(self.select(["codex-rs/core/src/lib.rs"]), ["codex-core"])

    def test_workspace_manifest_selects_everything(self):
        self.assertEqual(
            self.select(["codex-rs/Cargo.toml"]),
            ["codex-protocol", "codex-core", "codex-cli"],
        )

    def test_member_manifest_selects_reverse_dependencies(self):
        selected = self.select(["codex-rs/protocol/Cargo.toml"])
        self.assertEqual(selected, ["codex-protocol", "codex-core", "codex-cli"])

    def test_member_manifest_without_dependents_selects_itself(self):
        self.assertEqual(self.select(["codex-rs/cli/Cargo.toml"]), ["codex-cli"])


class LockTest(unittest.TestCase):
    def test_lock_only_change_selects_the_affected_subset(self):
        base = fixture("lock-base.json")
        head = fixture("lock-head.json")
        # serde 1.0.200 -> 1.0.210. codex-core depends on it directly and
        # codex-cli only through codex-core, so both reachable sets move.
        self.assertEqual(
            fork_lock_affected.affected(base, head), ["codex-cli", "codex-core"]
        )

    def test_identical_resolves_select_nothing(self):
        head = fixture("lock-head.json")
        self.assertEqual(fork_lock_affected.affected(head, head), [])


CLIPPY_GATE = r"""
select(.reason == "compiler-message")
| .message
| select(.level == "warning" or .level == "error")
| . as $m
| ($m.spans[] | select(.is_primary)) as $s
| select(any($changed[]; . == $s.file_name))
| "\($s.file_name):\($s.line_start):\($s.column_start): \($m.level): \($m.message)"
"""


class ClippyGateTest(unittest.TestCase):
    def test_nameless_package_id_in_a_changed_file_fails_the_gate(self):
        # cargo omits the name from a package id when it equals the directory's
        # last segment, so codex-home's id has no `#codex-home@` to match on.
        line = {
            "reason": "compiler-message",
            "package_id": "path+file:///work/codex-rs/codex-home#0.1.0",
            "message": {
                "level": "warning",
                "message": "this function has too many arguments",
                "spans": [
                    {
                        "file_name": "codex-home/src/lib.rs",
                        "line_start": 12,
                        "column_start": 1,
                        "is_primary": True,
                    }
                ],
            },
        }
        with tempfile.TemporaryDirectory() as directory:
            clippy = os.path.join(directory, "clippy.json")
            changed = os.path.join(directory, "changed")
            with open(clippy, "w", encoding="utf-8") as handle:
                handle.write(json.dumps(line) + "\n")
            with open(changed, "w", encoding="utf-8") as handle:
                handle.write("codex-home/src/lib.rs\n")
            # The workflow slurps `jq -R .` of the changed-file list, which
            # yields an array of JSON strings. Slurping the raw file would
            # yield one string with the newline still in it.
            quoted = os.path.join(directory, "changed.json")
            with open(quoted, "w", encoding="utf-8") as handle:
                subprocess.run(["jq", "-R", ".", changed], check=True, stdout=handle)
            result = subprocess.run(
                ["jq", "-r", "--slurpfile", "changed", quoted, CLIPPY_GATE, clippy],
                check=True, capture_output=True, text=True,
            )
        self.assertIn("codex-home/src/lib.rs:12:1: warning:", result.stdout)


if __name__ == "__main__":
    unittest.main()
