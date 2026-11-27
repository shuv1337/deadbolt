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
REPOSITORY_GUARD = ["github.repository", "==", "'anomalyco/deadbolt'"]
STATUS_FUNCTIONS = {"always", "cancelled", "failure", "success"}


def job_field(body, key):
    match = re.search(rf"^    {key}:[ \t]*([^\n]*(?:\n {{6,}}[^\n]*)*)", body, re.M)
    return match[1].strip() if match else ""


def job_condition(body):
    condition = job_field(body, "if")
    if re.match(r"^[>|][-+]?\s+", condition):
        condition = re.sub(r"^[>|][-+]?\s+", "", condition)
    elif len(condition) >= 2 and condition[0] == "'" and condition[-1] == "'":
        condition = condition[1:-1].replace("''", "'")
    elif len(condition) >= 2 and condition[0] == '"' and condition[-1] == '"':
        try:
            condition = json.loads(condition)
        except json.JSONDecodeError:
            return ["always", "(", ")"]
    condition = condition.removeprefix("${{").removesuffix("}}").strip()
    # Keep quoted strings whole: parentheses and || inside them are not operators.
    return re.findall(r"'(?:[^']|'')*'|==|&&|\|\||[\w.-]+|[^\s]", condition)


def repository_guarded(tokens):
    if tokens[:3] != REPOSITORY_GUARD or (len(tokens) > 3 and tokens[3] != "&&"):
        return False
    depth = 0
    for token in tokens[3:]:
        if token == "(":
            depth += 1
        elif token == ")":
            depth -= 1
        if depth < 0 or (depth == 0 and token == "||"):
            return False
    return depth == 0


def job_needs(body):
    needs = job_field(body, "needs")
    if needs.startswith("[") and needs.endswith("]"):
        return set(re.findall(r"[\w-]+", needs[1:-1]))
    if needs.startswith("- "):
        return set(re.findall(r"^- ([\w-]+)$", "\n".join(line.strip() for line in needs.splitlines()), re.M))
    return {needs} if needs else set()


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

    def pattern(self, path, pattern, message):
        self.require(re.search(pattern, (ROOT / path).read_text()) is not None, path, message)

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
        # Check the block mapping used by these workflows. Reject unsupported
        # job declarations rather than silently missing a publishing root.
        text = (ROOT / path).read_text()
        jobs = re.split(r"^\w[\w-]*:", text.split("\njobs:\n", 1)[1], maxsplit=1, flags=re.M)[0]
        parts = re.split(r"^  ([^#\s][^\n]*)$", jobs, flags=re.M)
        dependencies, safe = {}, set()
        for header, body in zip(parts[1::2], parts[2::2]):
            job = re.fullmatch(r"([\w-]+):\s*(?:#.*)?", header)
            if not job:
                self.require(False, path, f"unsupported job declaration: {header}")
                continue
            name = job[1]
            self.require(name not in dependencies, path, f"duplicate job {name}")
            needs = dependencies[name] = job_needs(body)
            condition = job_condition(body)
            override = any(
                name.lower() in STATUS_FUNCTIONS and paren == "("
                for name, paren in zip(condition, condition[1:])
            )
            guarded = repository_guarded(condition)
            if not needs or name in required or override:
                self.require(
                    guarded,
                    path,
                    f"{name}: job must require the upstream repository without an OR bypass",
                )
            if not needs and guarded:
                safe.add(name)
        self.require(required <= dependencies.keys(), path, f"missing jobs: {', '.join(sorted(required - dependencies.keys()))}")
        # Every dependency must itself reach a guarded root. An unknown job
        # or a cycle will leave that chain unresolved.
        while True:
            connected = {job for job, needs in dependencies.items() if needs and needs <= safe} - safe
            if not connected:
                break
            safe.update(connected)
        for job in sorted(dependencies.keys() - safe):
            self.require(False, path, f"{job}: dependency chain does not reach a guarded root")


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
    paths = "crates/client/src/paths.rs"
    c.tokens(paths, '"deadbolt{suffix}"')
    c.pattern(
        paths,
        r'let data = var\("DEADBOLT_HOME"\)\s*\.unwrap_or_else\(\|\|\s*'
        r'xdg\("XDG_DATA_HOME", &\["\.local", "share"\]\)\s*\.join\("deadbolt"\)\s*\)',
        'data must use DEADBOLT_HOME or XDG_DATA_HOME joined with "deadbolt"',
    )
    for field, directory, filename in (
        ("socket", "runtime", "daemon.sock"),
        ("lock", "runtime", "daemon.lock"),
        ("log", "state", "daemon.log"),
    ):
        c.pattern(
            paths,
            rf'\b{field}:\s*{directory}\s*\.join\("{re.escape(filename)}"\)',
            f'{field} must use {directory}.join("{filename}")',
        )
    c.tokens("crates/cli/tests/cli/migrate_v1.rs", r'b"deadbolt join consent v2\0"')
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
    gate = rf'if\s+!cfg!\(\s*feature\s*=\s*"{feature}"\s*\)\s*\{{'
    c.tokens(
        "crates/cli/src/update/install.rs",
        '"status": "disabled"', '"reason": "no_fork_release_channel"', '"DEADBOLT_FIXTURE_MANAGER"',
    )
    c.pattern(
        "crates/cli/src/update/install.rs",
        rf"\bpub async fn run\([^)]*\)\s*->\s*Result<ExitCode>\s*\{{\s*{gate}",
        "CLI updater must gate on the fixture feature before doing any work",
    )
    c.pattern(
        "crates/cli/src/daemon/updates.rs",
        rf"\bpub async fn run\([^)]*\)\s*\{{\s*{gate}\s*return;",
        "daemon updater must return immediately without the fixture feature",
    )
    c.pattern(
        "crates/app/src/settings.rs",
        r"\bif\s+s\.daemon\.is_live\(\)\s*\{\s*return\s+None;",
        "Settings must hide notices from a live daemon",
    )
    c.guarded_jobs(".github/workflows/release.yml", {"plan"})
    c.guarded_jobs(".github/workflows/deploy-site.yml", {"build", "deploy"})
    installer = "site/public/install"
    start = (ROOT / installer).read_text().splitlines()[:3]
    c.require(
        len(start) == 3 and start[0] == "#!/bin/sh" and start[1].startswith("printf ")
        and "https://github.com/shuv1337/shuvbolt" in start[1] and start[2] == "exit 1",
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
