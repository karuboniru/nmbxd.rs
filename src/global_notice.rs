//! Public service announcements and the acknowledgement of the current revision.
use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{fs, path::PathBuf, time::Duration};

pub const NOTICE_URL: &str = "https://nmb.ovear.info/nmb-notice.json";
const MAX_NOTICE_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalNotice {
    pub content: String,
    pub date: u64,
}

impl GlobalNotice {
    /// Include the body so an edited announcement is new even with the same date.
    pub fn identity(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(self.date.to_be_bytes());
        digest.update(self.content.as_bytes());
        format!("{:x}", digest.finalize())
    }
}

#[derive(Deserialize)]
struct NoticeResponse {
    content: String,
    date: u64,
    enable: bool,
}

fn parse_notice(bytes: &[u8]) -> Result<Option<GlobalNotice>> {
    let response: NoticeResponse = serde_json::from_slice(bytes).context("无法解析全局公告")?;
    if !response.enable
        || crate::model::html_to_text(&response.content)
            .trim()
            .is_empty()
    {
        return Ok(None);
    }
    Ok(Some(GlobalNotice {
        content: response.content,
        date: response.date,
    }))
}

/// Blocks briefly; call on a worker. This client never receives forum cookies.
pub fn fetch_current() -> Result<Option<GlobalNotice>> {
    use std::io::Read as _;

    let response = reqwest::blocking::Client::builder()
        .user_agent(crate::USER_AGENT)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("无法初始化公告客户端")?
        .get(NOTICE_URL)
        .send()
        .context("无法获取全局公告")?
        .error_for_status()
        .context("获取全局公告失败")?;
    let mut bytes = Vec::new();
    response
        .take(MAX_NOTICE_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("无法读取全局公告")?;
    if bytes.len() as u64 > MAX_NOTICE_BYTES {
        bail!("全局公告过大");
    }
    parse_notice(&bytes)
}

#[derive(Default, Deserialize, Serialize)]
struct SeenState {
    seen_identity: Option<String>,
}

pub struct SeenNotice {
    state: SeenState,
    path: Option<PathBuf>,
}

impl Default for SeenNotice {
    fn default() -> Self {
        Self {
            state: SeenState::default(),
            path: crate::config_paths::paths("global-notice.json").map(|paths| paths.0),
        }
    }
}

impl SeenNotice {
    pub fn load() -> Result<Self> {
        let (path, _) =
            crate::config_paths::paths("global-notice.json").context("无法确定公告配置目录")?;
        Self::load_from(path)
    }

    fn load_from(path: PathBuf) -> Result<Self> {
        let state = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("无法解析公告配置")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => SeenState::default(),
            Err(error) => return Err(error).context("无法读取公告配置"),
        };
        Ok(Self {
            state,
            path: Some(path),
        })
    }

    pub fn is_seen(&self, notice: &GlobalNotice) -> bool {
        self.state.seen_identity.as_deref() == Some(notice.identity().as_str())
    }

    /// Save first so a failed write leaves the checkbox's in-memory value intact.
    pub fn set_seen(&mut self, notice: &GlobalNotice, seen: bool) -> Result<()> {
        let state = SeenState {
            seen_identity: if seen {
                Some(notice.identity())
            } else {
                self.state
                    .seen_identity
                    .clone()
                    .filter(|identity| identity != &notice.identity())
            },
        };
        let path = self.path.as_ref().context("无法确定公告配置目录")?;
        let directory = path.parent().context("公告配置目录无效")?;
        let mut temporary =
            crate::config_paths::MigrationFile::new(directory).context("无法创建公告配置文件")?;
        serde_json::to_writer_pretty(&mut temporary.file, &state).context("无法写入公告配置")?;
        temporary.file.sync_all().context("无法同步公告配置")?;
        fs::rename(temporary.path(), path).context("无法替换公告配置文件")?;
        self.state = state;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("rdnmb-notice-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self) -> PathBuf {
            self.0.join("global-notice.json")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn notice(content: &str, date: u64) -> GlobalNotice {
        GlobalNotice {
            content: content.into(),
            date,
        }
    }

    #[test]
    fn accepts_live_schema_and_suppresses_disabled_or_empty_announcements() {
        let current = parse_notice(
            br#"{"content":"<b>Service notice</b><br>More","date":2026102100008,"enable":true}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            current,
            notice("<b>Service notice</b><br>More", 2026102100008)
        );
        for body in [
            r#"{"content":"notice","date":1,"enable":false}"#,
            r#"{"content":"","date":1,"enable":true}"#,
            r#"{"content":"<p> &nbsp; </p><br>","date":1,"enable":true}"#,
        ] {
            assert_eq!(parse_notice(body.as_bytes()).unwrap(), None);
        }
        for body in [
            r#"{}"#,
            r#"{"content":"notice","date":"1","enable":true}"#,
            r#"{"content":"notice","date":-1,"enable":true}"#,
            r#"{"content":null,"date":1,"enable":true}"#,
            r#"{"content":"notice","date":1,"enable":"true"}"#,
        ] {
            assert!(parse_notice(body.as_bytes()).is_err());
        }
    }

    #[test]
    fn acknowledgements_survive_reload_and_new_dates_or_edits_are_unseen() {
        let root = TestDirectory::new();
        let first = notice("<p>First</p>", 2026102100008);
        let changed_date = notice("<p>First</p>", 2026102100009);
        let edited = notice("<p>Edited</p>", 2026102100008);
        let mut seen = SeenNotice::load_from(root.file()).unwrap();
        assert!(!seen.is_seen(&first));
        seen.set_seen(&first, true).unwrap();
        let mut seen = SeenNotice::load_from(root.file()).unwrap();
        assert!(seen.is_seen(&first));
        assert!(!seen.is_seen(&changed_date));
        assert!(!seen.is_seen(&edited));
        seen.set_seen(&edited, true).unwrap();
        let mut seen = SeenNotice::load_from(root.file()).unwrap();
        assert!(!seen.is_seen(&first));
        assert!(seen.is_seen(&edited));
        seen.set_seen(&edited, false).unwrap();
        assert!(!seen.is_seen(&edited));
        assert!(!SeenNotice::load_from(root.file()).unwrap().is_seen(&edited));
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
        let bytes = fs::read(root.file()).unwrap();
        assert!(!String::from_utf8(bytes).unwrap().contains("Edited"));
    }

    #[test]
    fn failed_acknowledgement_save_keeps_memory_and_disk_unchanged() {
        let root = TestDirectory::new();
        let current = notice("Notice", 1);
        let mut seen = SeenNotice::load_from(root.file()).unwrap();
        seen.set_seen(&current, true).unwrap();
        let saved = fs::read(root.file()).unwrap();
        seen.path = Some(root.file().join("impossible.json"));
        assert!(seen.set_seen(&current, false).is_err());
        assert!(seen.is_seen(&current));
        assert_eq!(fs::read(root.file()).unwrap(), saved);
    }
}
