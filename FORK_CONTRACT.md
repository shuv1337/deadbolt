# Shuvbolt fork contract

Shuvbolt is maintained in [`shuv1337/deadbolt`](https://github.com/shuv1337/deadbolt)
on **`shuv/dev`**, forked from [`anomalyco/deadbolt`](https://github.com/anomalyco/deadbolt)
at `9e76cf23ed08ba30631492cc994fb264c9a0c03b`. `dev` follows upstream; fork
changes belong on `shuv/dev`. See [UPSTREAM_SYNC.md](UPSTREAM_SYNC.md).

The first fork delta is `74b0257` (QUIC Retry only under handshake pressure:
ordinary multipath races can connect without unconditional Retry). Keep that
commit in the ancestry. Later deltas are the Shuvbolt product identity, disabled
self-update and publishing, and the executable `bun run check:fork` boundary.

| Class | Families and decisions |
| --- | --- |
| Canonical | Product/display name **Shuvbolt**, executable and command **`shuvbolt`**, UI executable **`shuvbolt-ui`**, **`Shuvbolt.app`**, Linux desktop ID and macOS app bundle/App Group **`dev.shuv.shuvbolt`** (with `.cli`, `.autofill`, and `.enclave` signing identifiers). Root Bun package `@shuvbolt/root`, tracing and telemetry names, native package/archive, browser display and first-run help, CLI/MCP display copy, and README installation instructions follow the fork name. |
| Compatibility: persistence and cryptography | Keep `DEADBOLT_*`, `$XDG_DATA_HOME/deadbolt`, `$XDG_CONFIG_HOME/deadbolt`, `deadbolt<suffix>` state/runtime directories, `daemon.{sock,lock}`, 8-hex BLAKE3 suffix, and legacy `agent.*` and SSH cleanup. Keep the v2 formats (`DBF2`, `DBLG`, `DBPK`, `DBLS`), v1 migration readers, recovery kit and key derivation, crypto domains (`deadbolt/v2 …`, `deadbolt-enclave/v1`, and legacy domains), and existing persisted keys and metadata. |
| Compatibility: wire and automation | Keep daemon and browser schemas, `deadbolt/sync/1`, `deadbolt/pair/3`, `deadbolt/send/1`, `deadbolt-pair:` and older wire readers, n0 DNS/pkarr and relay defaults, `deadbolt://`, `<concealed by deadbolt>`, and `https://deadbolt.pw/r` (`DEADBOLT_SEND_URL` can override it). Keep MCP `serverInfo.name = deadbolt` and `deadbolt_status`, `deadbolt_list_vaults`, `deadbolt_list_items`, `deadbolt_health_summary`, `deadbolt_broker_hint`; only the title and runnable command change. Preserve existing executable-tool interfaces such as `DEADBOLT_SECRET_SOCKET`. |
| Compatibility: browsers and internals | Keep `anomaly.deadbolt.browser`, Chromium ID `jhocodkhpifjndffenppfmdkmiolngal` and its manifest key, `data-deadbolt-ignore`, `deadbolt-site-icons`, `https://deadbolt.invalid/`, other extension storage/message/DOM identifiers, and generated protocol types. Keep `deadbolt-enclave`, `DeadboltAutoFill.appex` and its Swift module, internal `deadbolt-*` Rust package names, modules and symbols, and UniFFI/Swift binding names. |
| Provenance | Keep Git history and authorship, `UNLICENSED`, `anomalyco/rune` and its pin, attribution and font/icon notices. `docs/`, `changelog/`, and the extension credits preserve upstream designs, history, research, proposals, and release notes, not fork installation instructions. The GitHub repository slug remains `shuv1337/deadbolt`. |
| Incidental: retained upstream distribution and site | `site/`, `.github/release/`, `.github/workflows/release.yml`, `packaging/{homebrew,npm}`, upstream release/signing scripts (`package-npm.sh`, `release-homebrew.sh`, `verify-macos-release.sh`, `set-crate-version.sh`), and updater fixtures may still contain upstream URLs, package names, signing IDs and distribution examples. These are not fork distribution: fork release/site workflows are disabled at GitHub and guarded in YAML, the raw `site/public/install` exits before network access, and the fork updater has no release source. The package-manager and release parsers are retained for debug-only loopback fixtures. |
| Incidental: implementation and tests | Existing graphics, internal icon keys and asset filenames, thread names, test fixture data and paths, simulator names, extension corpus and e2e `--deadbolt` flag, development-only local test-support labels, health-data tooling labels, generated files, and explanatory source comments may retain upstream names. `AGENTS.md` retains upstream architecture/release notes; its fork section takes precedence here. The boundary checker names retired strings in its rejection rules. |

## Client transition

The fork preserves vault bytes and wire identities; switching is not a vault
conversion. Do not move or copy vault files just to rename the product. No
production vault is used for fork validation.

On macOS and Linux, close browsers and quit upstream UI/AutoFill clients.
Run `deadbolt debug daemon stop` **with the upstream binary** before starting
`shuvbolt`. The socket path is intentionally shared, not a way to run two
daemons on one vault. Run `shuvbolt browser install` before reopening browsers
on either platform: the shared `anomaly.deadbolt.browser` manifest contains
an absolute executable path and must point to the fork. On Linux also update
launcher entries.

On macOS upstream and fork code-signing identities differ: use a consistently
signed fork app, CLI and AutoFill build. App Group/Touch ID enrollment is
signing-identity dependent and may need re-enrollment; keep the vault
passphrase and recovery kit available. There is no automatic signing or
launcher migration.

The extension shares the upstream Chromium ID and cannot sit alongside it in
the same browser profile. Send still interoperates with the existing receiver;
this is not an independent hosted Send service. This contract does not promise
compatibility with arbitrary future upstream revisions.

## Releases

Build from source at the reviewed fork revision. `shuvbolt update` exits 1,
does not check the network or write update state, and never delegates to an
upstream package manager. The debug-only, non-default
`upstream-update-fixtures` feature is for loopback test servers, not distribution.
Managed apply in fixture builds additionally requires `DEADBOLT_FIXTURE_MANAGER`
to name the exact stand-in executable by absolute path.
There is no fork release signing identity configured for automated replacement.
