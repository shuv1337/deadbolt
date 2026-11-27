# Shuvbolt

Shuvbolt is an alpha, local-first password manager in Rust, forked from
[anomalyco/deadbolt](https://github.com/anomalyco/deadbolt). Your devices sync
directly over [iroh](https://www.iroh.computer); a new device joins by pairing
from one you already have. See the [fork contract](FORK_CONTRACT.md) and
[upstream sync instructions](UPSTREAM_SYNC.md).

## Build and install

Shuvbolt has no hosted installer, release channel, or self-update. Clone
`shuv1337/deadbolt`, check out `shuv/dev`, and build with stable Rust:

```sh
cargo build --release --locked -p deadbolt-cli --bin shuvbolt
# Optional desktop app:
cargo build --release --locked -p deadbolt-app --bin shuvbolt-ui
install -d ~/.local/bin
install -m 755 target/release/shuvbolt ~/.local/bin/
# On Linux, install the desktop binary beside the CLI:
install -m 755 target/release/shuvbolt-ui ~/.local/bin/
```

Put `~/.local/bin` on `PATH`. Run `shuvbolt app` to open the installed app on
Linux; `shuvbolt desktop install` registers it with the launcher.
For a macOS bundle, use `crates/app/scripts/bundle.sh` on macOS; production
Touch ID/AutoFill require fork signing and provisioning. See
[the transition notes](FORK_CONTRACT.md#client-transition) before switching
an existing installation.

Create a vault with `shuvbolt init` or choose **Create a vault** in the app.
To pair another device, run `shuvbolt join` there, then `shuvbolt approve <code>`
on an enrolled device. A code works once, for five minutes.

## Browser extension

Build with Bun from the same checkout and register the native host using the
installed CLI:

```sh
(cd extension && bun install --frozen-lockfile && bun run build)
shuvbolt browser install
```

In a supported Chromium-based browser, open `chrome://extensions`, turn on
**Developer mode**, and **Load unpacked** from `extension/dist`. The fork and
upstream share the same extension ID; use one per profile. Rebuild and reload
the unpacked extension after updating your checkout.

## Secrets and Send

The secret URI scheme is still `deadbolt://`. Use `shuvbolt run`, `inject`, or
`read` with these references:

```sh
shuvbolt run --env GH_TOKEN=deadbolt://personal/GitHub/token -- gh api user
shuvbolt inject -i config.tpl -o config.json
```

`shuvbolt activity` lists resolves, never values. Lock the vault when away.
`shuvbolt send ./report.pdf` creates a one-time link. The existing
`https://deadbolt.pw/r` receiver remains the default; `shuvbolt receive` also
works without a vault or daemon.

## Reference

- [Vault design](docs/design.md) and [simulation](docs/simulation.md)
- [Desktop app](docs/app.md), [Autofill](docs/autofill.md), and [secrets](docs/secrets.md)
- [Send](docs/proposals/send.md) and [upstream release design](docs/releasing.md)

The reference docs describe upstream design and may show upstream commands.
Do not use upstream distribution channels to install or update Shuvbolt.
[Report fork issues here](https://github.com/shuv1337/deadbolt/issues).
