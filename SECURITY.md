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
| A feed or redirect points requests at another host | `net::Policy`: HTTPS to the policy's hosts only (`api.github.com` for feeds), every redirect hop checked before it is contacted; asset URLs in feeds must start with `https://github.com/` (`feed::github::DOWNLOAD_PREFIX`); downloads (M2) add GitHub's asset CDN and nothing else |
| A download is corrupted or swapped in transit or on a mirror | SHA-256 against the release's `SHA256SUMS.txt` before the file is opened (M2); no checksum entry, no install |
| A craft's release itself is compromised | A checksum can't detect this. Platform signatures can: Developer ID and notarization on macOS, Authenticode on Windows where present (M3). Report unsigned builds instead of hiding it |
| Malicious archives (zip-slip, symlinks, decompression bombs) | Paths confined to the staging dir, symlinks refused, entry count and total size capped (M2) |
| Hostile JSON, TOML, checksum files and asset names | Size caps on every input, no panics (`MAX_*` constants), excerpts only in error messages; hostile-input tests in every parser |
| A half-finished install breaks a working app | Stage, verify, then rename into place; the previous version is kept until the new one works |
| Privilege escalation | Per-user installs need no admin rights; anything that would is an explicit user choice |
| Automation surface (CLI now; control channel and MCP later) | Commands validate every param (`engine/tests/panic_hunt.rs`); the control channel will require a bearer token and localhost, like PhotoCraft's |
| Personal data in network requests | Generic User-Agent (`ArtCraft-Toolbox/<version>`), no identifiers |
| Token leakage | A GitHub token is used only when the user sets `ARTCRAFT_TOOLBOX_GITHUB_TOKEN`; it is sent only to `api.github.com` (never along a redirect to another host, tested in `crates/net/tests/client.rs`), never logged, never stored |
| Hostile or corrupt local files (settings, inventory, cached feeds) | Size-capped reads, parsed without panics; atomic replacement; an unreadable inventory is never overwritten; app ids are validated before they name a file |

## Scope

Reports may cover release-feed and catalog parsing, version and asset-name handling, checksum
verification, download and redirect handling, archive and disk-image extraction, install,
uninstall and rollback file handling (traversal, symlinks, overwrites), privilege handling,
self-update, the CLI and future automation endpoints, and the toolbox's own CI and release
pipeline. Vulnerabilities in a Crafting App itself belong to that app's repository.
