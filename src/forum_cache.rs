//! Startup metadata, replaced independently after each successful API refresh.
use crate::{
    config_paths::{self, MigrationFile},
    model::{ForumGroup, Timeline},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{fs, path::PathBuf};

const VERSION: u32 = 1;
const GROUPS_FILE: &str = "groups-v1.json";
const TIMELINES_FILE: &str = "timelines-v1.json";

#[derive(Clone, Debug)]
pub struct ForumCache {
    directory: Option<PathBuf>,
}

#[derive(Deserialize, Serialize)]
struct Metadata<T> {
    version: u32,
    data: T,
}

impl Default for ForumCache {
    fn default() -> Self {
        Self {
            directory: config_paths::cache_path("forum-metadata"),
        }
    }
}

impl ForumCache {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: Some(directory.into()),
        }
    }

    pub fn load_groups(&self) -> Vec<ForumGroup> {
        self.load(GROUPS_FILE)
    }

    pub fn load_timelines(&self) -> Vec<Timeline> {
        self.load(TIMELINES_FILE)
    }

    pub fn save_groups(&self, groups: &[ForumGroup]) -> Result<()> {
        self.save(GROUPS_FILE, groups)
    }

    pub fn save_timelines(&self, timelines: &[Timeline]) -> Result<()> {
        self.save(TIMELINES_FILE, timelines)
    }

    fn load<T: DeserializeOwned>(&self, filename: &str) -> Vec<T> {
        let Some(directory) = &self.directory else {
            return Vec::new();
        };
        fs::File::open(directory.join(filename))
            .ok()
            .and_then(|file| serde_json::from_reader::<_, Metadata<Vec<T>>>(file).ok())
            .filter(|metadata| metadata.version == VERSION)
            .map(|metadata| metadata.data)
            .unwrap_or_default()
    }

    fn save<T: Serialize>(&self, filename: &str, data: &[T]) -> Result<()> {
        let directory = self
            .directory
            .as_ref()
            .context("No metadata cache directory is available")?;
        let mut temporary = MigrationFile::new(directory)?;
        serde_json::to_writer(
            &mut temporary.file,
            &Metadata {
                version: VERSION,
                data,
            },
        )?;
        temporary.file.sync_all()?;
        fs::rename(temporary.path(), directory.join(filename))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Forum;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("rdnmb-forum-cache-{}", uuid::Uuid::new_v4())))
        }

        fn cache(&self) -> ForumCache {
            ForumCache::new(&self.0)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn groups() -> Vec<ForumGroup> {
        vec![ForumGroup {
            id: 12,
            name: "讨论区".into(),
            forums: vec![Forum {
                id: 34,
                name: "internal".into(),
                show_name: "<b>综合版</b> &amp; 交流".into(),
                msg: "<p>版规<br>请阅读</p>".into(),
                interval: 30,
            }],
        }]
    }

    fn timelines() -> Vec<Timeline> {
        vec![Timeline {
            id: 56,
            name: "timeline".into(),
            display_name: "<b>时间线</b> &amp; 热门".into(),
            notice: "<p>公告</p>".into(),
            max_page: 500,
        }]
    }

    #[test]
    fn roundtrip_retains_all_display_metadata() {
        let directory = TestDirectory::new();
        let cache = directory.cache();
        cache.save_groups(&groups()).unwrap();
        cache.save_timelines(&timelines()).unwrap();

        let reopened = directory.cache();
        assert_eq!(reopened.load_groups(), groups());
        assert_eq!(reopened.load_timelines(), timelines());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
    }

    #[test]
    fn missing_corrupt_and_unknown_versions_recover_independently() {
        let directory = TestDirectory::new();
        let cache = directory.cache();
        assert!(cache.load_groups().is_empty());
        assert!(cache.load_timelines().is_empty());
        cache.save_groups(&groups()).unwrap();
        cache.save_timelines(&timelines()).unwrap();

        fs::write(directory.0.join(GROUPS_FILE), b"{truncated").unwrap();
        assert!(cache.load_groups().is_empty());
        assert_eq!(cache.load_timelines(), timelines());
        cache.save_groups(&groups()).unwrap();
        fs::write(
            directory.0.join(TIMELINES_FILE),
            br#"{"version":2,"data":[]}"#,
        )
        .unwrap();
        assert!(cache.load_timelines().is_empty());
        assert_eq!(cache.load_groups(), groups());
        cache.save_timelines(&timelines()).unwrap();
        assert_eq!(cache.load_timelines(), timelines());
    }

    #[test]
    fn replacement_is_independent_and_empty_results_are_authoritative() {
        let directory = TestDirectory::new();
        let cache = directory.cache();
        cache.save_groups(&groups()).unwrap();
        cache.save_timelines(&timelines()).unwrap();
        cache.save_groups(&[]).unwrap();
        assert!(cache.load_groups().is_empty());
        assert_eq!(cache.load_timelines(), timelines());
        cache.save_groups(&groups()).unwrap();
        cache.save_timelines(&[]).unwrap();
        assert_eq!(cache.load_groups(), groups());
        assert!(cache.load_timelines().is_empty());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
    }

    #[test]
    fn failed_replacement_cleans_up_its_temporary_file() {
        let directory = TestDirectory::new();
        let cache = directory.cache();
        cache.save_groups(&groups()).unwrap();
        fs::create_dir(directory.0.join(TIMELINES_FILE)).unwrap();

        assert!(cache.save_timelines(&timelines()).is_err());
        assert_eq!(cache.load_groups(), groups());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
    }
}
