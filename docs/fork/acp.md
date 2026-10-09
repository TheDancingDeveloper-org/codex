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

Against fork release `fork-99a1e425512f158a171c3e45cc4c5db5acdc0a6f`
(`codex-cli 0.162.0`, static musl, sha256
`e917606968361fac650d493aa8450c5d1409766c12c60ac2f71293eb866ce3df`), driven by
`@agentclientprotocol/codex-acp@2.1.1` over a scripted JSON-RPC client. Zed and
Neovim were not installed on the host, so neither editor was exercised.

| Step | Result |
|---|---|
| `initialize` (protocol v1) | Works. Agent reports `2.1.1`, `loadSession`, resume, list, close, fork. |
| `session/new` | Works. Returns a thread id and the provider's model list. |
| Gateway auth | Works. With `model_provider = "theclawbay"` the adapter reports `authStatus.kind = gateway` and does not demand an OpenAI login. |
| `session/prompt` | Reached the fork and the provider, then failed upstream of the adapter: theclawbay closed the Responses stream ("model service is temporarily unavailable", five reconnects, `systemError`). Not an adapter or fork defect. |
| Approval prompt | Not reached. No turn ran far enough to call a tool. |
| `session/load` (resume) | Advertised (`loadSession: true`) but not exercised, because no turn completed to resume. |

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
