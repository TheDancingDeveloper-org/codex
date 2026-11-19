# Upstream sync

How upstream is brought into `fork/main`. The branch model and the
carried-patch table live in `FORK.md`; this is the runbook.

Upstream shipped 40 stable tags (`rust-v0.144` to `rust-v0.162`) in three
months and about 1,559 commits in 30 days, so a sync runs on every stable tag.
There is no weekly schedule: GitHub runs `schedule` only from the repository's
default branch, and that branch is `main`, the upstream mirror. The drift
check below is run on `workflow_dispatch` and by following this runbook.

## Merge model (current)

A sync **merges** upstream into `fork/main` through a pull request. It never
rebases `fork/main` and never force-pushes it (decision D1, 2026-11-13;
PR #37 is the first sync done this way).

Why: `fork/main` is followed by graft (see "Graft follower" below). graft's
follower never force-pushes, and a non-fast-forward move of a followed branch
is a divergence: the mirror holds, and every land on the graft line would sit
on history that no longer exists on GitHub. A merge commit is a fast-forward
of `fork/main`, so the follower imports it like any other move.

## Steps

1. `git fetch upstream --tags` and note the newest `rust-vX.Y.Z` stable tag
   (skip `alpha` and `dev` tags), or the upstream `main` commit to take.
2. Check with oversight that no graft batch PR into `fork/main` is open or
   queued. One writer of `fork/main` at a time: land the open batch first, or
   merge the sync first and batch after.
3. Branch `sync-upstream-<YYYY-MM-DD>` off the tip of `fork/main` and
   `git merge` the upstream tag or commit into it. Resolve conflicts in the
   merge commit; a carried patch that needs rework gets its own follow-up
   commit on the same branch.
4. Test the patched crates: app-server, tui, core spawn, agents_md, hooks.
5. Regenerate the app-server schema fixtures and commit the diff.
6. Replay the Responses proxy fixtures against the new request shapes and fix
   anything that drifted.
7. Update `upstream_version` in `.github/workflows/fork-release.yml`, the
   "Upstream fixes taken through" marker and sync record in `FORK.md`, and the
   carried-patch table.
8. Open a pull request from the sync branch into `fork/main`. It goes through
   the merge queue like any other PR (`docs/ci.md`); because `fork-ci` diffs
   against the upstream merge-base, it tests the carried patches on the new
   base and trusts upstream's CI for the rest.
9. Once it merges, the `fork-release` workflow publishes a GitHub release
   tagged `fork-<full sha>` (a bare 40-hex SHA is not a valid tag name, which
   is why the tag carries the prefix). Oversight then syncs the graft line
   onto the new `fork/main` before cutting the next batch.

## Graft follower

`fork/main` is mirrored into the graft repository `codex`. GitHub `fork/main`
stays the source of truth; graft only writes it through merged batch PRs.

- Workers clone the graft line `backlog-1` (`g clone codex <dir> --line
  backlog-1`), run checks for the crates they touched, and land with
  `g land`. They do not push branches or open pull requests.
- Oversight cuts batch PRs with
  `graft-mirror pr-batch backlog-1 --base fork/main`, which pushes
  `graft/batch-<n>` and opens a PR into `fork/main`. The batch merges
  through the `fork/main` merge queue (MERGE method, so each land keeps its
  own commit, and the queue adds its group diff and workspace
  `cargo check`).
- Batches are cut at most daily, or sooner for a release-relevant fix: each
  batch costs about two hours of runner time plus a release build.
- A batch is reviewed as a whole, at Opus level when it touches exec,
  sandbox, approvals or auth; otherwise a lighter review.
- Conflicts between a sync and in-flight lands are resolved in graft by the
  worker that owns the area (`g sync`, fix, `g land`), never by rewriting
  `fork/main`.
- If `fork/main` is ever force-pushed anyway, the mirror has to be removed
  and re-imported, the graft line rebuilt, and every unbatched land re-landed
  by hand. The merge model exists to avoid that.

## CI and release runners

- `fork-ci` runs same-repo PRs, queue entries and `workflow_dispatch` on
  node-f: the main half on `[self-hosted, rust-heavy, node-f]` and the
  app-server half on `[self-hosted, bigmem, node-f]` (it was OOM-killed on the
  10 GiB docker runner). `ci-bigmem-0` is not used: its scratch disk is too
  slow for a Codex workspace build. PRs from other forks stay on GitHub-hosted
  `ubuntu-24.04`.
- `fork-release.yml` builds on `[self-hosted, bigmem, node-f]`
  (`nodef-gha-public-rust`, 24 GiB, `CARGO_BUILD_JOBS: 5`) and installs the
  musl build tools when the runner lacks them (WI-4161). Every push to
  `fork/main` publishes a release tagged `fork-<full sha>`; the publish job
  runs GitHub-hosted.

## How Vogt consumes a release

The Vogt engine pins a fork release in its `engine/agent-versions.env`:
`VOGT_CODEX_SHA` is the full `fork/main` commit, alongside the sha256 of the
`x86_64-unknown-linux-musl` codex binary from the `fork-<sha>` release. The
engine installs only that pinned fork build and refuses stock npm Codex.
Moving the pin is a separate Vogt change; a fork merge never moves it.

## Superseded: rebase procedure

Until 2026-11-13 a sync rebased the carried series onto each new upstream
stable tag and moved `fork/main` with
`git push --force-with-lease=refs/heads/fork/main:<expected-old-sha>`. That
is retired by D1 because it rewrites the followed branch. It is kept here as
a record only; do not run it.

## Release tags stay reachable

The Vogt engine pins `fork/main` by commit SHA and installs the release tagged
`fork-<sha>` (WI-1142). Once a release is published its tag must keep pointing
at the commit it was built from, and that commit must stay reachable — never
delete a release tag and never garbage-collect its commit. Moving `fork/main`
does not move a tag, so older releases keep working after every sync.

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
grows past ten rows. It only reports; the sync is the steps above.

Last run, 2026-10-15: `FORK.md` and `fork-release.yml` still record the rebase
base `rust-v0.162.0`. The newest stable tag on upstream is `rust-v0.162.1`,
and its three commits are being taken by cherry-pick (a separate WI-1125 pull
request), not by rebasing `fork/main`, because published `fork-<sha>` tags
must stay reachable. `FORK.md` records that as `Upstream fixes taken through:
rust-v0.162.1`.
