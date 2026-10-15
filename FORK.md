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
  `rust-v0.162.0`.
- Upstream fixes taken through: rust-v0.162.1
  This line is separate from the rebase base above. It records fixes taken by
  cherry-pick, which does not move `fork/main`, because published `fork-<sha>`
  tags must stay reachable. `rust-v0.162.1`'s three commits are being taken
  that way, in a separate WI-1125 pull request. The tag lives on a release
  branch, not on upstream `main`:
  `2351d9e1b` (upstream `main` at fork time) is 114 commits away and not
  descended from the tag, so it is not a valid base.
- All fork work branches off `fork/main` and every fork pull request targets
  `fork/main`. Never open, comment on, or push anything to openai/codex.
- Vogt pins `fork/main` commit SHAs, not tags and not `main`.

Upstream history is never rewritten. Upstream commits mirror an internal
OpenAI repository and carry `GitOrigin-RevId` trailers; leave them intact.

## Rebase procedure

Run this for each new upstream stable tag (`rust-vX.Y.Z`), whenever a tag
ships. There is no weekly schedule: GitHub runs `schedule` only from the
default branch, which is the upstream mirror, so the drift check is run on
`workflow_dispatch` and from the runbook at `docs/fork-upstream-sync.md`.

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
`.github/workflows/fork-upstream-drift.yml` fails when a newer stable tag has
shipped than both the rebase base recorded here and the "Upstream fixes taken
through" marker above, or when the table below passes ten rows. The marker
passing on its own is enough: a cherry-pick does not move the base.

## Carried patches

| Commit | What | Upstream status |
|---|---|---|
| — | `agent-state` terminal title item (`codex: ready\|working\|awaiting approval`), written only on change, with the user's title pushed (`CSI 22;0t`) at the first title write and popped (`CSI 23;0t`) on exit. Idle with `animations = false` schedules no refresh. Landed on `fork/wi-1127-agent-state` at `b5f9b014`. | fork-only; Klaudia contract from vogt `internal/tui/idle.go` |
| — | `wire_api = "chat"`: Chat Completions client restored for providers whose Responses path drops tool calls (grok via theclawbay/LiteLLM, WI-1139). New files only, plus one dispatch arm: `core/src/chat_request.rs` (request conversion; namespaced tools flattened to `ns__name`, freeform tools such as apply_patch exposed as a function taking one `input` string, calls restored by `ChatToolMap`), `codex-api/src/{endpoint,sse}/chat.rs` (client and SSE parser, fixture tests in `sse/chat_tests.rs`). Reasoning, web search and tool search are not sent over chat. Branch `fork/wi-1286-chat-wire`. | fork-only; upstream removed the chat client |
| — | Headless MCP startup (issue #26, WI-1514): `codex exec` defaults `mcp_optional_startup_grace_ms` to 0 unless set, so every enabled server's startup is awaited (each bounded by its `startup_timeout_sec`) before the first request; a failed or timed-out server is printed as a warning and the run continues. Tool dispatch resolves un-namespaced `mcp__<server>__<tool>` names and bare names unique across MCP servers, and answers a call to a server still handshaking with "MCP server `x` is still starting" instead of "unsupported call". `exec/src/lib.rs`, `core/src/tools/registry.rs`, `codex-mcp` `pending_startup_servers`. Branch `fix/issue-26-mcp-startup`. | fork-only |

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

GitHub paused every workflow on the fork on 2026-10-15 over Actions usage.
After it was re-enabled, the issue bots (`issue-deduplicator`, `issue-labeler`,
`issue-translator`) were disabled too, leaving only `fork-ci` and
`fork-release` active. `postmerge-ci` (push to `main`, no fork guard) is not
registered yet and so cannot be disabled ahead of time: after the next mirror
sync of `main`, cancel its run and `gh workflow disable postmerge-ci`.
`fork-release` builds x86_64 only, on the self-hosted GCP `rust-heavy` runner,
whose image bakes the musl build packages (WI-1514); only its short publish job
runs GitHub-hosted. `fork-upstream-drift` is the check that the recorded base tag, or the
"Upstream fixes taken through" marker, is still the newest stable tag
(WI-1125). It has no schedule: run it on `workflow_dispatch`, or from the
sync runbook.
