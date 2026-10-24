//! Remember cookie names used successfully in each thread, never cookie values.
use anyhow::{Context as _, Result};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

pub struct ReplyCookies {
    names: BTreeMap<u64, String>,
    path: Option<PathBuf>,
}

impl Default for ReplyCookies {
    fn default() -> Self {
        Self {
            names: BTreeMap::new(),
            path: crate::config_paths::paths("reply-cookies.json").map(|paths| paths.0),
        }
    }
}

impl ReplyCookies {
    pub fn load() -> Result<Self> {
        let (path, _) =
            crate::config_paths::paths("reply-cookies.json").context("无法确定回复饼干配置目录")?;
        Self::load_from(path)
    }

    fn load_from(path: PathBuf) -> Result<Self> {
        let names = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("无法解析回复饼干配置")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error).context("无法读取回复饼干配置"),
        };
        Ok(Self {
            names,
            path: Some(path),
        })
    }

    pub fn selected<'a>(
        &'a self,
        thread: Option<u64>,
        default: Option<&'a str>,
        available: &BTreeMap<String, String>,
    ) -> Option<&'a str> {
        thread
            .and_then(|thread| self.names.get(&thread))
            .map(String::as_str)
            .filter(|name| available.contains_key(*name))
            .or_else(|| default.filter(|name| available.contains_key(*name)))
    }

    pub fn record(&mut self, thread: Option<u64>, name: &str) -> Result<()> {
        let Some(thread) = thread else {
            return Ok(());
        };
        self.names.insert(thread, name.to_owned());
        let path = self.path.as_ref().context("无法确定回复饼干配置目录")?;
        let parent = path.parent().context("回复饼干配置目录无效")?;
        fs::create_dir_all(parent).context("无法创建回复饼干配置目录")?;
        let bytes = serde_json::to_vec_pretty(&self.names).context("无法编码回复饼干配置")?;
        let temporary = parent.join(format!(".reply-cookies-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&temporary)
                .context("无法创建回复饼干配置文件")?;
            file.write_all(&bytes).context("无法写入回复饼干配置")?;
            file.sync_all().context("无法同步回复饼干配置")?;
            drop(file);
            fs::rename(&temporary, path).context("无法替换回复饼干配置文件")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("first".into(), "secret-one".into()),
            ("second".into(), "secret-two".into()),
        ])
    }

    fn path() -> PathBuf {
        std::env::temp_dir()
            .join(format!("rdnmb-reply-cookies-test-{}", uuid::Uuid::new_v4()))
            .join("reply-cookies.json")
    }

    #[test]
    fn remembers_each_thread_and_falls_back_when_name_is_missing() {
        let path = path();
        let mut memory = ReplyCookies::load_from(path.clone()).unwrap();
        let mut available = available();
        assert_eq!(
            memory.selected(Some(1), Some("first"), &available),
            Some("first")
        );
        memory.record(Some(1), "second").unwrap();
        memory.record(Some(2), "first").unwrap();
        assert_eq!(
            memory.selected(Some(1), Some("first"), &available),
            Some("second")
        );
        assert_eq!(
            memory.selected(Some(2), Some("second"), &available),
            Some("first")
        );
        assert_eq!(memory.selected(Some(3), None, &available), None);
        available.remove("second");
        assert_eq!(
            memory.selected(Some(1), Some("first"), &available),
            Some("first")
        );
        assert_eq!(memory.selected(Some(1), Some("second"), &available), None);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn persists_only_names_and_overwrites_atomically() {
        let path = path();
        let mut memory = ReplyCookies::load_from(path.clone()).unwrap();
        memory.record(Some(7), "first").unwrap();
        memory.record(Some(7), "second").unwrap();
        let loaded = ReplyCookies::load_from(path.clone()).unwrap();
        assert_eq!(loaded.selected(Some(7), None, &available()), Some("second"));
        assert_eq!(
            serde_json::from_slice::<BTreeMap<u64, String>>(&fs::read(&path).unwrap()).unwrap(),
            BTreeMap::from([(7, "second".into())])
        );
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn new_threads_do_not_create_persistence_or_override_default() {
        let path = path();
        let mut memory = ReplyCookies::load_from(path.clone()).unwrap();
        memory.record(None, "second").unwrap();
        assert!(!path.exists());
        assert!(memory.names.is_empty());
        assert_eq!(
            memory.selected(None, Some("first"), &available()),
            Some("first")
        );
    }
}
