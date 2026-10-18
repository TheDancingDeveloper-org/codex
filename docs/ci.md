# Fork CI: pull requests, the merge queue, and `fork/main`

Fork-only. Upstream's CI is disabled on this fork (see `FORK.md`,
"Workflows"); `fork-ci` is the one check that runs on pull requests into
`fork/main`. This file covers how it is meant to run with a merge queue
(WI-1886).

## What runs where

| Event | `fork-ci` |
|---|---|
| `pull_request` into `fork/main` | Rust steps only when the diff touches `codex-rs/` or `.github/`; fmt/clippy/nextest only for the packages the diff changes |
| `merge_group` (the queue) | Rust steps for the packages the **whole queued group** changes, diffed against the queue's base (`merge_group.base_sha`), plus `cargo check --workspace` |
| `workflow_dispatch` | the packages named in `packages`, or the diff |

A pull request that touches only docs, `FORK.md` or `sdk/` still reports
`fmt, clippy, test` as passed, with the Rust steps skipped, so a required
check never waits on it.

### Why the queue is package-scoped too

Vogt's queue runs everything. Here, "everything" is a workspace-wide nextest
over hundreds of upstream crates, at roughly an hour on the 30-vCPU runner, and
upstream lint and test debt makes a full run fail regardless of the PR. The
queue keeps the package selection but widens the diff to the whole group, and
`cargo check --workspace --all-targets` still compiles every crate, so a
cross-crate API break fails the queue.

### Concurrency

`fork-ci` cancels in progress per ref. A force-push to a PR cancels the
superseded run. Each queue entry has its own `gh-readonly-queue/fork/main/...`
ref, so entries never cancel each other.

### Runners

Same-repo PRs, queue entries and `workflow_dispatch` run on the self-hosted GCP
Spot `rust-heavy` runner. A PR from another fork stays on GitHub-hosted
`ubuntu-24.04`, so untrusted code never reaches a persistent runner. A
`merge_group` event is always trusted: only a maintainer can enqueue.

## Landing

`land-pr` (workspace `.agent-tools/bin/land-pr`) detects a merge queue from
the base branch's rulesets. It enqueues with `gh pr merge --auto` and follows
the entry's `merge_group` run. Don't rebase a queued PR by hand.

## Upstream syncs

A rebase of `fork/main` onto a new upstream tag is a force-push
(`docs/fork-upstream-sync.md`), not a queued merge. Drain or pause the queue
before you push the rebase. The ruleset below bypasses only the repository
admin role, which is what lets the `--force-with-lease` push through.

## Turning the queue on (operator)

`fork/main` has no branch protection and no ruleset today, so `fork-ci` is not
required and the queue is off. The workflow supports the queue without
needing it. The exact ruleset is in the WI-1886 PR description: one branch
ruleset on `fork/main` that requires `fmt, clippy, test` and adds a
`merge_queue` rule.
