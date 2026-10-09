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
| A craft's release itself is compromised | A checksum can't detect this. Platform signatures can: Developer ID and notarization on macOS, Authenticode on Windows where present (M3). Report unsigned builds instead of hiding it. Until M3, note that apps the toolbox installs carry no quarantine attribute, so macOS doesn't run a Gatekeeper assessment on their first launch |
| Malicious archives (zip-slip, symlinks, decompression bombs) | Entries with absolute, drive or `..` paths refused (not rewritten), symbolic links and duplicate names refused, entry count and unpacked size capped, no entry may write more than it declares (`install::extract_zip`) |
| Malicious disk images | Attached read-only, not browsed or auto-opened, at a private mount point; only the one `.app` with the expected bundle id is copied; always detached |
| Paths and names reaching shells or desktop files | App ids, names, bundle ids and versions are validated before they name a path (`install::AppInfo::validate`); desktop entry `Exec` quoted per the spec (control characters refused); the Windows shortcut script gets its paths in environment variables, never in the command line |
| Overwriting or deleting what the toolbox didn't install | Nothing that exists is replaced (an app installed by hand stays and the install fails); uninstall checks the recorded path still has the shape install gave it; Linux desktop entries are marked as the toolbox's and only those are removed |
| Hostile JSON, TOML, checksum files and asset names | Size caps on every input, no panics (`MAX_*` constants), excerpts only in error messages; hostile-input tests in every parser |
| A half-finished install breaks a working app | Verify, stage under a hidden name beside the target, then rename into place; a failed install removes its staging copy. M3: the previous version is kept until the new one works |
| Privilege escalation | Per-user installs need no admin rights; anything that would is an explicit user choice |
| Automation surface (CLI now; control channel and MCP later) | Commands validate every param (`engine/tests/panic_hunt.rs`); the control channel will require a bearer token and localhost, like PhotoCraft's |
| Personal data in network requests | Generic User-Agent (`ArtCraft-Toolbox/<version>`), no identifiers |
| Token leakage | A GitHub token is used only when the user sets `ARTCRAFT_TOOLBOX_GITHUB_TOKEN`; it is sent only to `api.github.com` (never along a redirect to another host, tested in `crates/net/tests/client.rs`), never logged, never stored |
| Links in release notes (written by each craft's maintainers) | Only `http(s)` links open (`ui_egui::safe_url` filters the frame's open-URL commands); images in notes are never loaded |
| Hostile icons | Fetched only from `raw.githubusercontent.com` (no token), at most 512 KB and 1024 × 1024, decoded on a worker with a memory limit (`icons::decode_png`); anything else keeps the monogram |
| Hostile or corrupt local files (settings, inventory, cached feeds and icons, state) | Size-capped reads, parsed without panics; atomic replacement; an unreadable inventory is never overwritten; app ids are validated before they name a file |

## Scope

Reports may cover release-feed and catalog parsing, version and asset-name handling, checksum
verification, download and redirect handling, archive and disk-image extraction, install,
uninstall and rollback file handling (traversal, symlinks, overwrites), privilege handling,
self-update, the CLI and future automation endpoints, and the toolbox's own CI and release
pipeline. Vulnerabilities in a Crafting App itself belong to that app's repository.
