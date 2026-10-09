# TheDancingDeveloper-org/codex — fork notes

This fork carries a small set of patches on top of upstream
[openai/codex](https://github.com/openai/codex) so the Vogt engine can drive it.
Upstream lands about 1,500 commits a month, so the patch set is kept small and
every patch is a separate, rebasable commit. The target is under about 10
carried commits. Extension points (config, managed hooks in
`requirements.toml`, app-server, MCP servers, external binaries) come before
core edits.

## Branch model

- `main` is a pure mirror of upstream `main`. Nothing fork-specific is ever
  committed to it, and it is never the base of a fork pull request.
- `fork/main` is the fork line: an upstream stable tag plus the carried patch
  series. It was created at `2351d9e1b` (upstream tag `rust-v0.162.0`).
- All fork work branches off `fork/main` and every fork pull request targets
  `fork/main`. Never open, comment on, or push anything to openai/codex.
- Vogt pins `fork/main` commit SHAs, not tags and not `main`.

Upstream history is never rewritten. Upstream commits mirror an internal
OpenAI repository and carry `GitOrigin-RevId` trailers; leave them intact.

## Rebase procedure

Run this for each new upstream stable tag (`rust-vX.Y.Z`), weekly or whenever
a tag ships:

1. `git fetch upstream --tags` and check out a fresh branch from the tag.
2. Cherry-pick (or rebase) the carried patch series from `fork/main` onto it,
   one commit at a time. A patch that no longer applies cleanly is reworked or
   dropped in its own commit, never squashed into another patch.
3. Run the patched crates' tests: app-server, tui, core spawn, agents_md,
   hooks. Regenerate the app-server schema fixtures.
4. Replay the proxy fixtures against the new Responses request shapes.
5. Fast-forward `fork/main` to the result and push it.
6. The release workflow (`.github/workflows/fork-release.yml`) builds static
   musl binaries and publishes a GitHub release tagged with the full
   `fork/main` commit SHA.

Record the new base tag and any dropped or reworked patch in the table below.

## Carried patches

| Commit | What | Upstream status |
|---|---|---|
| — | none yet | — |

## Workflows

Upstream workflows that need OpenAI-owned runners, BuildBuddy, signing, npm
OIDC, R2, winget, or `CODEX_OPENAI_API_KEY` are disabled on this fork with
`gh workflow disable` — the workflow files themselves are not edited, so
rebasing onto upstream stays conflict-free. The fork's own workflows are new
files with fork-unique names (`fork-*.yml`).
