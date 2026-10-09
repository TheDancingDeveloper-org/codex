# ACP against the fork binary

The fork does not ship an Agent Client Protocol server, and it should not grow
one. Editors that speak ACP reach the fork through the upstream adapter
[`@agentclientprotocol/codex-acp`](https://www.npmjs.com/package/@agentclientprotocol/codex-acp)
(Apache-2.0, repo `agentclientprotocol/codex-acp`). The adapter starts
`codex app-server` over stdio and translates ACP into it. Nothing in the fork
is patched for this.

The older Rust adapter `zed-industries/codex-acp` is archived and linked Codex
crates directly. Do not revive that model: it rebases on every upstream churn.

## Install

```bash
npm install -g @agentclientprotocol/codex-acp@2.1.1
```

Or, without a global install:

```bash
npx -y @agentclientprotocol/codex-acp@2.1.1
```

Point it at a fork release binary rather than the `@openai/codex` it bundles:

```bash
CODEX_PATH=/path/to/codex-fork codex-acp
```

Take the binary from a published fork release (`fork-<full sha>`), never from a
local build. Verify the asset against the release `SHA256SUMS` — those sums
cover the files inside the tarball (`x86_64-unknown-linux-musl/codex` and
friends), not the tarball itself.

## Client configuration

Zed (`settings.json`):

```json
{
  "agent_servers": {
    "Codex Fork": {
      "type": "custom",
      "command": "codex-acp",
      "env": { "CODEX_PATH": "/path/to/codex-fork" }
    }
  }
}
```

Neovim (the `agentclientprotocol` plugin, or any client that spawns a stdio
agent) uses the same command and `CODEX_PATH`.

The adapter reads `~/.codex/config.toml` through the binary, so the fork's
`model_provider` and auth config apply unchanged. Useful environment variables,
all read by the adapter:

| Variable | Effect |
|---|---|
| `CODEX_PATH` | Fork binary to run instead of the bundled Codex |
| `CODEX_HOME` | Alternate Codex home (config, sessions) |
| `CODEX_API_KEY` / `OPENAI_API_KEY` | API-key auth, if not using a configured provider |
| `INITIAL_AGENT_MODE` | `read-only`, `workspace-write`, `agent`, or `agent-full-access` |
| `NO_BROWSER=1` | Hide the ChatGPT browser login on headless hosts |
| `APP_SERVER_LOGS` | Directory for adapter logs |

## What maps where

| ACP | Codex app-server |
|---|---|
| `session/new`, `session/load` | `thread/start`, `thread/resume` |
| mode changes | `thread/settings/update` |
| `/skills` | `skills/list` |
| plan updates | `turn/plan/updated` |
| diffs | `turn/diff/updated` |
| permission prompts | `item/*/requestApproval`, `item/tool/requestUserInput` |

The adapter does not read an editor's unsaved buffers. Codex reads files from
disk, so save before asking it about a buffer.

## Verified (2026-10-09, WI-1135)

Against the newest published fork release at the time of the run,
`fork-ef97503e583191fdea5d8061a4553ed82436dc86` (`codex-cli 0.162.0+fork.ef97503e5`,
static musl, sha256 `b7a46b416d05941f95c5ab9bb1a39969d2d3c07c9b543858d8cef5b8d51a3611`),
driven by `@agentclientprotocol/codex-acp@2.1.1` over a scripted JSON-RPC client.
Zed and Neovim were not installed on the host, so neither editor was exercised.

The model was `grok-4.7`, set only in a throwaway `CODEX_HOME`. The GPT models
behind the same `theclawbay` Responses endpoint were returning
`service_unavailable`, and `grok-4.7` on that endpoint completes. No provider
setting outside that throwaway home was changed.

| Step | Result |
|---|---|
| `--version` stamp | `codex-cli 0.162.0+fork.ef97503e5`. The `+fork.<sha>` stamp ships from `8247a20` on; the earlier release `fork-99a1e425` printed a bare `codex-cli 0.162.0`. |
| `initialize` (protocol v1) | Works. Agent reports `2.1.1`, and advertises `loadSession`, resume, list, close and fork. |
| `session/new` | Works. Returns a thread id; the app-server reports `authStatus.kind = gateway`, so no OpenAI login is demanded. |
| `session/prompt` | Works. Thread `01a12267-f3f2-7771-b849-43f982306b05` was asked to reply with one word and the rollout's `task_complete` records `last_agent_message: "pong"` with no error. Usage is attributed to `grok-4.7`. |
| `session/load` (resume) | Works. Loading that same thread and prompting again appended a second turn to its rollout, which also completed with `last_agent_message: "pong"`. |
| Approval round-trip | Not reached, and not an adapter failure. Across three prompts (run a shell command, write a file, and the same shell command after `session/set_mode` to `agent-full-access`) `grok-4.7` answered in prose and never emitted a tool call, so neither `item/commandExecution/requestApproval` nor `session/request_permission` fired. The adapter's default mode is `agent`, whose approval policy is `on-request`, so a tool call would have raised one. A direct `codex app-server` session behaved the same way. |

Two things the work item asked to check came out differently than expected:

- **No standalone binary is published.** `2.1.1` has no release assets. The npm
  package ships only `dist/index.js` (a Node ESM bundle, shebang
  `#!/usr/bin/env node`), so Node is required to run the published adapter. The
  repo does know how to build one — `npm run bundle:linux-x64` runs
  `bun build --compile` — but that binary is not part of any release. Building
  it means cloning the adapter repo, which this fork does not do.
- **The peer pin is `@openai/codex ^0.159.1`, not `^0.160.1`.** The published
  package bundles `0.159.3`. The fork base is `0.162.0`, which is outside that
  range. `CODEX_PATH` bypasses the pin entirely — the adapter execs whatever
  binary it is given and talks app-server protocol to it — and that is exactly
  what was tested above, so the pin does not block the fork. It does mean
  `npm install` alone, without `CODEX_PATH`, would fetch a Codex older than
  this fork.

Two things the work item asked to check came out differently than expected:

- **No standalone binary is published.** `2.1.1` has no release assets. The npm
  package ships only `dist/index.js` (a Node ESM bundle, shebang
  `#!/usr/bin/env node`), so Node is required to run the published adapter. The
  repo does know how to build one — `npm run bundle:linux-x64` runs
  `bun build --compile` — but that binary is not part of any release. Building
  it means cloning the adapter repo, which this fork does not do.
- **The peer pin is `@openai/codex ^0.159.1`, not `^0.160.1`.** The published
  package bundles `0.159.3`. The fork base is `0.162.0`, which is outside that
  range. `CODEX_PATH` bypasses the pin entirely — the adapter execs whatever
  binary it is given and talks app-server protocol to it — and that is exactly
  what was tested above, so the pin does not block the fork. It does mean
  `npm install` alone, without `CODEX_PATH`, would fetch a Codex older than
  this fork.

## When this is no longer enough

Only if the adapter cannot express something the fork needs: add a small Rust
ACP server on `InProcessAppServerClient` inside the fork. That is a couple of
thousand lines and rebases against app-server, so it is the fallback, not the
plan. Do not start it without the fork epic lead agreeing.
