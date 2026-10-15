# Upstream sync

How `fork/main` is rebased onto each new upstream stable tag. The branch model
and the carried-patch table live in `FORK.md`; this is the runbook.

Upstream shipped 40 stable tags (`rust-v0.144` to `rust-v0.162`) in three
months and about 1,559 commits in 30 days, so this runs on every stable tag.
There is no weekly schedule: GitHub runs `schedule` only from the repository's
default branch, and that branch is `main`, the upstream mirror. The drift
check below is run on `workflow_dispatch` and by following this runbook.

## Steps

1. `git fetch upstream --tags` and note the newest `rust-vX.Y.Z` stable tag
   (skip `alpha` and `dev` tags).
2. Branch from that tag and replay the carried patch series from `fork/main`,
   one commit at a time. A patch that no longer applies is reworked or dropped
   in its own commit, never folded into another patch.
3. Test the patched crates: app-server, tui, core spawn, agents_md, hooks.
4. Regenerate the app-server schema fixtures and commit the diff.
5. Replay the Responses proxy fixtures against the new request shapes and fix
   anything that drifted.
6. Update `upstream_version` in `.github/workflows/fork-release.yml` and the
   base tag recorded in `FORK.md`, and update the carried-patch table.
7. Move `fork/main` to the result with
   `git push --force-with-lease=refs/heads/fork/main:<expected-old-sha>`.
   A rebase onto a new tag is **not a fast-forward**, so a plain push is
   rejected and a plain `--force` is never used: `--force-with-lease` refuses
   to overwrite the branch if someone else has moved it since you last saw it.
8. The `fork-release` workflow then publishes a GitHub release tagged
   `fork-<full sha>`. A bare 40-hex SHA is not a valid tag name, which is why
   the tag carries the prefix.

## Release tags stay reachable

The Vogt engine pins `fork/main` by commit SHA and installs the release tagged
`fork-<sha>` (WI-1142). Once a release is published its tag must keep pointing
at the commit it was built from, and that commit must stay reachable — never
delete a release tag and never garbage-collect its commit. Moving `fork/main`
does not move a tag, so older releases keep working after every rebase.

## Watching upstream

- Keep the carried-patch count under about 10. The generic patches (thread id,
  title restore, hook workdir, spawn output_schema, skills roots) are
  candidates to offer upstream; once one lands, drop it from the series.
- Watch for upstream removals that the Vogt engine depends on. This already
  happened with `mcp-server`, `--full-auto`, `wire_api=chat`, and ghost
  snapshots. A removal means an engine change, not a fork patch that
  reintroduces it.

## Drift check

`.github/scripts/fork-upstream-drift.sh` is the check, and
`.github/workflows/fork-upstream-drift.yml` runs it on `workflow_dispatch`.
It has no `schedule` trigger, because that would only fire from `main`. It
fetches upstream's tags and fails when the newest stable `rust-vX.Y.Z` (alpha
and dev tags excluded) is newer than both the rebase base recorded in
`FORK.md` (and the `upstream_version` stamp in
`.github/workflows/fork-release.yml`) and the separate `Upstream fixes taken
through:` marker in `FORK.md`. The marker is how a cherry-pick is recorded
without moving the rebase base. It also fails when the carried-patch table
grows past ten rows. It only reports; the rebase stays the steps above.

Last run, 2026-10-15: `FORK.md` and `fork-release.yml` still record the rebase
base `rust-v0.162.0`. The newest stable tag on upstream is `rust-v0.162.1`,
and its three commits are being taken by cherry-pick (a separate WI-1125 pull
request), not by rebasing `fork/main`, because published `fork-<sha>` tags
must stay reachable. `FORK.md` records that as `Upstream fixes taken through:
rust-v0.162.1`.
