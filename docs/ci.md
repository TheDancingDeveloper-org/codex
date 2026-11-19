# Fork CI: pull requests, the merge queue, and `fork/main`

Fork-only. Upstream's CI is disabled on this fork (see `FORK.md`,
"Workflows"); `fork-ci` is the one check that runs on pull requests into
`fork/main`. This file covers how it is meant to run with a merge queue
(WI-1886).

## What runs where

| Event | `fork-ci` |
|---|---|
| `pull_request` into `fork/main` | Rust steps only when the fork-owned delta touches `codex-rs/` or `.github/`; fmt/clippy/nextest only for the packages the fork owns |
| `merge_group` (the queue) | the same selection: every PR in a queue group is fork-owned, so the group's delta is inside the fork-owned delta |
| `workflow_dispatch` | the packages named in `packages`, or the selection |

A pull request that touches only docs, `FORK.md` or `sdk/` still reports
`fmt, clippy, test` as passed, with the Rust steps skipped, so a required
check never waits on it.

### What the fork tests

The fork trusts upstream's CI for upstream's code. Every commit on upstream
`main` has passed upstream's own checks, and the fork never edits those files.
What nobody else tests is the fork's own delta: the carried patch series and
the fork-only files, as they sit on the upstream commit the tree merged or
rebased onto. So `fork-ci` diffs HEAD against its merge-base with upstream
`main` (fetched from `openai/codex` in the `plan` job; the fork's `main`
mirror can lag) and selects the packages that own those files
(`.github/scripts/fork-ci-select.sh`, `fork-changed-packages.py`). The same
base feeds clippy's hunk gate, so fork-owned lines lint clean and upstream
lint debt never fails a run.

Why not diff against `fork/main`: an upstream sync then looks like a change
to every package (PR #37 touched 4,673 files), and a workspace-wide nextest
over hundreds of upstream crates took 95-105 minutes, past the merge queue's
90-minute `check_response_timeout`, so the queue dropped the entry (WI-4161).

What the selection ignores on purpose:

- The fork's version restamp. `Cargo.toml` and `Cargo.lock` carry the release
  version where upstream has `0.0.0`; the workflow resets the manifest before
  selecting, the workspace manifest counts only when it differs beyond its
  version line, and the lock is compared by resolve with both sides at
  `0.0.0`.
- A member `Cargo.toml` selects its own package only. A fork-owned manifest
  hunk is a dependency a patch added, and dependents compile against the same
  API; a patch that changes an API changes `.rs` files, and dependents the
  fork edits are selected by those edits.
- There is no whole-workspace `cargo check` any more. clippy compiles the
  selected packages with `--all-targets` and fails on any rustc error.

What this gives up: a regression upstream introduces in a crate the fork does
not touch ships in a fork release untested here. Upstream's CI is the check
for that. A behaviour change upstream that a carried patch depends on is
caught only if the patch's own tests exercise it; the patched crates'
tests always run.

The two heavy halves (`test`, `test (app-server integration)`) and the
`fmt, clippy` job run in parallel and report through one aggregator job
that keeps the required context name; see `fork-ci.yml`.

### Concurrency

`fork-ci` cancels in progress per ref. A force-push to a PR cancels the
superseded run. Each queue entry has its own `gh-readonly-queue/fork/main/...`
ref, so entries never cancel each other.

### Runners

Same-repo PRs, queue entries and `workflow_dispatch` run on node-f: the main
half on `[self-hosted, rust-heavy, node-f]` and the app-server half on
`[self-hosted, bigmem, node-f]`. `ci-bigmem-0` is not used (its scratch disk is
too slow for a Codex workspace build). `fork-release` is pinned to
`[self-hosted, bigmem, node-f]` too. A PR from another fork stays on GitHub-hosted
`ubuntu-24.04`, so untrusted code never reaches a persistent runner. A
`merge_group` event is always trusted: only a maintainer can enqueue.

## Landing

`land-pr` (workspace `.agent-tools/bin/land-pr`) detects a merge queue from
the base branch's rulesets. It enqueues with `gh pr merge --auto` and follows
the entry's `merge_group` run. Don't rebase a queued PR by hand.

## Upstream syncs

A sync merges upstream into `fork/main` through a `sync-upstream-<date>` PR
(the PR #37 model; `docs/fork-upstream-sync.md`). It goes through the queue
like any PR, and because the selection diffs against the upstream merge-base,
it tests the carried patches on the new base and nothing else. `fork/main` is
never rebased or force-pushed: graft follows it, and a non-fast-forward move
diverges the mirror. Don't run a sync while a graft batch PR is open or
queued.

## Graft batches

Workers land on the graft line `backlog-1`; oversight cuts `graft/batch-<n>`
PRs into `fork/main` with `graft-mirror pr-batch backlog-1 --base fork/main`,
at most daily. A batch merges through the queue (MERGE method, so each land
keeps its commit), with an Opus-level review when it touches exec, sandbox,
approvals or auth.

## The queue ruleset

Ruleset 25362303 ("fork/main merge queue") is active on `fork/main`: it
requires `fmt, clippy, test` and runs a merge queue with the MERGE method
(grouping ALLGREEN, check timeout 90 min). Only the repository admin role
bypasses it. The original proposal is in the WI-1886 PR description.
