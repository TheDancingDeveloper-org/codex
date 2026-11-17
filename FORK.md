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
  series. The rebase onto upstream tag `rust-v0.169.0` (`d32063f96c`) is
  branch `wi-4323-rebase-0.169.0` (WI-4323); `fork/main` moves there once it
  is force-pushed with a lease and released.
- Upstream fixes taken through: rust-v0.169.0
  The 0.162.1 cherry-picks (#52648, #51866) are in 0.169.0 and were dropped
  from the carried series by the rebase. The base and this marker are now the
  same tag.
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
| — | Per-handler hook `failClosed` (omitted from serialization when false, so the hook identity hash and existing trust approvals of handlers that do not set it are unchanged; fork/main c6d8d792af) and `write_stdin` review input. `HookHandlerConfig` now lives in `protocol/src/hooks.rs`; `write_stdin` has no `session_id` in its args, so it is parsed locally. WI-4323 rebase. | fork-only |
| — | `spawn_agent` `output_schema`: `ThreadSpawnRequest.output_schema` feeds `SpawnAgentOptions.final_output_json_schema` (v2 spawns go through `thread_spawn`). | fork-only |
| — | Caller-chosen thread id on thread start, and `exec --session-id` kept off the resume and fork positionals. | fork-only |
| — | Configurable `[skills]` extra roots, Claude-compat global instructions, `project_doc_mode` and confined `@import` expansion (`ext/skills/src/host_roots.rs`, `core/src/agents_md.rs`). | fork-only |
| — | Model search discovery stays stable when Apps finishes starting up mid-turn: `shares_model_search_source` replaces the `Arc::ptr_eq` base check (`core/src/model_search.rs`, `codex-mcp/src/model_search.rs`; WI-4137; fork/main f23f8fd2b8). Re-carried after the first rebase attempt dropped it. | fork-only; not fixed in 0.169 |
| — | `codex doctor` accepts the `agent-state` title item (`cli/src/doctor/title.rs`; fork/main b9e7e0ddec). | fork-only, goes with the agent-state patch |

### Dropped or reworked in the 0.169.0 rebase (WI-4323)

Every `fork/main` 5c4381bb commit after the merge base `6dfaca4dad` is either
carried above, reworked, or dropped here.

Dropped, superseded by upstream 0.169.0:

- `3e2cb8584e` and `b7f4b598d7` (resolved workdir in the exec_command
  PreToolUse payload) and `674c84189c` (non-blocking start-cwd lookup).
  Upstream now sends the resolved directory as the hook's `cwd`
  (`pre_tool_use_cwd`), for local environments only. `tool_input.workdir` is
  no longer in the payload; nothing in vogt reads it.
- `a0a515ba17` (hermetic repo-ancestry test): fixed upstream.
- `a19cabb2ba` (#52648), `88286d41c3` (#51866): the 0.162.1 cherry-picks, in
  0.169.0.
- `a028250541` (#51812), `f86cffa3a7`, `c1382380de`: upstream commits that
  arrived through earlier merges; in 0.169.0.

Reworked for 0.169.0 internals (same behaviour, new shapes):

- `wire_api = "chat"` (new `http_transport` / `map_response_stream` API,
  `ResponseItem`/`ContentItem`/`Prompt` shapes).
- failClosed (`HookHandlerConfig` moved to `protocol/src/hooks.rs`) and the
  `write_stdin` hook input (parsed locally; no `session_id` in its args).
- `spawn_agent` `output_schema` (v2 goes through `thread_spawn`).
- Headless MCP startup (`exec/src/lib.rs` mutability fix).
- Skills extra roots and `agents_md` (re-done by hand).
- The scenario snapshots `7d861cdae2`, `7bcbbc47c5`: re-taken in a clean HOME;
  only the spawn_agent hash lines differ from upstream.
- `1e672adbfc`, `ec939d3d89` (exec override binding and its rustfmt): folded
  into the exec rework.
- `.github/`: fork-added files restored from fork/main 5c4381bb unchanged
  (`fork-ci.yml`, `fork-release.yml`, `fork-ci-*.sh`,
  `fork-ci-known-failures.txt`, `fork-changed-packages.py`, ...). The only
  edit is the `fork-release.yml` `upstream_version` stamp, now `0.169.0`.
  The fork-ci and quarantine commits (`30459fcdbc`, `82998424d8`, `31270db240`,
  `5316cb6e2e`, `8c54367b27`, `de93ab34e8`, `9d0846847a`, `49ace6eff7`,
  `1a91dee43f`, `8475fceee3`, `870a01b41e`, `a4c2fdd846`, `df176b49ef`,
  `9effa42ce5`, `53cda7fb47`, `700da378d3`, `f89b8a6920`, `c256821418`) come
  back with that restore.

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

`fork-ci` supports a merge queue (`merge_group`), skips its Rust steps when a
PR touches nothing under `codex-rs/` or `.github/`, and cancels superseded runs
per ref. How it runs on PRs, in the queue, and around upstream syncs:
`docs/ci.md`.
