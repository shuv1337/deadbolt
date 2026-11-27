use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::*;

const DEV: &str = "/release/deadbolt/cli/dev.json";
const DIST_TAGS: &str = "/release/-/package/@deadbolt/cli/dist-tags";

/// The `cli` manifest for dev at `version`, `sequence`.
fn manifest(version: &str, sequence: u64) -> Vec<u8> {
    let sha = "a".repeat(64);
    let target = serde_json::json!({
        "url": format!("/release/deadbolt/files/{sha}/cli.bin"), "size": 1, "sha256": sha
    });
    serde_json::json!({
        "schema": 1, "app": "deadbolt", "artifact": "cli", "channel": "dev",
        "sequence": sequence, "published_at": "2026-11-01T00:00:00Z",
        "release": {
            "version": version, "rollout_percent": 100,
            "targets": {
                "macos-arm64": target, "macos-x64": target,
                "linux-arm64": target, "linux-x64": target,
                "linux-arm64-musl": target, "linux-x64-musl": target
            },
            "distributions": [{ "kind": "direct" }]
        }
    })
    .to_string()
    .into_bytes()
}

/// The update the daemon's status reply carries, as the app and the
/// terminal notice read it.
fn update(device: &Device) -> Option<deadbolt_proto::daemon::UpdateInfo> {
    let paths = deadbolt_client::Paths::from_vars(|name| match name {
        "DEADBOLT_HOME" => Some(device.path("home").into()),
        "XDG_RUNTIME_DIR" => Some(device.path("run").into()),
        "XDG_STATE_HOME" => Some(device.path("state").into()),
        "XDG_CONFIG_HOME" => Some(device.path("config").into()),
        _ => None,
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime
        .block_on(deadbolt_client::status(&paths))
        .expect("the daemon's status")
        .update
}

/// The update the daemon reports, if it answered at all: the bounded wait
/// an opted-out daemon is watched through, without a fixed sleep.
fn reported_update(device: &Device) -> Option<Option<deadbolt_proto::daemon::UpdateInfo>> {
    let paths = deadbolt_client::Paths::from_vars(|name| match name {
        "DEADBOLT_HOME" => Some(device.path("home").into()),
        "XDG_RUNTIME_DIR" => Some(device.path("run").into()),
        "XDG_STATE_HOME" => Some(device.path("state").into()),
        "XDG_CONFIG_HOME" => Some(device.path("config").into()),
        _ => None,
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime
        .block_on(deadbolt_client::status(&paths))
        .map(|status| status.update)
}

#[test]
fn the_daemon_tells_status_and_watchers_when_an_update_is_out() {
    let a = Device::new();
    let files = Arc::new(Mutex::new(HashMap::from([(
        DEV.to_string(),
        manifest(env!("CARGO_PKG_VERSION"), 1),
    )])));
    let base = release_service(files.clone());
    let mut watch = a
        .cmd(&["debug", "watch"])
        .env("DEADBOLT_RELEASE_URL", &base)
        .env("DEADBOLT_UPDATE_CHECK_CHANNEL", "dev")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = watch.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let next = || -> serde_json::Value {
        loop {
            let line = rx.recv_timeout(Duration::from_secs(30)).expect("a change");
            let change: serde_json::Value = serde_json::from_str(&line).unwrap();
            if change != serde_json::json!({"change": "activity"}) {
                return change;
            }
        }
    };
    assert_eq!(next(), serde_json::json!({"change": "watching"}));
    assert_eq!(update(&a), None);

    // A new release: the check (every second here) finds it, and only says so.
    files
        .lock()
        .unwrap()
        .insert(DEV.to_string(), manifest("0.0.0-dev-9999", 2));
    assert_eq!(next(), serde_json::json!({"change": "update"}));
    let found = update(&a).expect("an update");
    assert_eq!(found.version, "0.0.0-dev-9999");
    assert_eq!(found.method, "standalone");
    assert_eq!(a.state(), "not set up");

    // Back to this build (a rollback): nothing to say any more.
    files
        .lock()
        .unwrap()
        .insert(DEV.to_string(), manifest(env!("CARGO_PKG_VERSION"), 3));
    assert_eq!(next(), serde_json::json!({"change": "update"}));
    assert_eq!(update(&a), None);
    a.run(&["debug", "daemon", "stop"]);
    assert!(wait(&mut watch, Duration::from_secs(10)).success());

    // Opted out, the daemon never asks. Wait for it to answer, then watch
    // through a check interval (a second here) without an update.
    let b = Device::new();
    files
        .lock()
        .unwrap()
        .insert(DEV.to_string(), manifest("0.0.0-dev-9999", 4));
    let out = b.output(
        b.cmd(&["debug", "whoami"])
            .env("DEADBOLT_RELEASE_URL", &base)
            .env("DEADBOLT_UPDATE_CHECK_CHANNEL", "dev")
            .env("DEADBOLT_NO_UPDATE_CHECK", "1"),
    );
    assert!(out.status.success());
    let deadline = Instant::now() + Duration::from_secs(10);
    while reported_update(&b).is_none() {
        assert!(Instant::now() < deadline, "the daemon never answered");
        std::thread::sleep(Duration::from_millis(50));
    }
    let watch = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < watch {
        assert_eq!(reported_update(&b), Some(None));
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Copies this deadbolt to `to`, with the `deadbolt-enclave` beside it that
/// a macOS build needs to check its daemon.
fn copy_deadbolt(to: &Path) {
    std::fs::copy(BIN, to).unwrap();
    let enclave = Path::new(BIN).with_file_name("deadbolt-enclave");
    if enclave.exists() {
        std::fs::copy(&enclave, to.with_file_name("deadbolt-enclave")).unwrap();
    }
}

/// A bun global install laid out under `root` (`bun/install/global/…`,
/// linked from `bun/bin/deadbolt`), with a fake `bun` beside that link that
/// lists `@deadbolt/cli`, logs its arguments to `root/bun.log` and on
/// `install` puts a new file in place, as bun does: a stand-in whose
/// `--version` says `deadbolt <installs>` and which otherwise runs a copy of
/// this deadbolt, so its daemon starts. Or it fails, when `fails`. Returns
/// the user's link and the installed `deadbolt`.
fn bun_install(root: &Path, installs: &str, fails: bool) -> (PathBuf, PathBuf) {
    let modules = root.join("bun/install/global/node_modules");
    let package = modules.join("@deadbolt/cli-linux-x64");
    let exe = package.join("bin/deadbolt");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    // A copy, never a hard link: macOS can name any of an inode's links as
    // a running program's path, so other tests' daemons would seem to run it.
    if !exe.exists() {
        copy_deadbolt(&exe);
    }
    std::fs::write(
        package.join("package.json"),
        r#"{"name":"@deadbolt/cli-linux-x64","version":"0.0.1"}"#,
    )
    .unwrap();
    let wrapper = modules.join("@deadbolt/cli");
    std::fs::create_dir_all(wrapper.join("bin")).unwrap();
    std::fs::write(
        wrapper.join("package.json"),
        r#"{"name":"@deadbolt/cli","bin":{"deadbolt":"./bin/deadbolt"}}"#,
    )
    .unwrap();
    std::os::unix::fs::symlink(
        "../../cli-linux-x64/bin/deadbolt",
        wrapper.join("bin/deadbolt"),
    )
    .ok();
    let bin = root.join("bun/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let link = bin.join("deadbolt");
    std::os::unix::fs::symlink(wrapper.join("bin/deadbolt"), &link).ok();
    let log = root.join("bun.log");
    let install = if fails {
        "echo 'resolving @deadbolt/cli'; echo 'error: GET https://registry.npmjs.org/@deadbolt%2fcli - 503' >&2; exit 1"
            .to_string()
    } else {
        let copy = root.join("new-deadbolt");
        if !copy.exists() {
            copy_deadbolt(&copy);
        }
        format!(
            "/bin/rm -f '{exe}'; printf '#!/bin/sh\\nif [ \"$1\" = --version ]; then echo \"deadbolt {installs} (dev)\"; exit; fi\\nexec \"{copy}\" \"$@\"\\n' > '{exe}'; /bin/chmod 755 '{exe}'",
            exe = exe.display(),
            copy = copy.display()
        )
    };
    let bun = bin.join("bun");
    std::fs::write(
        &bun,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\ncase \"$1\" in\n  pm) echo '/x node_modules (2 installed)'; echo '├── @deadbolt/cli@0.0.1'; exit 0 ;;\n  install) {install} ;;\nesac\n",
            log = log.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bun, std::fs::Permissions::from_mode(0o755)).unwrap();
    (link, exe)
}

#[test]
fn update_runs_bun_for_a_bun_install_pinned_to_what_npm_serves() {
    let a = Device::new();
    let files = Arc::new(Mutex::new(HashMap::from([
        (DEV.to_string(), manifest("0.0.0-dev-9999", 1)),
        (DIST_TAGS.to_string(), br#"{"dev":"0.0.1"}"#.to_vec()),
    ])));
    let base = release_service(files.clone());
    let root = a.path("user");
    let (link, exe) = bun_install(&root, "0.0.0-dev-9999", true);
    let empty = a.path("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let with_manager = |args: &[&str], stand_in: Option<&Path>| {
        let mut cmd = a.cmd_of(&link, args);
        cmd.env("DEADBOLT_RELEASE_URL", &base)
            .env("DEADBOLT_NPM_REGISTRY_URL", &base)
            .env("PATH", &empty)
            .env("HOME", &root)
            .env_remove("BUN_INSTALL");
        if let Some(stand_in) = stand_in {
            cmd.env("DEADBOLT_FIXTURE_MANAGER", stand_in);
        }
        let out = cmd.output().unwrap();
        eprintln!(
            "$ {args:?}\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        out
    };
    let fixture = root.join("bun/bin/bun");
    let update = |args: &[&str]| with_manager(args, Some(&fixture));
    let log = || std::fs::read_to_string(root.join("bun.log")).unwrap_or_default();

    // npm still serves the old build: nothing bun could install yet.
    let out = update(&["update", "--check", "--channel", "dev"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("bun doesn't offer"), "{stdout}");
    assert!(log().contains("pm ls -g"), "{}", log());
    assert!(!log().contains("install"));

    // npm serves the wrapper, but not yet this system's platform package.
    files.lock().unwrap().insert(
        DIST_TAGS.to_string(),
        br#"{"dev":"0.0.0-dev-9999"}"#.to_vec(),
    );
    let out = update(&["update", "--check", "--channel", "dev"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("bun doesn't offer"), "{stdout}");

    for target in [
        "linux-x64",
        "linux-arm64",
        "linux-x64-musl",
        "linux-arm64-musl",
        "darwin-arm64",
        "darwin-x64",
    ] {
        let name = format!("@deadbolt/cli-{target}");
        files.lock().unwrap().insert(
            format!("/release/{name}/0.0.0-dev-9999"),
            serde_json::json!({"name": name, "version": "0.0.0-dev-9999"})
                .to_string()
                .into_bytes(),
        );
    }
    let out = update(&["update", "--check", "--channel", "dev"]);
    assert_eq!(out.status.code(), Some(10));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("installed with bun"), "{stdout}");
    assert!(
        stdout.contains(
            "shuvbolt update runs: bun install --global --trust @deadbolt/cli@0.0.0-dev-9999"
        ),
        "{stdout}"
    );
    assert!(!log().contains("install --global"));

    let apply = ["update", "--channel", "dev", "--yes"];
    for (stand_in, refusal) in [(None, "requires"), (Some(Path::new(BIN)), "refused")] {
        let out = with_manager(&apply, stand_in);
        assert_eq!(out.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(refusal) && stderr.contains("DEADBOLT_FIXTURE_MANAGER"),
            "{stderr}"
        );
        assert!(!log().contains("install --global"));
    }

    // bun fails: its output's end, the command, and what to do.
    let out = update(&apply);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bun couldn't install deadbolt 0.0.0-dev-9999"),
        "{stderr}"
    );
    assert!(stderr.contains("503"), "{stderr}");
    assert!(
        stderr.contains("It ran: bun install --global --trust @deadbolt/cli@0.0.0-dev-9999"),
        "{stderr}"
    );

    // bun works: the new deadbolt answers through the user's link, and the
    // daemon, which ran the old one, comes back on it. bun deletes the file
    // this `deadbolt update` runs from, which mustn't make the new daemon
    // look like it never started.
    bun_install(&root, "0.0.0-dev-9999", false);
    let pid = |status: &str| {
        status
            .strip_prefix("running pid ")
            .and_then(|l| l.split_whitespace().next())
            .map(str::to_string)
    };
    // Through the link: `debug whoami` starts the daemon (or, on macOS, only
    // passes the new deadbolt's daemon check), then `debug daemon` names it.
    let daemon = || {
        assert!(update(&["debug", "whoami"]).status.success());
        String::from_utf8_lossy(&update(&["debug", "daemon"]).stdout).to_string()
    };
    let before = daemon();
    let old = pid(&before).unwrap_or_else(|| panic!("{before}"));
    let out = update(&["update", "--channel", "dev", "--yes"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stdout.contains("Updated"), "{stdout}");
    assert!(stdout.contains("Restarted the daemon."), "{stdout}");
    assert!(!stderr.contains("warning"), "{stderr}");
    let after = daemon();
    let new = pid(&after).unwrap_or_else(|| panic!("{after}"));
    assert_ne!(new, old, "{after}");
    a.run(&["debug", "daemon", "stop"]);
    let ran = log();
    let install = ran.lines().find(|l| l.starts_with("install")).unwrap();
    let args: Vec<&str> = install.split(' ').collect();
    assert_eq!(args[..4], ["install", "--global", "--trust", "--cache-dir"]);
    assert_eq!(args[5], "@deadbolt/cli@0.0.0-dev-9999");
    assert!(!Path::new(args[4]).exists(), "bun's cache is gone");
    assert!(
        std::fs::read_to_string(&exe)
            .unwrap()
            .contains("0.0.0-dev-9999")
    );
}

#[test]
fn update_check_json_emits_structured_report_and_honors_install_receipt() {
    use std::os::unix::fs::PermissionsExt;

    let a = Device::new();
    let bin = a.path("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let exe = bin.join("shuvbolt");
    copy_deadbolt(&exe);
    let root = bin.canonicalize().unwrap();
    let receipt = bin.join("install.json");
    std::fs::write(
        &receipt,
        serde_json::json!({
            "app": "deadbolt",
            "artifact": "cli",
            "channel": "dev",
            "method": "direct",
            "root": root,
        })
        .to_string(),
    )
    .unwrap();
    std::fs::set_permissions(&receipt, std::fs::Permissions::from_mode(0o644)).unwrap();

    let files = Arc::new(Mutex::new(HashMap::from([(
        DEV.to_string(),
        manifest(env!("CARGO_PKG_VERSION"), 1),
    )])));
    let base = release_service(files.clone());
    let run_json = |args: &[&str]| {
        let mut cmd = a.cmd_of(&exe, args);
        cmd.env("DEADBOLT_RELEASE_URL", &base);
        a.output(&mut cmd)
    };

    // Up to date: exit 0, clean JSON with method = "direct", available = false, action = "none".
    let out = run_json(&["update", "--check", "--channel", "dev", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(
        report,
        serde_json::json!({
            "current": env!("CARGO_PKG_VERSION"),
            "latest": env!("CARGO_PKG_VERSION"),
            "channel": "dev",
            "available": false,
            "method": "direct",
            "action": "none",
            "reason": "up_to_date",
        })
    );

    // Update available: exit 10, clean JSON with available = true, action = "install".
    files
        .lock()
        .unwrap()
        .insert(DEV.to_string(), manifest("0.0.0-dev-9999", 2));
    let out = run_json(&["update", "--check", "--channel", "dev", "--json"]);
    assert_eq!(out.status.code(), Some(10));
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(
        report,
        serde_json::json!({
            "current": env!("CARGO_PKG_VERSION"),
            "latest": "0.0.0-dev-9999",
            "channel": "dev",
            "available": true,
            "method": "direct",
            "action": "install",
            "reason": "update",
        })
    );
}

/// A release service on 127.0.0.1 for `shuvbolt update`: serves whatever
/// `files` holds at each path, one request per connection.
fn release_service(
    files: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>>,
) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/release/", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).ok();
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
            let body = files.lock().unwrap().get(&path).cloned();
            let (status, body) = match body {
                Some(body) => ("200 OK", body),
                None => ("404 Not Found", b"{}".to_vec()),
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).ok();
            stream.write_all(&body).ok();
        }
    });
    base
}

#[test]
fn concurrent_updates_cannot_pass_a_pending_extension_install() {
    use sha2::Digest;
    let first = ["update", "--channel", "dev"];
    let a = Device::new();
    let ext = a.path("extension");
    let output = a.output(
        a.cmd(&[
            "browser",
            "install",
            "--browser",
            "chrome",
            "--extension-dir",
        ])
        .arg(&ext)
        .env("HOME", a.path("user")),
    );
    assert!(output.status.success());
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"{"version":"1.0","permissions":["nativeMessaging"]}"#)
        .unwrap();
    let zip = zip.finish().unwrap().into_inner();
    let sha = data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(&zip));
    let file = format!("/release/deadbolt/files/{sha}/extension.zip");
    let target = serde_json::json!({"url": file, "size": zip.len(), "sha256": sha});
    let extension = serde_json::json!({
        "schema": 1, "app": "deadbolt", "artifact": "extension", "channel": "dev",
        "sequence": 7, "published_at": "2026-10-28T00:00:00Z",
        "release": {"version": "1.0.0", "rollout_percent": 100,
            "targets": {"chromium": target}, "distributions": []}
    });
    let mut cli = extension.clone();
    cli["artifact"] = "cli".into();
    cli["release"]["version"] = env!("CARGO_PKG_VERSION").into();
    cli["release"]["targets"] = serde_json::json!({
        "macos-arm64": target, "macos-x64": target,
        "linux-arm64": target, "linux-x64": target
    });
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/release/", listener.local_addr().unwrap());
    let (pending, ready) = mpsc::channel();
    let (resume, proceed) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let path = request.split_whitespace().nth(1).unwrap();
            let downloading = path == file;
            let body = if downloading {
                pending.send(()).unwrap();
                proceed.recv_timeout(Duration::from_secs(15)).unwrap();
                zip.clone()
            } else if path.ends_with("/cli/dev.json") {
                cli.to_string().into_bytes()
            } else {
                assert!(path.ends_with("/extension/dev.json"), "{path}");
                extension.to_string().into_bytes()
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
            if downloading {
                break;
            }
        }
    });
    let mut updating = a
        .cmd(&first)
        .env("DEADBOLT_RELEASE_URL", &base)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    ready.recv_timeout(Duration::from_secs(10)).unwrap();
    // The floor is already written, but the payload has not arrived.
    let paths = deadbolt_client::Paths::from_vars(|name| match name {
        "DEADBOLT_HOME" => Some(a.path("home").into()),
        "XDG_STATE_HOME" => Some(a.path("state").into()),
        "XDG_CONFIG_HOME" => Some(a.path("config").into()),
        _ => None,
    });
    let floors = paths.log.with_file_name("release-sequences.json");
    let before = std::fs::read(&floors).unwrap();
    let config = paths.config;
    let registered = std::fs::read(&config).unwrap();
    let contender = ["update", "--check", "--channel", "dev"];
    let mut child = a
        .cmd(&contender)
        .env("DEADBOLT_RELEASE_URL", &base)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    assert!(!wait(&mut child, Duration::from_secs(5)).success());
    let output = child.wait_with_output().unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("another shuvbolt update"));
    assert_eq!(std::fs::read(&floors).unwrap(), before);
    assert_eq!(std::fs::read(&config).unwrap(), registered);
    resume.send(()).unwrap();
    assert!(wait(&mut updating, Duration::from_secs(5)).success());
    server.join().unwrap();
    assert!(
        std::fs::read_to_string(ext.join("manifest.json"))
            .unwrap()
            .contains("1.0")
    );
    // A process finishing releases the persistent lock file for the next one.
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(paths.log.with_file_name("update.lock"))
        .unwrap();
    lock.try_lock().unwrap();
}

#[test]
fn update_replaces_a_standalone_deadbolt_only_once_it_checks_out() {
    use sha2::Digest;
    let a = Device::new();
    let bin = a.path("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join("shuvbolt");
    std::fs::copy(BIN, &exe).unwrap();
    let original = std::fs::read(&exe).unwrap();

    // The release: a tarball whose deadbolt is a stand-in script.
    let release = a.path("release");
    std::fs::create_dir_all(&release).unwrap();
    std::fs::write(
        release.join("shuvbolt"),
        "#!/bin/sh\necho stand-in shuvbolt\n",
    )
    .unwrap();
    let tarball = a.path("deadbolt.tar.gz");
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&tarball)
        .arg("-C")
        .arg(&release)
        .arg("shuvbolt")
        .status()
        .unwrap();
    assert!(status.success());
    let tar = std::fs::read(&tarball).unwrap();
    let sha = |bytes: &[u8]| data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(bytes));
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let manifest = |artifact: &str, channel: &str, version: &str, target: &str, size, sha256| {
        let mut targets = serde_json::Map::new();
        targets.insert(
            target.to_string(),
            serde_json::json!({
                "url": format!("/release/deadbolt/files/{sha256}/{artifact}.bin"),
                "size": size,
                "sha256": sha256,
            }),
        );
        serde_json::json!({
            "schema": 1, "app": "deadbolt", "artifact": artifact, "channel": channel,
            "sequence": 1, "published_at": "2026-10-28T00:00:00Z",
            "release": {
                "version": version, "rollout_percent": 100, "targets": targets,
                "distributions": [{ "kind": "direct" }], "revoked_versions": []
            }
        })
        .to_string()
        .into_bytes()
    };
    let target = format!("{os}-{arch}");
    let good = sha(&tar);
    let wrong = sha(b"something else");
    let files = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([
        (
            format!("/release/deadbolt/files/{good}/cli.bin"),
            tar.clone(),
        ),
        (
            format!("/release/deadbolt/files/{wrong}/cli.bin"),
            tar.clone(),
        ),
        (
            "/release/deadbolt/cli/beta.json".to_string(),
            manifest(
                "cli",
                "beta",
                "0.0.0-beta.9",
                &target,
                tar.len(),
                wrong.clone(),
            ),
        ),
        (
            "/release/deadbolt/cli/stable.json".to_string(),
            manifest(
                "cli",
                "stable",
                "9.9.9",
                &target,
                tar.len() + 1,
                good.clone(),
            ),
        ),
        (
            "/release/deadbolt/cli/dev.json".to_string(),
            manifest(
                "cli",
                "dev",
                env!("CARGO_PKG_VERSION"),
                &target,
                tar.len(),
                good.clone(),
            ),
        ),
    ])));
    let base = release_service(files.clone());
    let update = |args: &[&str]| {
        let mut cmd = a.cmd_of(&exe, args);
        cmd.env("DEADBOLT_RELEASE_URL", &base);
        a.output(&mut cmd)
    };
    let leftovers = || {
        std::fs::read_dir(&bin)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|name| name != "shuvbolt")
            .collect::<Vec<_>>()
    };

    // A build from source follows no channel until told one.
    let output = update(&["update", "--check"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--channel"));

    // The running version is the channel's: nothing to do.
    let output = update(&["update", "--check", "--channel", "dev"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("up to date"));

    // A wrong SHA-256 or size leaves the old binary alone.
    for channel in ["beta", "stable"] {
        let output = update(&["update", "--channel", channel]);
        assert!(!output.status.success(), "{channel}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("SHA-256") || stderr.contains("size"),
            "{stderr}"
        );
        assert!(std::fs::read(&exe).unwrap() == original, "{channel}");
        assert!(leftovers().is_empty(), "{:?}", leftovers());
    }
    // So does a manifest pointing anywhere but the service's files.
    files.lock().unwrap().insert(
        "/release/deadbolt/cli/dev.json".to_string(),
        String::from_utf8(manifest(
            "cli",
            "dev",
            "0.0.0-dev-99",
            &target,
            tar.len(),
            good.clone(),
        ))
        .unwrap()
        .replace(
            "/release/deadbolt/files/",
            "http://198.51.100.1/release/deadbolt/files/",
        )
        .into_bytes(),
    );
    let output = update(&["update", "--channel", "dev"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside"));
    assert!(std::fs::read(&exe).unwrap() == original);

    // A replayed older manifest is refused unless asked for.
    let sequenced = |n: u32| {
        String::from_utf8(manifest(
            "cli",
            "dev",
            "0.0.0-dev-99",
            &target,
            tar.len(),
            good.clone(),
        ))
        .unwrap()
        .replace("\"sequence\":1", &format!("\"sequence\":{n}"))
        .into_bytes()
    };
    let serve = |n: u32| {
        files
            .lock()
            .unwrap()
            .insert("/release/deadbolt/cli/dev.json".to_string(), sequenced(n));
    };
    serve(3);
    assert_eq!(
        update(&["update", "--check", "--channel", "dev"])
            .status
            .code(),
        Some(10)
    );
    serve(2);
    let output = update(&["update", "--channel", "dev"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("--allow-rollback"));
    assert!(std::fs::read(&exe).unwrap() == original);
    let output = update(&["update", "--check", "--channel", "dev", "--allow-rollback"]);
    assert_eq!(output.status.code(), Some(10));
    serve(3);

    // On macOS the new deadbolt pins its own helper: a release without one
    // never replaces a deadbolt that has one beside it.
    if cfg!(target_os = "macos") {
        std::fs::write(bin.join("deadbolt-enclave"), "old helper").unwrap();
        let output = update(&["update", "--channel", "dev"]);
        assert!(String::from_utf8_lossy(&output.stderr).contains("deadbolt-enclave"));
        assert!(std::fs::read(&exe).unwrap() == original);
        assert_eq!(
            std::fs::read(bin.join("deadbolt-enclave")).unwrap(),
            b"old helper"
        );
        std::fs::remove_file(bin.join("deadbolt-enclave")).unwrap();
    }

    // A good release, with an unpacked extension registered beside it.
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"{"name":"Shuvbolt","version":"0.0.0.99","version_name":"0.0.0-dev-99","permissions":["storage","nativeMessaging"]}"#)
        .unwrap();
    let extension = zip.finish().unwrap().into_inner();
    let ext_sha = sha(&extension);
    files.lock().unwrap().extend([
        (
            format!("/release/deadbolt/files/{ext_sha}/extension.bin"),
            extension.clone(),
        ),
        (
            "/release/deadbolt/extension/dev.json".to_string(),
            manifest(
                "extension",
                "dev",
                "0.0.0-dev-99",
                "chromium",
                extension.len(),
                ext_sha.clone(),
            ),
        ),
    ]);
    let ext = a.path("extension");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(ext.join("mine.txt"), "not an extension").unwrap();
    let output = a.output(
        a.cmd(&[
            "browser",
            "install",
            "--browser",
            "chrome",
            "--extension-dir",
        ])
        .arg(&ext)
        .env("HOME", a.path("user")),
    );
    assert!(
        !output.status.success(),
        "replaced a directory that isn't the extension"
    );
    std::fs::remove_file(ext.join("mine.txt")).unwrap();
    let output = a.output(
        a.cmd(&[
            "browser",
            "install",
            "--browser",
            "chrome",
            "--extension-dir",
        ])
        .arg(&ext)
        .env("HOME", a.path("user")),
    );
    assert!(output.status.success());

    // A rejected extension manifest must not poison the replay floor.
    let ext_feed = "/release/deadbolt/extension/dev.json";
    let valid = files.lock().unwrap()[ext_feed].clone();
    for (field, value) in [("artifact", "cli"), ("channel", "stable"), ("app", "other")] {
        let mut invalid: serde_json::Value = serde_json::from_slice(&valid).unwrap();
        invalid[field] = value.into();
        invalid["sequence"] = 999.into();
        files
            .lock()
            .unwrap()
            .insert(ext_feed.into(), invalid.to_string().into_bytes());
        let output = update(&["update", "--check", "--channel", "dev"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("not a usable"));
        files.lock().unwrap().insert(ext_feed.into(), valid.clone());
        let output = update(&["update", "--check", "--channel", "dev"]);
        assert_eq!(
            output.status.code(),
            Some(10),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = update(&["update", "--check", "--channel", "dev"]);
    assert_eq!(output.status.code(), Some(10));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0.0.0-dev-99 is available"), "{stdout}");
    assert!(
        stdout.contains("Browser extension 0.0.0-dev-99"),
        "{stdout}"
    );
    assert!(std::fs::read(&exe).unwrap() == original);

    let output = update(&["update", "--channel", "dev"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("chrome://extensions"));
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        b"#!/bin/sh\necho stand-in shuvbolt\n"
    );
    let ran = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "stand-in shuvbolt\n");
    assert!(leftovers().is_empty(), "{:?}", leftovers());
    assert!(
        std::fs::read_to_string(ext.join("manifest.json"))
            .unwrap()
            .contains("0.0.0-dev-99")
    );
    // It knows the files it put there, and nothing else is in the way.
    assert_eq!(
        std::fs::read_to_string(ext.join(".deadbolt-extension")).unwrap(),
        "manifest.json\n"
    );

    // This test's deadbolt (built from source, so it names the channel);
    // the copy above is the stand-in now.
    let deadbolt = |args: &[&str]| {
        let mut cmd = a.cmd(args);
        cmd.env("DEADBOLT_RELEASE_URL", &base);
        a.output(&mut cmd)
    };

    // A second directory: `update` refreshes each one registered.
    let second = a.path("extension-second");
    let output = a.output(
        a.cmd(&[
            "browser",
            "install",
            "--browser",
            "chrome",
            "--extension-dir",
        ])
        .arg(&second)
        .env("HOME", a.path("user")),
    );
    assert!(output.status.success());
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"{"name":"Shuvbolt","version":"0.0.0.100","version_name":"0.0.0-dev-100","permissions":["storage","nativeMessaging"]}"#)
        .unwrap();
    let native = zip.finish().unwrap().into_inner();
    let native_sha = sha(&native);
    let mut release: serde_json::Value = serde_json::from_slice(&manifest(
        "extension",
        "dev",
        "0.0.0-dev-100",
        "chromium",
        native.len(),
        native_sha.clone(),
    ))
    .unwrap();
    release["sequence"] = 3.into();
    // This test's deadbolt is the channel's current CLI, so `update` only
    // touches the extensions.
    let current_cli = String::from_utf8(manifest(
        "cli",
        "dev",
        env!("CARGO_PKG_VERSION"),
        &target,
        tar.len(),
        good.clone(),
    ))
    .unwrap()
    .replace("\"sequence\":1", "\"sequence\":4")
    .into_bytes();
    files.lock().unwrap().extend([
        (
            format!("/release/deadbolt/files/{native_sha}/extension.bin"),
            native,
        ),
        (
            "/release/deadbolt/extension/dev.json".to_string(),
            release.to_string().into_bytes(),
        ),
        ("/release/deadbolt/cli/dev.json".to_string(), current_cli),
    ]);
    let output = deadbolt(&["update", "--channel", "dev"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stdout.matches("Updated the browser extension").count(),
        2,
        "{stdout}"
    );
    let manifest_of =
        |dir: &std::path::Path| std::fs::read_to_string(dir.join("manifest.json")).unwrap();
    assert!(manifest_of(&ext).contains("0.0.0-dev-100"));
    assert!(manifest_of(&second).contains("0.0.0-dev-100"));

    // Unregistered, a directory is left alone by `update`.
    let output = a.output(
        a.cmd(&["browser", "uninstall", "--extension-dir"])
            .arg(&second)
            .env("HOME", a.path("user")),
    );
    assert!(output.status.success());
    let output = a.output(
        a.cmd(&["browser", "uninstall", "--extension-dir"])
            .arg(&second)
            .env("HOME", a.path("user")),
    );
    assert!(!output.status.success(), "unregistered twice");
    let output = deadbolt(&["update", "--check", "--channel", "dev"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let shown = |dir: &std::path::Path| dir.canonicalize().unwrap().display().to_string();
    assert!(stdout.contains(&shown(&ext)), "{stdout}");
    assert!(!stdout.contains(&shown(&second)), "{stdout}");
    assert!(second.join("manifest.json").exists());
}
