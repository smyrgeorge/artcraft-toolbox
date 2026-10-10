# Security Policy

ArtCraft Toolbox downloads executables from the internet and installs and runs them on the
user's machine. That makes it a software-supply-chain component: a flaw here can put someone
else's code on a user's computer. Treat release feeds, asset names, checksum files, archives,
disk images, settings files and automation requests as potentially malicious.

## Reporting a vulnerability

Please give the maintainers a reasonable opportunity to investigate and coordinate a fix before
publishing exploitable details. This repository does not yet document a private reporting
channel: contact the maintainers through the repository owner's channels, give only a minimal
non-exploitable summary in public, and ask for a private transfer method before sharing details.
Never open a public issue containing a working exploit, a secret or a malicious file.

A useful report includes the affected version or commit, OS and architecture, the component,
the impact and required attacker position (network, a compromised craft release, local user),
exact reproduction steps, and expected versus actual behaviour.

## Threat model

| Threat | Mitigation |
|---|---|
| A feed or redirect points requests at another host | `net::Policy`: HTTPS to the policy's hosts only (`api.github.com` for feeds), every redirect hop checked before it is contacted; asset URLs in feeds must start with `https://github.com/` (`feed::github::DOWNLOAD_PREFIX`); downloads add GitHub's asset CDN (`release-assets.githubusercontent.com`, `objects.githubusercontent.com`) and nothing else |
| A download is corrupted or swapped in transit or on a mirror | `SHA256SUMS.txt` is fetched before the download: no entry for the asset, no download. The file must end at exactly the size GitHub lists and match the SHA-256 before it is opened; otherwise it is deleted. A resumed download is checked whole |
| A craft's release itself is compromised | A checksum can't detect this; platform signatures partly can (`install::trust`). Before a version is activated: macOS `codesign --verify --deep` and a Gatekeeper assessment (`spctl`; the toolbox's installs carry no quarantine attribute, so this is the only one they get), Windows Authenticode. A broken signature is refused; a missing one is shown, not hidden. The developer that signed the installed version is recorded, and an update signed by anyone else (or by nobody) is refused, so a release from a stolen repository but without the developer's signing key can't replace a signed app. The first install trusts whoever signed it (roadmap › Open questions: pinning each craft's team in the catalog) |
| Files of an installed or kept version changed on disk | A copy found outside the toolbox is adopted only if its signature is intact; kept versions were checked when installed and are switched to without a new check (they live in the user's own data folder) |
| Malicious archives (zip-slip, symlinks, decompression bombs) | Entries with absolute, drive or `..` paths refused (not rewritten), symbolic links and duplicate names refused, entry count and unpacked size capped, no entry may write more than it declares (`install::extract_zip`) |
| Malicious disk images | Attached read-only, not browsed or auto-opened, at a private mount point; only the one `.app` with the expected bundle id is copied; always detached |
| Paths and names reaching shells or desktop files | App ids, names, bundle ids and versions are validated before they name a path (`install::AppInfo::validate`); desktop entry `Exec` quoted per the spec (control characters refused); the Windows shortcut script gets its paths in environment variables, never in the command line |
| Overwriting or deleting what the toolbox didn't install | Nothing that exists is replaced (an app installed by hand stays and the install fails until it is adopted); removal checks the recorded path still has the shape install gave it, and kept macOS bundles are removed only from the toolbox's own `versions.noindex` folder; Linux desktop entries are marked as the toolbox's and only those are removed |
| Secrets reaching the apps it opens | A launched craft gets `ARTCRAFT_TOOLBOX_MANAGED=1` and none of the toolbox's `ARTCRAFT_TOOLBOX_*` variables (the GitHub token among them); on macOS Launch Services starts it with a fresh environment |
| Hostile JSON, TOML, checksum files and asset names | Size caps on every input, no panics (`MAX_*` constants), excerpts only in error messages; hostile-input tests in every parser |
| A half-finished install or update breaks a working app | Verify, place beside the version in use, check the signature, then activate; a failure removes the placed version and leaves the one in use untouched (on macOS a failed swap moves the old bundle back). The replaced version is kept (`keepPrevious`) for rollback |
| Privilege escalation | Per-user installs need no admin rights; anything that would is an explicit user choice |
| The toolbox's own update (a compromised toolbox release replaces the toolbox) | The same pipeline as an app's: `SHA256SUMS.txt` first, size and SHA-256 checked, the package opened by the same hostile-input installers into the toolbox's data folder, and the platform signature checked and **required to match the running copy's developer** (a running copy whose own signature is broken refuses to update). The swap happens at the next start, old version kept in `self-update/previous/`; only a copy in a folder the user can write to updates itself, never with elevation (`toolbox_cmds`, `install::selfupdate`) |
| Automation surface (CLI, control channel, MCP) | Commands validate every param (`engine/tests/panic_hunt.rs`), and the servers add no command of their own, so an agent can do exactly what the user can without elevation. The control servers listen on loopback only, require a 256-bit bearer token in the first frame of every TCP connection (constant-time compare; the token file is created `0600`; a generated token goes to stderr, never the log), and bound every input before dispatch: 1 MiB requests, 8 MiB replies, 16 connections, 30 s socket timeouts, 256 batch steps, a 60 s reply deadline (`crates/automation/src/security.rs`, `budgets.rs`). `ui.set` validates every field before applying any. The MCP bridge accepts loopback addresses only and never resends a request whose outcome is unknown (`docs/control-protocol.md` › Transport limits) |
| Personal data in network requests | Generic User-Agent (`ArtCraft-Toolbox/<version>`), no identifiers |
| Token leakage | A GitHub token is used only when the user sets `ARTCRAFT_TOOLBOX_GITHUB_TOKEN`; it is sent only to `api.github.com` (never along a redirect to another host, tested in `crates/net/tests/client.rs`), never logged, never stored |
| Links in release notes (written by each craft's maintainers) | Only `http(s)` links open (`ui_egui::safe_url` filters the frame's open-URL commands); images in notes are never loaded |
| Hostile icons | Fetched only from `raw.githubusercontent.com` (no token), at most 512 KB and 1024 × 1024, decoded on a worker with a memory limit (`icons::decode_png`); anything else keeps the monogram |
| Hostile or corrupt local files (settings, inventory, cached feeds and icons, state) | Size-capped reads, parsed without panics; atomic replacement; an unreadable inventory is never overwritten; app ids are validated before they name a file |

## Scope

Reports may cover release-feed and catalog parsing, version and asset-name handling, checksum
verification, download and redirect handling, archive and disk-image extraction, install,
uninstall and rollback file handling (traversal, symlinks, overwrites), privilege handling,
self-update, the CLI, the control channel and the MCP server, and the toolbox's own CI and
release pipeline. Vulnerabilities in a Crafting App itself belong to that app's repository.
