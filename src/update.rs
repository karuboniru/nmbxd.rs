//! A best-effort, daily check of the public GitHub release feed.

use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use semver::Version;
use serde::{Deserialize, Serialize};

pub const RELEASE_URL: &str = "https://github.com/karuboniru/nmbxd.rs/releases/latest";
const RELEASE_API_URL: &str = "https://api.github.com/repos/karuboniru/nmbxd.rs/releases/latest";
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Cache {
    last_attempt_unix_secs: Option<u64>,
    latest_version: Option<String>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

impl Release {
    fn stable_tag(self) -> Option<String> {
        (!self.draft && !self.prerelease).then_some(self.tag_name)
    }
}

pub struct UpdateChecker {
    cache_path: Option<PathBuf>,
    cache: Cache,
}

impl Default for UpdateChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateChecker {
    pub fn new() -> Self {
        Self::from_cache_path(crate::config_paths::cache_path("update-check.json"))
    }

    fn from_cache_path(cache_path: Option<PathBuf>) -> Self {
        let cache = cache_path
            .as_ref()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { cache_path, cache }
    }

    /// Recompare on every read so a cached release stops showing after an upgrade.
    pub fn available_version(&self) -> Option<String> {
        self.available_version_for(env!("CARGO_PKG_VERSION"))
    }

    fn available_version_for(&self, current: &str) -> Option<String> {
        let latest = parse_stable_version(self.cache.latest_version.as_deref()?)?;
        let mut current = Version::parse(current).ok()?;
        // Build identifiers do not affect SemVer precedence.
        current.build = semver::BuildMetadata::EMPTY;
        (latest > current).then(|| latest.to_string())
    }

    /// Blocks for at most the HTTP client's timeout; call from a background task.
    /// Network and cache failures never prevent normal application use.
    pub fn check_if_due(&mut self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.check_at(now, fetch_release);
    }

    fn check_at(&mut self, now: u64, fetch: impl FnOnce() -> anyhow::Result<Option<String>>) {
        if self
            .cache
            .last_attempt_unix_secs
            .is_some_and(|previous| now >= previous && now - previous < CHECK_INTERVAL_SECS)
        {
            return;
        }

        // Persist before the request: errors and process exits are throttled too.
        // A timestamp in the future is treated as due after a clock rollback.
        self.cache.last_attempt_unix_secs = Some(now);
        self.save_cache();
        if let Ok(Some(tag)) = fetch()
            && let Some(version) = parse_stable_version(&tag)
        {
            self.cache.latest_version = Some(version.to_string());
            self.save_cache();
        }
    }

    fn save_cache(&self) {
        let Some(path) = &self.cache_path else {
            return;
        };
        let Some(directory) = path.parent() else {
            return;
        };
        let _ = (|| -> anyhow::Result<()> {
            // Keep the temporary beside its target so rename is atomic. The
            // helper also sets private permissions and cleans up failed writes.
            let mut temporary = crate::config_paths::MigrationFile::new(directory)?;
            serde_json::to_writer(&mut temporary.file, &self.cache)?;
            temporary.file.sync_all()?;
            fs::rename(temporary.path(), path)?;
            Ok(())
        })();
    }
}

fn parse_stable_version(tag: &str) -> Option<Version> {
    let mut version = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
    if !version.pre.is_empty() {
        return None;
    }
    version.build = semver::BuildMetadata::EMPTY;
    Some(version)
}

fn fetch_release() -> anyhow::Result<Option<String>> {
    // Deliberately independent of the application's authenticated API client.
    let release: Release = reqwest::blocking::Client::builder()
        .user_agent(crate::USER_AGENT)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()?
        .get(RELEASE_API_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()?
        .error_for_status()?
        .json()?;
    Ok(release.stable_tag())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("rdnmb-update-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn checker(&self) -> UpdateChecker {
            UpdateChecker::from_cache_path(Some(self.0.join("update-check.json")))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn successful_checks_are_throttled_and_persisted() {
        let directory = TestDirectory::new();
        let mut checker = directory.checker();
        checker.check_at(100, || Ok(Some("v2.0.0".into())));
        assert_eq!(checker.available_version_for("1.0.0"), Some("2.0.0".into()));

        let mut reloaded = directory.checker();
        assert_eq!(
            reloaded.available_version_for("1.0.0"),
            Some("2.0.0".into())
        );
        reloaded.check_at(100 + CHECK_INTERVAL_SECS - 1, || panic!("not due"));
        reloaded.check_at(100 + CHECK_INTERVAL_SECS, || Ok(Some("v3.0.0".into())));
        assert_eq!(
            directory.checker().available_version_for("2.0.0"),
            Some("3.0.0".into())
        );
    }

    #[test]
    fn failed_checks_preserve_the_release_and_persist_the_attempt() {
        let directory = TestDirectory::new();
        let mut checker = directory.checker();
        checker.check_at(100, || Ok(Some("2.0.0".into())));
        let failed_at = 100 + CHECK_INTERVAL_SECS;
        checker.check_at(failed_at, || anyhow::bail!("offline"));

        let mut reloaded = directory.checker();
        assert_eq!(
            reloaded.available_version_for("1.0.0"),
            Some("2.0.0".into())
        );
        reloaded.check_at(failed_at + CHECK_INTERVAL_SECS - 1, || panic!("not due"));
        reloaded.check_at(failed_at + CHECK_INTERVAL_SECS, || Ok(Some("3.0.0".into())));
        assert_eq!(
            reloaded.available_version_for("2.0.0"),
            Some("3.0.0".into())
        );
    }

    #[test]
    fn initial_failure_is_throttled_across_restarts() {
        let directory = TestDirectory::new();
        directory
            .checker()
            .check_at(100, || anyhow::bail!("offline"));
        let mut reloaded = directory.checker();
        reloaded.check_at(101, || panic!("failed attempts must be throttled"));
        assert_eq!(reloaded.available_version(), None);
    }

    #[test]
    fn upgrade_and_build_metadata_do_not_leave_a_stale_notice() {
        let mut checker = UpdateChecker::from_cache_path(None);
        checker.check_at(100, || Ok(Some("v2.0.0+published".into())));
        assert_eq!(
            checker.available_version_for("1.99.0"),
            Some("2.0.0".into())
        );
        assert_eq!(checker.available_version_for("2.0.0+local"), None);
        assert_eq!(checker.available_version_for("2.0.1"), None);
        assert_eq!(
            checker.available_version_for("2.0.0-rc.1"),
            Some("2.0.0".into())
        );
    }

    #[test]
    fn versions_use_numeric_semver_ordering() {
        let mut checker = UpdateChecker::from_cache_path(None);
        checker.check_at(0, || Ok(Some("v0.1.10".into())));
        assert_eq!(
            checker.available_version_for("0.1.9"),
            Some("0.1.10".into())
        );
        checker.check_at(CHECK_INTERVAL_SECS, || Ok(Some("0.1.9".into())));
        assert_eq!(checker.available_version_for("0.1.10"), None);
    }

    #[test]
    fn draft_and_prerelease_responses_are_excluded() {
        for (draft, prerelease) in [(true, false), (false, true), (true, true)] {
            let release = Release {
                tag_name: "v2.0.0".into(),
                draft,
                prerelease,
            };
            assert_eq!(release.stable_tag(), None);
        }
        assert_eq!(
            Release {
                tag_name: "v2.0.0".into(),
                draft: false,
                prerelease: false,
            }
            .stable_tag(),
            Some("v2.0.0".into())
        );
    }

    #[test]
    fn invalid_and_prerelease_tags_preserve_a_previous_release() {
        let mut checker = UpdateChecker::from_cache_path(None);
        checker.check_at(0, || Ok(Some("2.0.0".into())));
        for (index, tag) in ["garbage", "2.0", "v3.0.0-rc.1", "V3.0.0", " 3.0.0"]
            .iter()
            .enumerate()
        {
            checker.check_at((index as u64 + 1) * CHECK_INTERVAL_SECS, || {
                Ok(Some((*tag).into()))
            });
            assert_eq!(checker.available_version_for("1.0.0"), Some("2.0.0".into()));
        }
    }

    #[test]
    fn future_cache_timestamp_recovers_after_clock_rollback() {
        let directory = TestDirectory::new();
        directory
            .checker()
            .check_at(u64::MAX, || Ok(Some("2.0.0".into())));
        let mut reloaded = directory.checker();
        reloaded.check_at(100, || Ok(Some("3.0.0".into())));
        assert_eq!(
            reloaded.available_version_for("2.0.0"),
            Some("3.0.0".into())
        );
        directory
            .checker()
            .check_at(101, || panic!("rollback must reestablish throttle"));
    }

    #[test]
    fn corrupt_cache_and_unwritable_cache_are_nonfatal() {
        let directory = TestDirectory::new();
        fs::write(directory.0.join("update-check.json"), b"not json").unwrap();
        let mut checker = directory.checker();
        assert_eq!(checker.available_version(), None);
        checker.check_at(100, || Ok(Some("2.0.0".into())));
        assert_eq!(
            directory.checker().available_version_for("1.0.0"),
            Some("2.0.0".into())
        );

        // A regular file used as a parent fails even when tests run as root.
        let mut unwritable =
            UpdateChecker::from_cache_path(Some(directory.0.join("update-check.json/child")));
        unwritable.check_at(100, || Ok(Some("2.0.0".into())));
        assert_eq!(
            unwritable.available_version_for("1.0.0"),
            Some("2.0.0".into())
        );
        unwritable.check_at(101, || panic!("in-memory throttle still applies"));
    }

    #[test]
    fn no_eligible_release_preserves_previous_version_and_throttles() {
        let mut checker = UpdateChecker::from_cache_path(None);
        checker.check_at(0, || Ok(Some("2.0.0".into())));
        checker.check_at(CHECK_INTERVAL_SECS, || Ok(None));
        checker.check_at(CHECK_INTERVAL_SECS + 1, || panic!("not due"));
        assert_eq!(checker.available_version_for("1.0.0"), Some("2.0.0".into()));
    }
}
