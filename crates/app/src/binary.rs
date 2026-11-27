//! Finding the `shuvbolt` that runs the daemon for `shuvbolt-ui`. Bundle
//! discovery is deliberately independent of the shell's PATH.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// A bundle must use its own helper, even if it is missing (surface the error,
/// rather than quietly starting a different installation).
pub fn bundled(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then(|| contents.join("Helpers/shuvbolt"))
}

/// Whether `path` has the shape of a bundle's helper,
/// `<Something>.app/Contents/Helpers/shuvbolt`, whether or not it exists.
pub fn is_helper(path: &Path) -> bool {
    let mut up = path.ancestors().skip(1);
    path.file_name().is_some_and(|n| n == "shuvbolt")
        && up
            .next()
            .and_then(Path::file_name)
            .is_some_and(|n| n == "Helpers")
        && up
            .next()
            .and_then(Path::file_name)
            .is_some_and(|n| n == "Contents")
        && up
            .next()
            .and_then(Path::extension)
            .is_some_and(|e| e == "app")
}

/// Where commands live even when the app's PATH doesn't say so: Finder,
/// the Dock and Spotlight start apps with only `/usr/bin:/bin:/usr/sbin:/sbin`.
pub fn well_known_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home {
        dirs.push(home.join(".local/bin"));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    if let Some(home) = home {
        dirs.push(home.join(".cargo/bin"));
    }
    dirs
}

/// PATH's absolute directories, then the well-known ones it lacks, in order.
pub fn search_dirs(path: &OsStr, home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::split_paths(path)
        .filter(|p| p.is_absolute())
        .collect();
    for dir in well_known_dirs(home) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// This program's path with symlinks resolved, so a link named like a
/// bundle's executable (`Some.app/Contents/MacOS/Shuvbolt`) doesn't make its
/// directory this app's bundle, with that bundle's helper as the daemon, and
/// a linked `shuvbolt-ui` finds the `shuvbolt` installed beside the real one.
pub fn resolved(exe: PathBuf) -> PathBuf {
    std::fs::canonicalize(&exe).unwrap_or(exe)
}

/// The daemon program for the app at `exe` ([`resolved`]): the bundle's own
/// helper inside Shuvbolt.app, else `override_bin` (`DEADBOLT_BIN`), else
/// the `shuvbolt` beside `exe` (a build's `target/<profile>`, an install's
/// directory), else the first in `search` ([`search_dirs`]) that `usable`
/// takes, else plain `shuvbolt`, which fails to start with a message that
/// says to install it.
pub fn discover(
    exe: Option<&Path>,
    override_bin: Option<&OsStr>,
    search: &[PathBuf],
    usable: impl Fn(&Path) -> bool,
) -> PathBuf {
    if let Some(binary) = exe.and_then(bundled) {
        return binary;
    }
    if let Some(binary) = override_bin.filter(|s| !s.is_empty()) {
        return binary.into();
    }
    exe.and_then(Path::parent)
        .into_iter()
        .chain(search.iter().map(PathBuf::as_path))
        .map(|dir| dir.join("shuvbolt"))
        .find(|binary| usable(binary))
        .unwrap_or_else(|| "shuvbolt".into())
}

pub fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_wins_even_when_missing_and_override_is_set() {
        let exe = Path::new("/Applications/Shuvbolt.app/Contents/MacOS/Shuvbolt");
        assert_eq!(
            discover(Some(exe), Some(OsStr::new("/other/shuvbolt")), &[], |_| {
                true
            }),
            Path::new("/Applications/Shuvbolt.app/Contents/Helpers/shuvbolt")
        );
        assert!(bundled(Path::new("/tmp/MacOS/Shuvbolt")).is_none());
    }

    #[test]
    fn a_link_shaped_like_a_bundle_is_not_one() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("bin/shuvbolt");
        let link = dir.path().join("Fake.app/Contents/MacOS/Shuvbolt");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::fs::write(&real, "").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(bundled(&link).is_some());
        let exe = resolved(link);
        assert!(bundled(&exe).is_none());
        assert_eq!(
            discover(Some(&exe), Some(OsStr::new("/override")), &[], |_| true),
            Path::new("/override")
        );
        let gone = dir.path().join("gone");
        assert_eq!(resolved(gone.clone()), gone);
    }

    #[test]
    fn helpers_are_recognised_by_shape_alone() {
        for helper in [
            "/Applications/Shuvbolt.app/Contents/Helpers/shuvbolt",
            "/Volumes/Shuvbolt/Shuvbolt.app/Contents/Helpers/shuvbolt",
            "/gone/Shuvbolt 2.app/Contents/Helpers/shuvbolt",
        ] {
            assert!(is_helper(Path::new(helper)), "{helper}");
        }
        for other in [
            "/opt/homebrew/Cellar/shuvbolt/1/bin/shuvbolt",
            "/Applications/Shuvbolt.app/Contents/MacOS/Shuvbolt",
            "/Applications/Shuvbolt/Contents/Helpers/shuvbolt",
            "Contents/Helpers/shuvbolt",
        ] {
            assert!(!is_helper(Path::new(other)), "{other}");
        }
    }

    #[test]
    fn a_finder_launch_still_searches_the_usual_places() {
        let home = Path::new("/Users/sam");
        let dirs = search_dirs(OsStr::new("/usr/bin:/bin:/usr/sbin:/sbin"), Some(home));
        for dir in [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/Users/sam/.cargo/bin",
            "/Users/sam/.local/bin",
        ] {
            assert!(dirs.contains(&PathBuf::from(dir)), "{dir}");
        }
        assert_eq!(dirs[0], Path::new("/usr/bin"));
        let once = search_dirs(OsStr::new("/opt/homebrew/bin:relative"), Some(home));
        assert_eq!(
            once.iter()
                .filter(|d| *d == Path::new("/opt/homebrew/bin"))
                .count(),
            1
        );
        assert!(!once.contains(&PathBuf::from("relative")));
    }

    #[test]
    fn outside_a_bundle_the_daemon_is_the_deadbolt_beside_else_on_the_path() {
        let exe = Path::new("/home/alex/.local/bin/shuvbolt-ui");
        let beside = Path::new("/home/alex/.local/bin/shuvbolt");
        let search = [
            PathBuf::from("/usr/bin"),
            PathBuf::from("/opt/homebrew/bin"),
        ];
        let all = |_: &Path| true;
        assert_eq!(discover(Some(exe), None, &search, all), beside);
        assert_eq!(
            discover(Some(exe), Some(OsStr::new("")), &search, all),
            beside
        );
        assert_eq!(
            discover(Some(exe), Some(OsStr::new("/override")), &search, all),
            Path::new("/override")
        );
        let brew = |p: &Path| p == Path::new("/opt/homebrew/bin/shuvbolt");
        assert_eq!(
            discover(Some(exe), None, &search, brew),
            Path::new("/opt/homebrew/bin/shuvbolt")
        );
        let none = |_: &Path| false;
        assert_eq!(
            discover(Some(exe), None, &search, none),
            Path::new("shuvbolt")
        );
        assert_eq!(discover(None, None, &[], all), Path::new("shuvbolt"));
    }
}
