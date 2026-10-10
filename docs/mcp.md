# MCP server

`artcraft-toolbox-cli mcp` is an MCP server (Model Context Protocol, over stdio, built on the
official Rust SDK `rmcp`) that gives an agent the toolbox's whole command registry. Two modes:

- **Headless** (`artcraft-toolbox-cli mcp`): an in-process engine session on the toolbox's data
  folder. No window. Checks, installs, updates, rollbacks and the toolbox's own update all work.
- **Bridge** (`artcraft-toolbox-cli mcp --bridge 127.0.0.1:7878 --control-token-file <path>`):
  every tool is forwarded to a running `artcraft-toolbox --control 7878 --control-token-file
  <path>` over the control protocol (`docs/control-protocol.md`), so the agent also sees and
  drives the live window, and the user sees what the agent does.

A client configuration, for Claude Code, Claude Desktop or any client that starts stdio servers:

```json
{
  "mcpServers": {
    "artcraft-toolbox": { "command": "artcraft-toolbox-cli", "args": ["mcp"] }
  }
}
```

For bridge mode, add `"--bridge", "127.0.0.1:7878", "--control-token-file", "/private/path/artcraft-toolbox.token"`
to `args`, or set `ARTCRAFT_TOOLBOX_CONTROL_TOKEN_FILE` in the client's environment for the server.
The bridge accepts loopback addresses only, as the app listens on loopback only.

## Tools

| Tool | Arguments | Does |
|---|---|---|
| `apps_status` | none | every Crafting App's install and update status (`apps.status`) and the toolbox's own (`toolbox.status`). Start here |
| `command_list` | `filter?`, `enabled_only?` | the registry: id, label, parameter doc, `background`, `enabled` and `disabledReason` |
| `command_run` | `id`, `params?`, `wait?` | one engine command; a background command waits to finish unless `wait` is false, which returns `{job, pending}` |
| `command_batch` | `steps: [{id, params?, wait?}]`, `stop_on_error?` | several commands in order → `{completed, failed, results}` |
| `jobs_list` | none | the running background jobs with their progress |
| `jobs_cancel` | `job?` | cancel one job, or every running one |
| `ui_get` | none | bridge only: the window's state (`ui.get`) |
| `ui_set` | `fields` | bridge only: change it (`ui.set`; unknown fields are an error and nothing changes) |
| `control_call` | `method`, `params?` | bridge only: any control method, passed through |

Tool names use underscores (dots are not valid in every client). Every tool has a title and all
four annotation hints; the generic `command_run`, `command_batch` and `control_call` advertise
writes because the command decides. Hints never grant permission. Headless mode answers the three
bridge-only tools with a tool error that says how to start bridge mode.

Command parameters are the engine's (`docs/architecture.md` § 6). Typical sessions:

- Update everything: `apps_status`, then `command_run {id: "updates.check"}`, then
  `command_run {id: "apps.updateAll"}` (or `app.update` per app), then `apps_status` again.
- Install a specific version: `command_run {id: "app.releases", params: {app: "photocraft"}}`
  to see what is published, then `command_run {id: "app.install", params: {app: "photocraft", version: "0.5.0"}}`.
- Something went wrong: `command_run {id: "app.versions", params: {app: …}}`, then `app.rollback`.
- Keep the toolbox current: `command_run {id: "toolbox.update"}`; the new version is used at the
  next start (`toolbox.apply` swaps it in while the app isn't running; in bridge mode the window
  offers "Restart to update").

Every install goes through the same verification as a click in the window (`SECURITY.md`):
checksums first, then the platform signature, and nothing ever elevates.

## Resources

`artcraft-toolbox://apps` and `artcraft-toolbox://commands` return the live JSON of `apps_status`
and `command_list` (`application/json`). Resource reads are uncached; the tool and resource
catalogs are private and cached for ten minutes. List and read responses carry the MCP
2026-07-28 result and cache fields for peers that negotiated that version, and the historical
shape for older ones.

## Errors and limits

- Unknown tool arguments return JSON-RPC `-32602`, naming the key, before anything runs; batch
  step envelopes are strict too. The engine's own parameter errors come back as tool errors
  (`isError: true`) with the engine's message.
- A malformed JSON line gets `-32700` with `id: null`; the next line is still served.
- A command that panics becomes a tool error and the session keeps serving.
- Tool results are checked as encoded JSON against the 8 MiB reply ceiling; a batch stops at the
  step whose result no longer fits, keeping the results so far. `command_batch` takes at most 256
  steps.
- In bridge mode, a transport failure or a 60-second reply timeout drops the connection and reports
  that the operation may have completed. The bridge never resends the request; the next call
  reconnects and authenticates again. Inspect `apps_status` before retrying an install.

Implementation: `crates/automation/src/server.rs` (tools), `server/conventions.rs` (catalog
metadata, strict envelopes, resources), `server/transport.rs` (line recovery), `bridge.rs` (the
client of the control protocol). Tests: `crates/automation/tests/{mcp,mcp_wire,bridge}.rs`.
