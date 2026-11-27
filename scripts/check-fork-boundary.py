#!/usr/bin/env python3
"""Check the Shuvbolt shipping boundary without building the workspace."""

import argparse
import base64
import hashlib
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
APP_ID = "dev.shuv.shuvbolt"
EXTENSION_ID = "jhocodkhpifjndffenppfmdkmiolngal"
WORKFLOW_GUARD = "github.repository == 'anomalyco/deadbolt'"


class Check:
    def __init__(self):
        self.errors = []

    def require(self, ok, path, message):
        if not ok:
            self.errors.append(f"{path}: {message}")

    def tokens(self, path, *required, forbidden=()):
        text = (ROOT / path).read_text()
        for token in required:
            self.require(token in text, path, f"missing {token!r}")
        for token in forbidden:
            self.require(token not in text, path, f"retired identity {token!r}")

    def binary(self, path, package, expected):
        manifest = tomllib.loads((ROOT / path).read_text())
        self.require(manifest["package"]["name"] == package, path, f"keep package {package}")
        self.require(
            [target["name"] for target in manifest["bin"]] == [expected],
            path,
            f"expected only the {expected} executable",
        )

    def extension(self, path, key, *, built=False):
        manifest = json.loads(path.read_text())
        for field in ("name", "short_name"):
            self.require(manifest.get(field) == "Shuvbolt", path, f"{field} must be Shuvbolt")
        self.require(
            manifest["action"].get("default_title") == "Shuvbolt",
            path,
            "action.default_title must be Shuvbolt",
        )
        self.require("update_url" not in manifest, path, "must not set update_url")
        if built:
            self.require("nativeMessaging" in manifest["permissions"], path, "missing nativeMessaging")
            self.require(manifest.get("key") == key, path, "must use the checked-in manifest key")

    def guarded_jobs(self, path, required):
        # Job keys are the two-space entries under `jobs:`; job conditions are
        # four-space fields, including YAML folded conditions on following lines.
        jobs = (ROOT / path).read_text().split("\njobs:\n", 1)[1]
        parts = re.split(r"^  ([\w-]+):[^\S\n]*$", jobs, flags=re.M)
        seen = set()
        for name, body in zip(parts[1::2], parts[2::2]):
            match = re.search(r"^    if: (.*(?:\n {6,}[^\n]+)*)", body, re.M)
            condition = match[1] if match else ""
            if name in required or re.search(r"\b(?:always|cancelled)\s*\(", condition):
                seen.add(name)
                predicate = " ".join(condition.removeprefix(">-").split()).removeprefix("${{ ")
                self.require(
                    predicate == WORKFLOW_GUARD or predicate.startswith(WORKFLOW_GUARD + " && "),
                    path,
                    f"{name}: repository guard must come first in the job condition",
                )
        self.require(required <= seen, path, f"missing jobs: {', '.join(sorted(required - seen))}")


def canonical(c):
    c.binary("crates/cli/Cargo.toml", "deadbolt-cli", "shuvbolt")
    c.binary("crates/app/Cargo.toml", "deadbolt-app", "shuvbolt-ui")
    c.tokens("package.json", '"@shuvbolt/root"', "--bin shuvbolt --")
    c.tokens("crates/cli/src/main.rs", 'name = "shuvbolt"', '"warn,shuvbolt=info"')
    c.tokens(
        "crates/cli/src/app.rs", 'UI_PROGRAM: &str = "shuvbolt-ui"', f'BUNDLE_ID: &str = "{APP_ID}"',
        "Shuvbolt.app", "Contents/MacOS/Shuvbolt",
        forbidden=("anomalyco/tap/deadbolt", "deadbolt.pw/install"),
    )
    c.tokens("crates/app/src/binary.rs", '"Helpers/shuvbolt"', 'dir.join("shuvbolt")')
    c.tokens("crates/cli/src/daemon/caller.rs", '"MacOS/Shuvbolt"', 'CallerClass::App => return "Shuvbolt"')
    c.tokens("crates/cli/src/desktop.rs", f'ID: &str = "{APP_ID}"', "Name=Shuvbolt")
    c.tokens("crates/app/src/lib.rs", f'APP_ID: &str = "{APP_ID}"', 'title: "Shuvbolt"')
    c.tokens("crates/app/src/panel.rs", 'title: "Shuvbolt"')
    c.tokens(
        "crates/client/src/peer.rs", f'APP_ID: &str = "{APP_ID}"',
        f'DAEMON_ID: &str = "{APP_ID}.cli"', f'AUTOFILL_ID: &str = "{APP_ID}.autofill"',
        f'APP_GROUP_SUFFIX: &str = "{APP_ID}"',
    )
    c.tokens("crates/app/scripts/bundle.sh", "Shuvbolt.app", APP_ID, "--bin shuvbolt --bin shuvbolt-ui")
    c.tokens("crates/app/scripts/build-autofill.sh", f"{APP_ID}.autofill", "${team_id}." + APP_ID)
    c.tokens("crates/app/autofill/Client.swift", '"Helpers/shuvbolt"')
    c.tokens("crates/app/autofill/Info.plist.in", f"<string>{APP_ID}.autofill</string>")
    c.tokens("crates/enclave/swift/Info.plist", f"<string>{APP_ID}.enclave</string>")
    for page in ("popup", "menu"):
        c.tokens(f"extension/static/{page}.html", "<title>Shuvbolt</title>", forbidden=("<title>Deadbolt</title>",))


def compatibility(c):
    c.tokens(
        "crates/client/src/paths.rs",
        '"DEADBOLT_HOME"', '.join("deadbolt")', '"deadbolt{suffix}"', '"daemon.sock"',
    )
    c.tokens("crates/net/src/lib.rs", 'b"deadbolt/sync/1"', 'b"deadbolt/pair/3"')
    c.tokens("crates/send/src/lib.rs", 'b"deadbolt/send/1"')
    c.tokens("crates/proto/src/browser.rs", 'NATIVE_HOST: &str = "anomaly.deadbolt.browser"')
    c.tokens("crates/cli/src/browser.rs", f'EXTENSION_ID: &str = "{EXTENSION_ID}"')
    c.tokens("crates/proto/src/daemon/reference.rs", 'SCHEME: &str = "deadbolt://"')
    c.tokens(
        "crates/core/src/v2/crypto.rs",
        *(f'"deadbolt/v2 {label}"' for label in (
            "vault id", "frame id", "recovery sign", "recovery kem", "device sign",
            "device kem", "device net", "epoch data", "epoch back", "epoch state",
            "epoch commit", "x25519 wrap", "recovery wrap kem", "wrap",
        )),
    )
    c.tokens("crates/cli/src/disk.rs", 'b"deadbolt/v2 passphrase slot"', 'b"deadbolt/v2 biometric slot"')
    c.tokens("crates/cli/src/mcp.rs", '"name": "deadbolt"', '"title": "Shuvbolt"')
    c.tokens(
        "crates/cli/src/mcp/tools.rs",
        *(f'name: "deadbolt_{tool}"' for tool in (
            "status", "list_vaults", "list_items", "health_summary", "broker_hint",
        )),
    )
    c.tokens(
        "crates/net/src/node.rs",
        "const RETRY_ABOVE_HANDSHAKES: usize = MAX_INBOUND_HANDSHAKES / 2;",
        "!validated && MAX_INBOUND_HANDSHAKES - free_handshakes >= RETRY_ABOVE_HANDSHAKES",
        "if should_retry(incoming.remote_addr_validated(), handshakes.available_permits())",
    )


def release_boundary(c):
    feature = "upstream-update-fixtures"
    cargo = "crates/cli/Cargo.toml"
    features = tomllib.loads((ROOT / cargo).read_text())["features"]
    c.require(feature in features and feature not in features["default"], cargo, "fixture must be opt-in")
    c.tokens(
        "crates/cli/src/main.rs",
        f'#[cfg(all(feature = "{feature}", not(debug_assertions)))]',
        f'compile_error!("{feature} is debug-only");',
        f'let notice = cfg!(feature = "{feature}")',
    )
    c.tokens(
        "crates/cli/src/update/service.rs",
        "SERVICE: Option<&str> = None;", "REGISTRY: Option<&str> = None;",
        f'cfg!(all(debug_assertions, feature = "{feature}"))',
        forbidden=("anoma.ly/release", "registry.npmjs.org"),
    )
    c.tokens("crates/cli/src/update/method.rs", forbidden=("anomalyco/tap/deadbolt",))
    c.tokens(
        "crates/app/update-requirement.txt", f'identifier "{APP_ID}"', "SHUVBOLT_UNSET_TEAM_ID",
        forbidden=("5NZ4Q7NXJ4", '"anomaly.deadbolt"'),
    )
    gate = f'if !cfg!(feature = "{feature}") {{'
    c.tokens(
        "crates/cli/src/update/install.rs",
        f"pub async fn run(paths: &Paths, args: UpdateArgs) -> Result<ExitCode> {{\n    {gate}",
        '"status": "disabled"', '"reason": "no_fork_release_channel"',
    )
    c.tokens("crates/cli/src/daemon/updates.rs", f"pub async fn run(daemon: Arc<Daemon>) {{\n    {gate}\n        return;")
    c.tokens("crates/app/src/settings.rs", "if s.daemon.is_live() {\n        return None;")
    c.guarded_jobs(".github/workflows/release.yml", {"plan"})
    c.guarded_jobs(".github/workflows/deploy-site.yml", {"build", "deploy"})
    installer = "site/public/install"
    start = (ROOT / installer).read_text().splitlines()[:3]
    c.require(
        len(start) == 3 and start[0] == "#!/bin/sh" and start[1].startswith("printf ")
        and "https://github.com/shuv1337/deadbolt" in start[1] and start[2] == "exit 1",
        installer, "must refuse installation before the upstream script runs",
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", type=Path, help="also check the built browser extension")
    args = parser.parse_args()
    c = Check()
    try:
        canonical(c)
        compatibility(c)
        release_boundary(c)
        for path in ("FORK_CONTRACT.md", "UPSTREAM_SYNC.md"):
            c.tokens(path, "anomalyco/deadbolt", "9e76cf2", "74b0257")
        c.tokens("Cargo.toml", 'license = "UNLICENSED"')
        c.tokens(
            "README.md", "Shuvbolt", "`shuvbolt", "anomalyco/deadbolt",
            forbidden=("deadbolt.pw/install", "anomalyco/tap/deadbolt", "@deadbolt/cli"),
        )
        key_path = "extension/manifest-key.txt"
        key = (ROOT / key_path).read_text().strip()
        digest = hashlib.sha256(base64.b64decode(key, validate=True)).hexdigest()
        extension_id = "".join(chr(ord("a") + int(digit, 16)) for digit in digest[:32])
        c.require(extension_id == EXTENSION_ID, key_path, f"key must derive {EXTENSION_ID}")
        c.extension(ROOT / "extension/static/manifest.json", key)
        if args.dist is not None:
            c.extension(args.dist / "manifest.json", key, built=True)
    except (OSError, ValueError, KeyError, TypeError, IndexError) as error:
        c.errors.append(str(error))
    if c.errors:
        for error in c.errors:
            print(f"fork boundary: {error}", file=sys.stderr)
        return 1
    print("ok: Shuvbolt fork boundary" + (f" (including {args.dist})" if args.dist else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
