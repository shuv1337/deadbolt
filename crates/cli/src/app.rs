//! Opening the desktop app, `shuvbolt-ui`, a program of its own beside this
//! headless one (docs/app.md, "One command"): `shuvbolt app` from anywhere,
//! bare `shuvbolt` started from outside a terminal, and `shuvbolt ui [flags]`,
//! which runs it in the foreground with Rune's flags. The app is a thin client
//! of the daemon like any other. A running app is brought forward through the
//! daemon (`app-activate`) rather than started again.

use std::ffi::{OsStr, OsString};
use std::io::IsTerminal;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::paths::Paths;
use crate::protocol::Reply;

/// The hidden subcommand that runs `shuvbolt-ui` in the foreground, taking
/// Rune's flags (`--scene`, `--snap`, `--panel`, …).
pub const UI: &str = "ui";
/// The app's program outside a bundle.
const UI_PROGRAM: &str = "shuvbolt-ui";
/// Names the `shuvbolt-ui` to run, ahead of every other place.
const UI_BIN: &str = "DEADBOLT_UI_BIN";
/// Shuvbolt.app's bundle identifier.
const BUNDLE_ID: &str = "dev.shuv.shuvbolt";
/// Keeps bare `shuvbolt` from ever opening a window (scripts, agents).
const NO_UI: &str = "DEADBOLT_NO_UI";
/// The longest `app-activate` may take: the daemon holds it while an app
/// another launch is starting gets to its window (`watchers::STARTING`).
const ACTIVATE_TIMEOUT: Duration = Duration::from_secs(10);
/// What `shuvbolt app` and `shuvbolt ui` say when there's no app to open.
const NOT_INSTALLED: &str = "Shuvbolt's desktop app (shuvbolt-ui) isn't installed here. \
Build it from https://github.com/shuv1337/shuvbolt. On macOS install Shuvbolt.app; \
on Linux put shuvbolt-ui beside shuvbolt. DEADBOLT_UI_BIN names one anywhere else";

/// The token a launcher gave this process for the window it opens:
/// Wayland's `XDG_ACTIVATION_TOKEN`, else X11's `DESKTOP_STARTUP_ID`. A
/// running app needs it for the compositor to let it take focus.
pub fn activation_token(var: impl Fn(&str) -> Option<String>) -> Option<String> {
    ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
        .into_iter()
        .find_map(|name| var(name).filter(|v| !v.is_empty()))
}

/// Whether `app-activate`'s outcome means an app came forward, so none is
/// started: only `activated`. No daemon, a daemon of another protocol, `no-app`
/// or no answer in time all start one.
fn came_forward<E>(outcome: Result<Result<Reply, E>, tokio::time::error::Elapsed>) -> bool {
    matches!(outcome, Ok(Ok(Reply::Activated)))
}

/// Asks the daemon, if one runs, to bring a running app forward with this
/// launch's activation token. False when the caller should start the app.
async fn activate_running(paths: &Paths) -> bool {
    let token = activation_token(|name| crate::config::env(name).ok());
    let command = crate::protocol::Command::AppActivate { token };
    came_forward(
        tokio::time::timeout(ACTIVATE_TIMEOUT, crate::client::simple(paths, command)).await,
    )
}

/// Whether bare `shuvbolt` (no arguments) opens the app instead of printing
/// usage: only when none of stdin, stdout and stderr is a terminal, a
/// graphical session exists (always on macOS; a Wayland or X display on
/// Linux), it isn't an SSH session and `DEADBOLT_NO_UI` isn't set.
pub fn bare_opens_app(terminal: bool, macos: bool, var: impl Fn(&str) -> Option<String>) -> bool {
    let opted_out = var(NO_UI).is_some_and(|v| !v.is_empty() && v != "0");
    if terminal || opted_out || set(&var, "SSH_CONNECTION") {
        return false;
    }
    graphical(macos, var)
}

fn set(var: &impl Fn(&str) -> Option<String>, name: &str) -> bool {
    var(name).is_some_and(|v| !v.is_empty())
}

/// Whether there's a graphical session to open a window in: always on
/// macOS; a Wayland or X display elsewhere.
fn graphical(macos: bool, var: impl Fn(&str) -> Option<String>) -> bool {
    macos || set(&var, "WAYLAND_DISPLAY") || set(&var, "DISPLAY")
}

/// Whether this process was started from a terminal.
pub fn from_terminal() -> bool {
    std::io::stdin().is_terminal()
        || std::io::stdout().is_terminal()
        || std::io::stderr().is_terminal()
}

/// The `shuvbolt-ui` for the `shuvbolt` at `exe` (symlinks resolved), in
/// order: `override_bin` (`DEADBOLT_UI_BIN`); on macOS, the app of the
/// Shuvbolt.app this is the `Contents/Helpers/shuvbolt` of (even when
/// missing: a bundle never opens another install's app); the `shuvbolt-ui`
/// beside `exe` (a build's `target/<profile>`, an install's directory); on
/// macOS, Shuvbolt.app in `/Applications`, then `~/Applications`; then the
/// first `shuvbolt-ui` in `path`. Only files `usable` takes count, the
/// override and the bundle's own aside.
pub fn find_ui(
    exe: &Path,
    override_bin: Option<&OsStr>,
    macos: bool,
    home: Option<&Path>,
    path: &OsStr,
    usable: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if let Some(program) = override_bin.filter(|p| !p.is_empty()) {
        return Some(program.into());
    }
    if macos && let Some(bundle) = bundle_of(exe) {
        return Some(bundle.join("Contents/MacOS/Shuvbolt"));
    }
    let beside = exe.parent().map(|dir| dir.join(UI_PROGRAM));
    let apps = [
        Some(PathBuf::from("/Applications")),
        home.map(|h| h.join("Applications")),
    ]
    .into_iter()
    .flatten()
    .filter(|_| macos)
    .map(|dir| dir.join("Shuvbolt.app/Contents/MacOS/Shuvbolt"));
    let on_path = std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(UI_PROGRAM));
    beside
        .into_iter()
        .chain(apps)
        .chain(on_path)
        .find(|program| usable(program))
}

/// [`find_ui`] for this process.
fn this_ui(exe: &Path) -> Option<PathBuf> {
    let var = |name: &str| crate::config::env(name).ok().map(OsString::from);
    let home = var("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    find_ui(
        exe,
        var(UI_BIN).as_deref(),
        cfg!(target_os = "macos"),
        home.as_deref(),
        &var("PATH").unwrap_or_default(),
        executable,
    )
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `shuvbolt ui [args]`: replaces this process with `shuvbolt-ui [args]`,
/// so its exit status and signals are the app's own. Returns only when it
/// can't.
pub fn run_ui(args: &[OsString]) -> ExitCode {
    let program = this_binary().ok().as_deref().and_then(this_ui);
    let Some(program) = program else {
        eprintln!("error: {NOT_INSTALLED}");
        return ExitCode::FAILURE;
    };
    let error = Command::new(&program).args(args).exec();
    eprintln!("error: couldn't run {}: {error}", program.display());
    ExitCode::FAILURE
}

/// What opens the app.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// A Shuvbolt.app, through `open`: the one this binary is the helper
    /// of, or the one [`find_ui`] found.
    Bundle(PathBuf),
    /// A `shuvbolt-ui` outside any bundle, started directly.
    Program(PathBuf),
    /// No app here.
    Missing,
}

/// What opens the app for the `shuvbolt` at `exe` (symlinks resolved),
/// given the `shuvbolt-ui` [`find_ui`] found.
pub fn locate(exe: &Path, macos: bool, ui: Option<PathBuf>) -> Target {
    if macos && let Some(bundle) = bundle_of(exe) {
        return Target::Bundle(bundle);
    }
    match ui {
        Some(ui) => match app_bundle_of(&ui).filter(|_| macos) {
            Some(bundle) => Target::Bundle(bundle),
            None => Target::Program(ui),
        },
        None => Target::Missing,
    }
}

/// `X.app` for `X.app/Contents/Helpers/shuvbolt`.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    bundle_with(exe, "Helpers")
}

/// `X.app` for its own executable, `X.app/Contents/MacOS/<name>`.
fn app_bundle_of(exe: &Path) -> Option<PathBuf> {
    bundle_with(exe, "MacOS")
}

fn bundle_with(exe: &Path, dir: &str) -> Option<PathBuf> {
    let inner = exe.parent()?;
    let contents = inner.parent()?;
    let bundle = contents.parent()?;
    (inner.file_name()? == dir
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_path_buf())
}

/// This binary's path, symlinks resolved, so a `~/.local/bin/shuvbolt`
/// linked into Shuvbolt.app still finds its bundle.
pub fn this_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("finding this binary")?;
    Ok(exe.canonicalize().unwrap_or(exe))
}

/// Opens the app and returns once it's launched, detached from this
/// process, or once a running one came forward. Shuvbolt.app goes through
/// Launch Services, which brings a running copy to the front; a macOS
/// release build first tries whichever Shuvbolt.app Launch Services knows
/// (`open -b`). A `shuvbolt-ui` outside a bundle is asked through the daemon
/// first (`app-activate`), and only started when no app answers.
pub async fn open(paths: &Paths) -> Result<()> {
    let exe = this_binary()?;
    let macos = cfg!(target_os = "macos");
    let home = crate::config::env("HOME").ok().filter(|h| !h.is_empty());
    if macos && let Some(home) = &home {
        tidy_applications(Path::new(home), bundle_of(&exe).as_deref());
    }
    match locate(&exe, macos, this_ui(&exe)) {
        Target::Bundle(bundle) => {
            if let Some(home) = &home {
                link_into_applications(&bundle, Path::new(home));
            }
            launch_services(Command::new("/usr/bin/open").arg(&bundle))
                .with_context(|| format!("couldn't open {}", bundle.display()))
        }
        target => {
            // A development build opens its own app, not an installed one.
            if macos
                && !cfg!(debug_assertions)
                && launch_services(Command::new("/usr/bin/open").args(["-b", BUNDLE_ID])).is_ok()
            {
                return Ok(());
            }
            let Target::Program(program) = target else {
                bail!("{NOT_INSTALLED}");
            };
            if !graphical(macos, |name| crate::config::env(name).ok()) {
                bail!(
                    "there's no graphical session here (neither WAYLAND_DISPLAY nor DISPLAY \
                     is set); `shuvbolt app` opens a window on your desktop"
                );
            }
            if activate_running(paths).await {
                return Ok(());
            }
            if !macos {
                crate::desktop::ensure(&crate::desktop::launcher_program(&exe));
            }
            start(&program)
        }
    }
}

/// Starts `shuvbolt-ui` in its own process group with no stdio. It inherits
/// the launcher's activation token for its first window (Rune takes it from
/// the environment), and finds the `shuvbolt` beside it for its daemon.
fn start(program: &Path) -> Result<()> {
    Command::new(program)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .with_context(|| format!("starting {}", program.display()))?;
    Ok(())
}

/// Removes `~/Applications/Shuvbolt.app` when it's a link an npm install
/// made ([`stale_npm_link`]). Best effort.
fn tidy_applications(home: &Path, bundle: Option<&Path>) {
    let ours = home.join("Applications/Shuvbolt.app");
    let target = std::fs::read_link(&ours).ok();
    if stale_npm_link(target.as_deref(), bundle, |p| {
        ours.parent().unwrap_or(home).join(p).exists()
    }) && let Err(error) = std::fs::remove_file(&ours)
    {
        tracing::warn!("removing {}: {error}", ours.display());
    }
}

/// Puts a link to `bundle` in `~/Applications`, so Launchpad, Finder and
/// Spotlight's apps list Shuvbolt, when it lives somewhere else and neither
/// Applications folder has a Shuvbolt yet. Best effort: opening the app
/// doesn't wait on it.
fn link_into_applications(bundle: &Path, home: &Path) {
    let Some(link) = applications_link(bundle, home, |p| p.symlink_metadata().is_ok()) else {
        return;
    };
    if let Some(dir) = link.parent()
        && std::fs::create_dir_all(dir).is_ok()
        && std::os::unix::fs::symlink(bundle, &link).is_ok()
    {
        eprintln!("Linked {} to {}", link.display(), bundle.display());
    }
}

/// Whether `~/Applications/Shuvbolt.app` links into a `node_modules`
/// directory that no longer has the bundle
/// (`exists`), or, opening `bundle`, anywhere but `bundle`. Anything else
/// there is left alone.
fn stale_npm_link(
    target: Option<&Path>,
    bundle: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> bool {
    target.is_some_and(|target| {
        target.components().any(|c| c.as_os_str() == "node_modules")
            && Some(target) != bundle
            && (bundle.is_some() || !exists(target))
    })
}

/// Where [`link_into_applications`] links `bundle`, if it should.
fn applications_link(
    bundle: &Path,
    home: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let user = home.join("Applications");
    let text = bundle.to_string_lossy();
    let movable = !(text.starts_with("/Volumes/") || text.contains("/AppTranslocation/"));
    let elsewhere = [Path::new("/Applications"), &user]
        .iter()
        .all(|dir| bundle.parent() != Some(dir));
    let link = user.join("Shuvbolt.app");
    let taken = exists(&link) || exists(Path::new("/Applications/Shuvbolt.app"));
    (movable && elsewhere && !taken).then_some(link)
}

/// Runs `open`, which returns once Launch Services has the app started.
fn launch_services(command: &mut Command) -> Result<()> {
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running open")?;
    if !status.success() {
        bail!("open exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<String> + 'a {
        |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn the_launchers_token_is_waylands_else_x11s() {
        assert_eq!(activation_token(env(&[])), None);
        let x11 = [("DESKTOP_STARTUP_ID", "kde-123_TIME456")];
        assert_eq!(
            activation_token(env(&x11)).as_deref(),
            Some("kde-123_TIME456")
        );
        let both = [("DESKTOP_STARTUP_ID", "x"), ("XDG_ACTIVATION_TOKEN", "w")];
        assert_eq!(activation_token(env(&both)).as_deref(), Some("w"));
        let empty = [("XDG_ACTIVATION_TOKEN", ""), ("DESKTOP_STARTUP_ID", "x")];
        assert_eq!(activation_token(env(&empty)).as_deref(), Some("x"));
    }

    #[tokio::test(start_paused = true)]
    async fn only_an_app_that_came_forward_keeps_another_from_starting() {
        let answered = |reply| async move { Ok::<_, ()>(reply) };
        let forward = tokio::time::timeout(ACTIVATE_TIMEOUT, answered(Reply::Activated)).await;
        assert!(came_forward(forward));
        let no_app = tokio::time::timeout(ACTIVATE_TIMEOUT, answered(Reply::NoApp)).await;
        assert!(!came_forward(no_app));
        // No daemon, or one that refused the request.
        let failed = tokio::time::timeout(ACTIVATE_TIMEOUT, async { Err::<Reply, _>(()) }).await;
        assert!(!came_forward(failed));
        let stuck = tokio::time::timeout(
            ACTIVATE_TIMEOUT,
            std::future::pending::<Result<Reply, ()>>(),
        );
        assert!(!came_forward(stuck.await));
    }

    #[test]
    fn bare_deadbolt_opens_the_app_only_outside_a_terminal_with_a_display() {
        let wayland = [("WAYLAND_DISPLAY", "wayland-1")];
        let x11 = [("DISPLAY", ":0")];
        assert!(bare_opens_app(false, false, env(&wayland)));
        assert!(bare_opens_app(false, false, env(&x11)));
        assert!(bare_opens_app(false, true, env(&[])));
        // A terminal on any stream is the CLI.
        assert!(!bare_opens_app(true, false, env(&wayland)));
        assert!(!bare_opens_app(true, true, env(&[])));
        // No graphical session on Linux.
        assert!(!bare_opens_app(false, false, env(&[])));
        assert!(!bare_opens_app(false, false, env(&[("DISPLAY", "")])));
        // SSH, even with a forwarded display.
        let ssh = [("DISPLAY", "localhost:10"), ("SSH_CONNECTION", "1 2 3 4")];
        assert!(!bare_opens_app(false, false, env(&ssh)));
        assert!(!bare_opens_app(false, true, env(&ssh[1..])));
        // The opt-out.
        for value in ["1", "yes"] {
            let vars = [("WAYLAND_DISPLAY", "wayland-1"), (NO_UI, value)];
            assert!(!bare_opens_app(false, false, env(&vars)));
            assert!(!bare_opens_app(false, true, env(&vars[1..])));
        }
        for value in ["", "0"] {
            assert!(bare_opens_app(false, true, env(&[(NO_UI, value)])));
        }
    }

    #[test]
    fn the_ui_is_the_override_else_the_bundles_else_beside_else_installed() {
        let cli = Path::new("/home/alex/.local/bin/shuvbolt");
        let home = Path::new("/home/alex");
        let path = OsStr::new("/usr/bin:relative:/opt/bin");
        let all = |_: &Path| true;
        let none = |_: &Path| false;
        let find = |exe, over: Option<&str>, macos, usable: &dyn Fn(&Path) -> bool| {
            find_ui(exe, over.map(OsStr::new), macos, Some(home), path, usable)
        };
        // The override, even when it isn't there.
        assert_eq!(
            find(cli, Some("/x/shuvbolt-ui"), true, &none),
            Some(PathBuf::from("/x/shuvbolt-ui"))
        );
        assert_eq!(
            find(cli, Some(""), false, &all),
            Some(PathBuf::from("/home/alex/.local/bin/shuvbolt-ui"))
        );
        // A bundle's helper opens only its own app, even a missing one.
        let helper = Path::new("/Applications/Shuvbolt.app/Contents/Helpers/shuvbolt");
        assert_eq!(
            find(helper, None, true, &none),
            Some(PathBuf::from(
                "/Applications/Shuvbolt.app/Contents/MacOS/Shuvbolt"
            ))
        );
        // Linux has no bundles, even shaped like one.
        assert_eq!(find(helper, None, false, &none), None);
        // Beside, then the Applications folders (macOS only), then PATH.
        let only = |want: &'static str| move |p: &Path| p == Path::new(want);
        assert_eq!(
            find(
                cli,
                None,
                true,
                &only("/home/alex/Applications/Shuvbolt.app/Contents/MacOS/Shuvbolt")
            ),
            Some(PathBuf::from(
                "/home/alex/Applications/Shuvbolt.app/Contents/MacOS/Shuvbolt"
            ))
        );
        assert_eq!(
            find(
                cli,
                None,
                false,
                &only("/home/alex/Applications/Shuvbolt.app/Contents/MacOS/Shuvbolt")
            ),
            None
        );
        assert_eq!(
            find(cli, None, true, &|p: &Path| p.starts_with("/Applications")
                || p.starts_with("/opt")),
            Some(PathBuf::from(
                "/Applications/Shuvbolt.app/Contents/MacOS/Shuvbolt"
            ))
        );
        assert_eq!(
            find(cli, None, false, &|p: &Path| p.starts_with("/usr")
                || p.starts_with("/opt")),
            Some(PathBuf::from("/usr/bin/shuvbolt-ui"))
        );
        assert_eq!(find(cli, None, false, &only("relative/shuvbolt-ui")), None);
        assert_eq!(find(cli, None, true, &none), None);
    }

    #[test]
    fn the_app_is_a_bundle_else_a_program_else_missing() {
        let helper = Path::new("/Applications/Shuvbolt.app/Contents/Helpers/shuvbolt");
        let bundle = PathBuf::from("/Applications/Shuvbolt.app");
        assert_eq!(locate(helper, true, None), Target::Bundle(bundle.clone()));
        let cli = Path::new("/usr/local/bin/shuvbolt");
        let app = PathBuf::from("/Applications/Shuvbolt.app/Contents/MacOS/Shuvbolt");
        assert_eq!(locate(cli, true, Some(app.clone())), Target::Bundle(bundle));
        let ui = PathBuf::from("/usr/local/bin/shuvbolt-ui");
        assert_eq!(
            locate(cli, true, Some(ui.clone())),
            Target::Program(ui.clone())
        );
        assert_eq!(locate(cli, false, Some(ui.clone())), Target::Program(ui));
        // Linux has no bundles, even shaped like one.
        assert_eq!(
            locate(helper, false, Some(app.clone())),
            Target::Program(app)
        );
        assert_eq!(locate(cli, true, None), Target::Missing);
        assert_eq!(locate(helper, false, None), Target::Missing);
    }

    #[test]
    fn the_bundle_is_found_by_its_helpers_shape() {
        assert_eq!(
            bundle_of(Path::new(
                "/Applications/Shuvbolt.app/Contents/Helpers/shuvbolt"
            )),
            Some(PathBuf::from("/Applications/Shuvbolt.app"))
        );
        assert!(
            bundle_of(Path::new(
                "/Applications/Shuvbolt/Contents/Helpers/shuvbolt"
            ))
            .is_none()
        );
        assert!(
            bundle_of(Path::new(
                "/Applications/Shuvbolt.app/Contents/MacOS/Shuvbolt"
            ))
            .is_none()
        );
    }

    #[test]
    fn only_a_link_an_older_npm_install_made_is_removed() {
        let npm = Path::new("/p/lib/node_modules/@deadbolt/cli-darwin-arm64/Shuvbolt.app");
        let gone = |_: &Path| false;
        let there = |_: &Path| true;
        // Its package no longer carries the app.
        assert!(stale_npm_link(Some(npm), None, gone));
        assert!(!stale_npm_link(Some(npm), None, there));
        // Opening another bundle, it's replaced either way.
        let bundle = Path::new("/Users/alex/Downloads/Shuvbolt.app");
        assert!(stale_npm_link(Some(npm), Some(bundle), there));
        assert!(!stale_npm_link(Some(npm), Some(npm), there));
        let elsewhere = Path::new("/Applications/Shuvbolt.app");
        assert!(!stale_npm_link(Some(elsewhere), None, gone));
        assert!(!stale_npm_link(Some(elsewhere), Some(bundle), gone));
        assert!(!stale_npm_link(None, None, gone));
    }

    #[test]
    fn an_app_outside_applications_gets_a_link_in_the_users() {
        let home = Path::new("/Users/alex");
        let npm = Path::new("/Users/alex/Downloads/Shuvbolt.app");
        let nothing = |_: &Path| false;
        assert_eq!(
            applications_link(npm, home, nothing),
            Some(PathBuf::from("/Users/alex/Applications/Shuvbolt.app"))
        );
        // Either Applications folder already has a Shuvbolt (a link included).
        for there in [
            "/Users/alex/Applications/Shuvbolt.app",
            "/Applications/Shuvbolt.app",
        ] {
            assert_eq!(
                applications_link(npm, home, |p| p == Path::new(there)),
                None
            );
        }
        // Already in one, or somewhere it can't stay.
        for bundle in [
            "/Applications/Shuvbolt.app",
            "/Users/alex/Applications/Shuvbolt.app",
            "/Volumes/Shuvbolt/Shuvbolt.app",
            "/private/var/folders/x/AppTranslocation/y/d/Shuvbolt.app",
        ] {
            assert_eq!(
                applications_link(Path::new(bundle), home, nothing),
                None,
                "{bundle}"
            );
        }
    }
}
