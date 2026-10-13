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
  series. It currently sits on `c1382380d`, the commit of upstream tag
  `rust-v0.162.0`. The tag lives on a release branch, not on upstream `main`:
  `2351d9e1b` (upstream `main` at fork time) is 114 commits away and not
  descended from the tag, so it is not a valid base.
- `integrate/codex-next` is the landing base for fork work. It is `fork/main`
  plus the work in flight, and workers branch from it, not from `fork/main`.
  The flow is:
  - A worker branches from `origin/integrate/codex-next` in its own worktree,
    on `wi-<n>-<slug>`, and lands by pushing that branch. It opens no pull
    request.
  - Oversight reviews the pushed branch and merges it into
    `integrate/codex-next` by rebase or fast-forward only.
  - The integration pull request from `integrate/codex-next` targets
    `fork/main`, never `main`, and the operator approves it. Merging it
    publishes `fork-<sha>`, where `<sha>` is the full commit SHA of the new
    `fork/main`.
- The integration branch is rebased onto `fork/main` when `fork/main` moves.
  Never merge `fork/main` or `main` into `integrate/*`, and never leave a merge
  commit on it. History stays linear.
- Never open, comment on, or push anything to openai/codex.
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
5. Update `fork/main` to the result with `git push --force-with-lease` and
   publish the release. A rebase onto a new tag is not a fast-forward, and a
   plain `--force` is never used. The procedure itself, including why each
   published release tag must stay reachable, is the runbook at
   `docs/fork-upstream-sync.md`. The release it publishes is tagged
   `fork-<full sha>`, not the bare SHA.

Record the new base tag and any dropped or reworked patch in the table below.

## Carried patches

| Commit | What | Upstream status |
|---|---|---|
| — | `agent-state` terminal title item (`codex: ready\|working\|awaiting approval`), written only on change, with the user's title pushed (`CSI 22;0t`) at the first title write and popped (`CSI 23;0t`) on exit. Idle with `animations = false` schedules no refresh. Landed on `fork/wi-1127-agent-state` at `b5f9b014`. | fork-only; Klaudia contract from vogt `internal/tui/idle.go` |
| — | `wire_api = "chat"`: Chat Completions client restored for providers whose Responses path drops tool calls (grok via theclawbay/LiteLLM, WI-1139). New files only, plus one dispatch arm: `core/src/chat_request.rs` (request conversion; namespaced tools flattened to `ns__name`, freeform tools such as apply_patch exposed as a function taking one `input` string, calls restored by `ChatToolMap`), `codex-api/src/{endpoint,sse}/chat.rs` (client and SSE parser, fixture tests in `sse/chat_tests.rs`). Reasoning, web search and tool search are not sent over chat. Branch `fork/wi-1286-chat-wire`. | fork-only; upstream removed the chat client |

## External adapters

- ACP: the fork has no in-tree ACP server. Editors speak ACP through the
  upstream `@agentclientprotocol/codex-acp` adapter, pointed at a fork release
  binary with `CODEX_PATH`. Setup and what was verified: `docs/fork/acp.md`.

## Workflows

Upstream workflows that need OpenAI-owned runners, BuildBuddy, signing, npm
OIDC, R2, winget, or `CODEX_OPENAI_API_KEY` are disabled on this fork with
`gh workflow disable` — the workflow files themselves are not edited, so
rebasing onto upstream stays conflict-free. The fork's own workflows are new
files with fork-unique names (`fork-*.yml`).
