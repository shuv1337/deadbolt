//! Self-update is disabled in Shuvbolt. The debug-only fixture feature
//! exercises the retained upstream updater protocol against loopback servers.

mod extension;
mod install;
mod method;
mod notice;
mod protocol;
mod replace;
mod report;
mod service;
mod state;

pub use self::extension::extension_dir_ok;
pub use self::install::{Checked, Etags, UpdateArgs, available, run};
pub use self::notice::notice;
pub(crate) use self::protocol::build_channel;

#[cfg(test)]
use anyhow::Result;
#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::process::Command;
#[cfg(test)]
use url::Url;

#[cfg(test)]
use crate::protocol::UpdateInfo;

#[cfg(test)]
use self::extension::{EXTENSION_LIMIT, EXTENSION_MARKER, extension_version, unpack_extension};
#[cfg(test)]
use self::install::{
    OUTPUT_LINES, npm_from_release, output_tail, platform_package, served_by_npm, static_for,
    upgrade_failure, user_link,
};
#[cfg(test)]
use self::method::{
    CASKROOMS, Install, Manager, PACKAGE, PLATFORM_PACKAGES, classify, installed_with,
    lists_package, manager_order, package_of, upgrade_command,
};
#[cfg(test)]
use self::notice::{NOTICE_EVERY, notice_due, notice_line};
#[cfg(test)]
use self::protocol::{Decision, Reason, Running, Target, decide, rollout_bucket};
#[cfg(test)]
use self::replace::{
    CODESIGN, check_app_zip, link_inside, pax_path, requirement, root_may_update, satisfies,
    signature_required, swap, unpack_cli,
};
#[cfg(test)]
use self::service::{SERVICE, SERVICE_OVERRIDE, file_url, loopback_or};
#[cfg(test)]
use self::state::{UpdateState, check_sequence};

/// No fork team is configured for automated replacement.
const APP_REQUIREMENT: &str = include_str!("../../app/update-requirement.txt");

#[cfg(test)]
mod conformance_tests;
#[cfg(test)]
mod tests;
