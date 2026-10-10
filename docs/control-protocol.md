# Control protocol

How agents, scripts and tests drive the toolbox from outside: one JSON request per line, one
JSON reply per line, over loopback TCP or a pipe. Two servers speak it:

- **The desktop app** (`artcraft-toolbox --control <port>`): the running window. Everything the
  engine does, plus the window's state (`ui.get`, `ui.set`) and quitting.
- **The headless server** (`artcraft-toolbox-cli serve`): one engine session without a window,
  on stdio or on a loopback port. The same engine methods, plus `batch`.

The MCP server (`docs/mcp.md`) wraps either one. PhotoCraft's protocol is the model; the
toolbox's surface is much smaller.

## Starting the desktop server

```sh
artcraft-toolbox --control 7878 --control-token-file /private/path/artcraft-toolbox.token
```

The server listens on `127.0.0.1:<port>` only. If the token file does not exist, the app creates
it with a fresh 256-bit token (on Unix with mode `0600`; on Windows, protect it with a user-only
ACL); an existing file is reused. `--control-token <64 hex>` passes the token directly (visible
in the process list; prefer the file). With neither, the app generates a token for this launch
and prints it to standard error, never to the log. The environment variables
`ARTCRAFT_TOOLBOX_CONTROL_PORT`, `ARTCRAFT_TOOLBOX_CONTROL_TOKEN` and
`ARTCRAFT_TOOLBOX_CONTROL_TOKEN_FILE` are the equivalents (an argument wins).

The first request on every connection must authenticate; nothing else is dispatched before it:

```json
{"id": "auth", "method": "auth", "params": {"token": "<64 hexadecimal characters>"}}
{"id": "auth", "ok": true, "result": {"authenticated": true}}
```

A wrong token gets one error reply and the connection is closed. After that, each request is
one line, each reply one line with the same `id`:

```json
{"id": 1, "method": "ui.get", "params": {}}
{"id": 1, "ok": true, "result": {"ui": {"tab": "apps", "search": "", …}, "notice": null, …}}
{"id": 2, "method": "engine.execute", "params": {"command": "app.status", "params": {"app": "nope"}}}
{"id": 2, "ok": false, "error": "unknown app `nope`"}
```

`params` must be a JSON object (omit it or pass `null` for none). Requests are handled on the
UI thread between frames (the transport wakes the window), so a reply reflects the state the next
frame draws. The server waits up to 60 seconds for a reply: a request still queued at that
deadline is refused with `"error": "timeout"` before it is dispatched; work that has started
is never cancelled by a timeout.

Transport: `apps/artcraft-toolbox/src/control_server.rs`. Handlers: `crates/ui-egui/src/control.rs`.

## Methods

| Method | Params | Reply |
|---|---|---|
| `engine.execute` | `{command, params?, wait?}` | the command's JSON result (`docs/architecture.md` § 6). A background command (`updates.check`, `app.install`, `app.update`, `apps.updateAll`, `toolbox.update`, `icons.refresh`) replies when its job ends; with `wait: false` it replies `{job, pending: true}` at once |
| `engine.commands` | `{filter?}` | every command: `{id, label, params, background, enabled, disabledReason}`; `filter` matches id or label, case-insensitively |
| `jobs.list` | `{}` | `{jobs: [{id, command, label, done, total, apps, phase?, fraction?}]}` |
| `jobs.cancel` | `{job?}` | `{cancelled: n}`; without `job`, every running job. An unknown id is an error |
| `ui.get` | `{}` | the window: `ui` (the fields below), `notice` (the banner's text or null), `restartWanted`, `quitting`, `window {width, height, pixelsPerPoint}`, `services {notify, tray, popover}`, `language`, `jobs` |
| `ui.set` | any subset of `tab`, `search`, `searchOpen`, `availableFolded`, `selected`, `confirmUninstall`, `notice` | the same as `ui.get`, after the change |
| `app.quit` | `{}` | `{quitting: true}`; the window closes (to the tray when there is one, like the close button) |
| `methods` | `{}` | this list |

`ui.set` fields: `tab` is `"apps"` or `"settings"`; `search` the app list's filter text (at most
200 characters, no control characters); `searchOpen` shows the search field in place of the tabs;
`availableFolded` folds the "Available apps" panel; `selected` opens an app's page by catalog id
(`null` returns to the list); `confirmUninstall` opens the uninstall question for an app id
(`null` closes it); `notice` shows text in the banner (`null` dismisses it). Every field is
validated before the first one is applied, so an unknown field, a wrong type, an unknown app id or
an out-of-range value changes nothing and names the problem. These are exactly the fields of
`ui-egui/src/state.rs` plus the banner: there is no window state an agent cannot read and set.

Engine commands behave as from the CLI: the same ids, params and errors, the same verification
before anything is installed, and nothing ever elevates. The control channel adds no command of
its own.

## Headless server

`artcraft-toolbox-cli serve` opens the toolbox's data folder once (settings, inventory, cached
feeds, the same as the app's) and answers the envelope above on stdio, or on `127.0.0.1:<port>`
with `--port` (loopback only; every authenticated connection shares the session). TCP uses the
same first-frame `auth` and the same token options as the desktop app; stdio needs none, because
whoever holds the pipe already runs as the user. Implementation: `crates/automation/src/rpc.rs`
and `headless.rs`.

| Method | Params |
|---|---|
| `engine.execute` | `{command, params?, wait?}`, as above; with `wait: false`, `{job, pending}` |
| `engine.commands` | `{filter?}` |
| `jobs.list` / `jobs.cancel` | `{}` / `{job?}`. Every request first applies the jobs that have finished, so a status read after an install includes it without polling |
| `batch` | `{steps: [{command, params?, wait?}], stopOnError? (true)}` → `{completed, failed, results: [{ok, result} \| {ok, error}]}`; stops at the first error unless told not to |
| `methods` | the list above |

```sh
printf '%s\n' \
  '{"id":1,"method":"engine.execute","params":{"command":"updates.check","params":{"app":"photocraft"}}}' \
  '{"id":2,"method":"batch","params":{"steps":[{"command":"app.install","params":{"app":"photocraft"}},{"command":"app.status","params":{"app":"photocraft"}}]}}' \
  | artcraft-toolbox-cli serve
```

Don't run the desktop app and a headless server on the same data folder at the same time for
installs: each keeps its own inventory in memory (the headless server is for scripts while the
app isn't running, or for reading).

## Transport limits

Both TCP listeners enforce, before any dispatch:

- a 1 MiB maximum request line (the connection is closed after an oversized one);
- an 8 MiB maximum encoded reply, newline included: a larger result is replaced by one error
  carrying the request's `id` and saying the operation may have completed;
- at most 16 connections served at once per listener (an extra one gets one error line);
- a 30-second socket read and write timeout;
- at most 256 steps in a `batch` (and in MCP's `command_batch`), and an aggregate reply budget
  for the batch: the step that no longer fits ends the batch with an error, earlier results kept.

On stdio, an oversized or non-UTF-8 line gets one error reply with `"id": null`, the rest of
that line is skipped, and the session keeps serving. The MCP bridge bounds incoming replies the
same way and never resends a request after a transport failure (`docs/mcp.md`).

**Security.** The token is a bearer credential: whoever has it can do everything the user can do
without elevation (install, update, roll back, uninstall and launch the apps; update the toolbox;
change settings; drive and quit the window). Keep the token file private, don't log or commit
tokens, prefer the file over `--control-token` on shared machines, and keep the port on loopback:
the protocol is not encrypted. `SECURITY.md` has the threat model.
