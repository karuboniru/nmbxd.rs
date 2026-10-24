//! Private, persistent browsing history and thread-page snapshots.
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::model::Post;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadingPosition {
    pub page: u32,
    pub offset: f32,
    pub anchor: Option<u64>,
    pub anchor_offset: f32,
    pub aggregate: bool,
    pub first_page: u32,
    pub last_page: u32,
}

impl Default for ReadingPosition {
    fn default() -> Self {
        Self {
            page: 1,
            offset: 0.,
            anchor: None,
            anchor_offset: 0.,
            aggregate: false,
            first_page: 1,
            last_page: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BrowseEntry {
    pub thread_id: u64,
    pub title: String,
    pub preview: String,
    pub forum_id: i64,
    pub forum_name: String,
    pub visited_at: i64,
    pub position: ReadingPosition,
    pub cached_pages: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostingEntry {
    pub id: i64,
    pub target: Option<u64>,
    pub forum_id: i64,
    pub title: String,
    pub body: String,
    pub has_image: bool,
    pub sent_at: i64,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryCategory {
    Browsing,
    Threads,
    Replies,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HistoryEntry {
    Browse(BrowseEntry),
    Posted(PostingEntry),
}

#[derive(Clone)]
pub struct HistoryStore {
    connection: Arc<Mutex<Connection>>,
}

const BROWSE_COLUMNS: &str = "b.thread_id, b.title, b.preview, b.forum_id, b.forum_name,
    b.visited_at, b.position, (SELECT COUNT(*) FROM cached_pages c WHERE c.thread_id = b.thread_id)";

impl HistoryStore {
    pub fn open_default() -> Result<Self> {
        let path = crate::config_paths::data_path("history.sqlite3")
            .context("cannot determine history data directory")?;
        let sources = crate::config_paths::paths("history.sqlite3")
            .map(|(current, legacy)| vec![current, legacy])
            .unwrap_or_default();
        migrate_history(&path, &sources)?;
        Self::open(&path)
    }

    /// Open a database file, creating its private parent directory if needed.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(parent).context("create history directory")?;
        }
        // Create with restrictive permissions before SQLite can open a new file.
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("create history database"),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .context("secure history database permissions")?;
        }
        let mut connection = Connection::open(path).context("open history database")?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .context("configure history busy timeout")?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .context("enable history write-ahead log")?;
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .context("read history schema version")?;
        if version > 1 {
            bail!("history database schema is newer than this application supports");
        }
        if version == 0 {
            let transaction = connection
                .transaction()
                .context("begin history migration")?;
            transaction
                .execute_batch(
                    "CREATE TABLE browsing (
                    thread_id INTEGER PRIMARY KEY CHECK(thread_id >= 0),
                    title TEXT NOT NULL, preview TEXT NOT NULL,
                    forum_id INTEGER NOT NULL, forum_name TEXT NOT NULL,
                    visited_at INTEGER NOT NULL, position TEXT NOT NULL
                );
                CREATE INDEX browsing_by_visit ON browsing(visited_at DESC, thread_id DESC);
                CREATE TABLE cached_pages (
                    thread_id INTEGER NOT NULL CHECK(thread_id >= 0),
                    page INTEGER NOT NULL CHECK(page > 0),
                    post TEXT NOT NULL,
                    PRIMARY KEY(thread_id, page)
                );
                CREATE TABLE posting (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    target INTEGER CHECK(target >= 0), forum_id INTEGER NOT NULL,
                    title TEXT NOT NULL, body TEXT NOT NULL, has_image INTEGER NOT NULL,
                    sent_at INTEGER NOT NULL, message TEXT NOT NULL
                );
                CREATE INDEX posting_by_send ON posting(sent_at DESC, id DESC);
                PRAGMA user_version = 1;",
                )
                .context("initialize history tables")?;
            transaction.commit().context("commit history migration")?;
        }
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("history database lock poisoned"))
    }

    pub fn cached_page(&self, thread: u64, page: u32) -> Result<Option<Post>> {
        let thread = sqlite_id(thread)?;
        let connection = self.connection()?;
        let json: Option<String> = connection
            .query_row(
                "SELECT post FROM cached_pages WHERE thread_id = ?1 AND page = ?2",
                params![thread, page],
                |row| row.get(0),
            )
            .optional()
            .context("read cached thread page")?;
        let Some(json) = json else { return Ok(None) };
        match serde_json::from_str(&json) {
            Ok(post) => Ok(Some(post)),
            Err(_) => {
                connection
                    .execute(
                        "DELETE FROM cached_pages WHERE thread_id = ?1 AND page = ?2",
                        params![thread, page],
                    )
                    .context("discard damaged cached thread page")?;
                Ok(None)
            }
        }
    }

    pub fn cache_page(&self, thread: u64, page: u32, post: &Post) -> Result<()> {
        let thread = sqlite_id(thread)?;
        if page == 0 {
            bail!("cached thread page must be at least one");
        }
        let json = serde_json::to_string(post).context("serialize thread page")?;
        self.connection()?
            .execute(
                "INSERT INTO cached_pages(thread_id, page, post) VALUES (?1, ?2, ?3)
             ON CONFLICT(thread_id, page) DO UPDATE SET post = excluded.post",
                params![thread, page, json],
            )
            .context("cache thread page")?;
        Ok(())
    }

    pub fn clear_thread_cache(&self, thread: u64) -> Result<Vec<String>> {
        let thread = sqlite_id(thread)?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .context("begin thread cache removal")?;
        let mut urls = BTreeSet::new();
        {
            let mut statement = transaction
                .prepare("SELECT post FROM cached_pages WHERE thread_id = ?1")
                .context("read thread cache for removal")?;
            let rows = statement
                .query_map([thread], |row| row.get::<_, String>(0))
                .context("list cached thread pages")?;
            for row in rows {
                if let Ok(post) =
                    serde_json::from_str::<Post>(&row.context("read cached page snapshot")?)
                {
                    collect_image_urls(&post, &mut urls);
                }
            }
        }
        transaction
            .execute("DELETE FROM cached_pages WHERE thread_id = ?1", [thread])
            .context("remove cached thread pages")?;
        transaction
            .commit()
            .context("commit thread cache removal")?;
        Ok(urls.into_iter().collect())
    }

    pub fn record_visit(
        &self,
        thread: u64,
        title: &str,
        preview: &str,
        forum_id: i64,
        forum_name: &str,
    ) -> Result<()> {
        let thread = sqlite_id(thread)?;
        let position = serde_json::to_string(&ReadingPosition::default())
            .context("serialize default reading position")?;
        self.connection()?.execute(
            "INSERT INTO browsing(thread_id, title, preview, forum_id, forum_name, visited_at, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(thread_id) DO UPDATE SET title = excluded.title,
                 preview = excluded.preview, forum_id = excluded.forum_id,
                 forum_name = excluded.forum_name, visited_at = excluded.visited_at",
            params![thread, title, preview, forum_id, forum_name, now_millis(), position],
        ).context("record thread visit")?;
        Ok(())
    }

    pub fn save_position(&self, thread: u64, position: &ReadingPosition) -> Result<()> {
        let thread = sqlite_id(thread)?;
        if !position.offset.is_finite() || !position.anchor_offset.is_finite() {
            bail!("reading position offsets must be finite");
        }
        if position.page == 0
            || position.first_page == 0
            || position.last_page < position.first_page
        {
            bail!("reading position page range is invalid");
        }
        let json = serde_json::to_string(position).context("serialize reading position")?;
        self.connection()?
            .execute(
                "UPDATE browsing SET position = ?2 WHERE thread_id = ?1",
                params![thread, json],
            )
            .context("save reading position")?;
        Ok(())
    }

    pub fn last_visit(&self, thread: u64) -> Result<Option<BrowseEntry>> {
        let thread = sqlite_id(thread)?;
        self.connection()?
            .query_row(
                &format!("SELECT {BROWSE_COLUMNS} FROM browsing b WHERE b.thread_id = ?1"),
                [thread],
                browse_row,
            )
            .optional()
            .context("read last thread visit")
    }

    pub fn list(
        &self,
        category: HistoryCategory,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<HistoryEntry>> {
        let connection = self.connection()?;
        match category {
            HistoryCategory::Browsing => {
                let mut statement = connection
                    .prepare(&format!(
                        "SELECT {BROWSE_COLUMNS} FROM browsing b
                     ORDER BY b.visited_at DESC, b.thread_id DESC LIMIT ?1 OFFSET ?2",
                    ))
                    .context("prepare browsing history")?;
                statement
                    .query_map(params![limit, offset], |row| {
                        browse_row(row).map(HistoryEntry::Browse)
                    })
                    .context("list browsing history")?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .context("read browsing history")
            }
            HistoryCategory::Threads | HistoryCategory::Replies => {
                let target_filter = if category == HistoryCategory::Threads {
                    "IS NULL"
                } else {
                    "IS NOT NULL"
                };
                let mut statement = connection
                    .prepare(&format!(
                        "SELECT id, target, forum_id, title, body, has_image, sent_at, message
                     FROM posting WHERE target {target_filter}
                     ORDER BY sent_at DESC, id DESC LIMIT ?1 OFFSET ?2",
                    ))
                    .context("prepare posting history")?;
                statement
                    .query_map(params![limit, offset], |row| {
                        Ok(HistoryEntry::Posted(PostingEntry {
                            id: row.get(0)?,
                            target: row
                                .get::<_, Option<i64>>(1)?
                                .map(|id| unsigned_id(id, 1))
                                .transpose()?,
                            forum_id: row.get(2)?,
                            title: row.get(3)?,
                            body: row.get(4)?,
                            has_image: row.get(5)?,
                            sent_at: row.get(6)?,
                            message: row.get(7)?,
                        }))
                    })
                    .context("list posting history")?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .context("read posting history")
            }
        }
    }

    /// Append one confirmed successful submission. `id` is assigned by SQLite.
    pub fn record_post(&self, entry: &PostingEntry) -> Result<()> {
        let target = entry.target.map(sqlite_id).transpose()?;
        self.connection()?
            .execute(
                "INSERT INTO posting(target, forum_id, title, body, has_image, sent_at, message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    target,
                    entry.forum_id,
                    entry.title,
                    entry.body,
                    entry.has_image,
                    entry.sent_at,
                    entry.message
                ],
            )
            .context("record successful post")?;
        Ok(())
    }
}

/// Snapshot through SQLite so committed WAL pages are included. Never copy a
/// live database and its sidecars independently, or replace an occupied target.
fn migrate_history(destination: &Path, sources: &[std::path::PathBuf]) -> Result<()> {
    if !history_destination_available(destination)? {
        return Ok(());
    }
    for source in sources {
        match fs::symlink_metadata(source) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("inspect former history database"),
        }
        let connection =
            Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .context("open former history database for migration")?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let directory = destination.parent().context("history path has no parent")?;
        let temporary = crate::config_paths::MigrationFile::new(directory)
            .context("create temporary history snapshot")?;
        #[cfg(unix)]
        let snapshot_path = {
            use std::os::unix::ffi::OsStrExt;
            temporary.path().as_os_str().as_bytes()
        };
        #[cfg(not(unix))]
        let snapshot_path = temporary
            .path()
            .to_str()
            .context("history migration path is not valid UTF-8")?
            .as_bytes();
        let filename =
            rusqlite::types::ToSqlOutput::Borrowed(rusqlite::types::ValueRef::Text(snapshot_path));
        connection
            .execute("VACUUM INTO ?1", [filename])
            .context("snapshot former history database")?;
        if history_destination_available(destination)? {
            temporary
                .publish(destination)
                .context("publish migrated history database")?;
        }
        return Ok(());
    }
    Ok(())
}

fn history_destination_available(destination: &Path) -> Result<bool> {
    match fs::symlink_metadata(destination) {
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("inspect history destination"),
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = destination.as_os_str().to_os_string();
        sidecar.push(suffix);
        match fs::symlink_metadata(Path::new(&sidecar)) {
            Ok(_) => bail!("history database is absent but its {suffix} sidecar already exists"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("inspect history sidecar"),
        }
    }
    Ok(true)
}

fn sqlite_id(id: u64) -> Result<i64> {
    i64::try_from(id).context("thread ID exceeds SQLite integer range")
}

fn unsigned_id(id: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(id).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, id))
}

fn browse_row(row: &Row<'_>) -> rusqlite::Result<BrowseEntry> {
    let position: String = row.get(6)?;
    Ok(BrowseEntry {
        thread_id: unsigned_id(row.get(0)?, 0)?,
        title: row.get(1)?,
        preview: row.get(2)?,
        forum_id: row.get(3)?,
        forum_name: row.get(4)?,
        visited_at: row.get(5)?,
        position: serde_json::from_str(&position).unwrap_or_default(),
        cached_pages: row.get(7)?,
    })
}

fn collect_image_urls(post: &Post, urls: &mut BTreeSet<String>) {
    urls.extend(post.image_url(false));
    urls.extend(post.image_url(true));
    for reply in &post.replies {
        collect_image_urls(reply, urls);
    }
}

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TestDb(PathBuf);

    impl TestDb {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("rdnmb-history-{}", uuid::Uuid::new_v4())))
        }

        fn path(&self) -> PathBuf {
            self.0.join("history.sqlite3")
        }

        fn open(&self) -> HistoryStore {
            HistoryStore::open(&self.path()).unwrap()
        }
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn post(id: u64) -> Post {
        Post {
            id,
            fid: 4,
            title: "标题".into(),
            content: "<b>正文</b>".into(),
            img: format!("2026-10-23/{id}"),
            ext: ".png".into(),
            replies: vec![Post {
                id: id + 1,
                sage: true,
                ..Post::default()
            }],
            ..Post::default()
        }
    }

    #[test]
    fn migration_snapshots_live_wal_without_removing_the_original() {
        let db = TestDb::new();
        fs::create_dir_all(&db.0).unwrap();
        let source = db.0.join("old.sqlite3");
        let connection = Connection::open(&source).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
            CREATE TABLE sample(value TEXT); INSERT INTO sample VALUES ('committed in WAL');",
            )
            .unwrap();
        assert!(fs::metadata(db.0.join("old.sqlite3-wal")).unwrap().len() > 0);
        migrate_history(&db.path(), std::slice::from_ref(&source)).unwrap();
        let migrated = Connection::open(db.path()).unwrap();
        assert_eq!(
            migrated
                .query_row("SELECT value FROM sample", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "committed in WAL"
        );
        assert!(source.is_file());
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM sample", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn migration_uses_current_then_legacy_and_preserves_existing_database() {
        let db = TestDb::new();
        fs::create_dir_all(&db.0).unwrap();
        let current = db.0.join("rdnmb.sqlite3");
        let legacy = db.0.join("nmbxd.sqlite3");
        for (path, value) in [(&current, "current"), (&legacy, "legacy")] {
            let connection = Connection::open(path).unwrap();
            connection
                .execute_batch("CREATE TABLE sample(value TEXT)")
                .unwrap();
            connection
                .execute("INSERT INTO sample VALUES (?1)", [value])
                .unwrap();
        }
        let sources = [current.clone(), legacy.clone()];
        migrate_history(&db.path(), &sources).unwrap();
        let value = |path: &Path| {
            Connection::open(path)
                .unwrap()
                .query_row("SELECT value FROM sample", [], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap()
        };
        assert_eq!(value(&db.path()), "current");
        migrate_history(&db.path(), std::slice::from_ref(&legacy)).unwrap();
        assert_eq!(value(&db.path()), "current");
        fs::remove_file(&current).unwrap();
        let fallback = db.0.join("fallback.sqlite3");
        migrate_history(&fallback, &sources).unwrap();
        assert_eq!(value(&fallback), "legacy");
    }

    #[test]
    fn failed_history_migration_leaves_no_partial_destination_or_temporary() {
        let db = TestDb::new();
        fs::create_dir_all(&db.0).unwrap();
        let source = db.0.join("corrupt.sqlite3");
        fs::write(&source, b"not a database").unwrap();
        assert!(migrate_history(&db.path(), &[source]).is_err());
        assert!(!db.path().exists());
        assert!(!fs::read_dir(&db.0).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".rdnmb-migrate-")
        }));
    }

    #[test]
    fn orphan_destination_sidecars_are_preserved_and_block_migration() {
        let db = TestDb::new();
        fs::create_dir_all(&db.0).unwrap();
        for suffix in ["-wal", "-shm", "-journal"] {
            let path = db.0.join(format!("history.sqlite3{suffix}"));
            fs::write(&path, b"existing sidecar").unwrap();
            assert!(migrate_history(&db.path(), &[]).is_err());
            assert_eq!(fs::read(&path).unwrap(), b"existing sidecar");
            assert!(!db.path().exists());
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn reopen_preserves_snapshots_visits_positions_and_successful_posts() {
        let db = TestDb::new();
        let position = ReadingPosition {
            page: 3,
            offset: -159.5,
            anchor: Some(12),
            anchor_offset: -20.,
            aggregate: true,
            first_page: 2,
            last_page: 4,
        };
        let snapshot = post(10);
        {
            let store = db.open();
            store.cache_page(10, 3, &snapshot).unwrap();
            store
                .record_visit(10, "title", "preview", -1, "forum")
                .unwrap();
            store.save_position(10, &position).unwrap();
            let visited_at = store.last_visit(10).unwrap().unwrap().visited_at;
            // Saving scroll state never makes a thread appear newly visited.
            store.save_position(10, &position).unwrap();
            assert_eq!(
                store.last_visit(10).unwrap().unwrap().visited_at,
                visited_at
            );
            store
                .record_visit(10, "new title", "new preview", 4, "updated forum")
                .unwrap();
            store
                .record_post(&PostingEntry {
                    title: "sent title".into(),
                    body: "sent body".into(),
                    sent_at: 42,
                    message: "success".into(),
                    has_image: true,
                    ..PostingEntry::default()
                })
                .unwrap();
        }
        let store = db.open();
        assert_eq!(store.cached_page(10, 3).unwrap(), Some(snapshot));
        let visit = store.last_visit(10).unwrap().unwrap();
        assert_eq!(visit.position, position);
        assert_eq!(visit.title, "new title");
        assert_eq!(visit.preview, "new preview");
        assert_eq!(visit.forum_name, "updated forum");
        assert_eq!(visit.cached_pages, 1);
        let entries = store.list(HistoryCategory::Threads, 10, 0).unwrap();
        let HistoryEntry::Posted(entry) = &entries[0] else {
            panic!("expected posted history")
        };
        assert!(entry.id > 0);
        assert_eq!(entry.body, "sent body");
        assert!(entry.has_image);
        assert_eq!(entry.message, "success");
    }

    #[test]
    fn clearing_only_selected_thread_keeps_history_and_returns_unique_media_urls() {
        let db = TestDb::new();
        let store = db.open();
        let mut first = post(10);
        first.replies.push(post(20));
        store
            .record_visit(10, "visited", "saved preview", 4, "forum")
            .unwrap();
        store.record_visit(30, "other", "", 4, "forum").unwrap();
        store.cache_page(10, 1, &first).unwrap();
        store.cache_page(10, 2, &first).unwrap();
        store.cache_page(30, 1, &post(30)).unwrap();
        let urls = store.clear_thread_cache(10).unwrap();
        assert_eq!(urls.len(), 4);
        assert!(urls.contains(&first.image_url(false).unwrap()));
        assert!(urls.contains(&first.replies[1].image_url(true).unwrap()));
        assert_eq!(store.cached_page(10, 1).unwrap(), None);
        assert_eq!(store.cached_page(10, 2).unwrap(), None);
        assert!(store.cached_page(30, 1).unwrap().is_some());
        let visit = store.last_visit(10).unwrap().unwrap();
        assert_eq!(visit.preview, "saved preview");
        assert_eq!(visit.cached_pages, 0);
        assert_eq!(
            store.list(HistoryCategory::Browsing, 10, 0).unwrap().len(),
            2
        );
        assert!(store.clear_thread_cache(10).unwrap().is_empty());
    }

    #[test]
    fn categories_sort_by_timestamp_then_id_and_support_pagination() {
        let db = TestDb::new();
        let store = db.open();
        for id in [1, 3, 2] {
            store.record_visit(id, "", "", 1, "").unwrap();
        }
        store
            .connection()
            .unwrap()
            .execute("UPDATE browsing SET visited_at = 9", [])
            .unwrap();
        let visits = store.list(HistoryCategory::Browsing, 1, 1).unwrap();
        assert!(matches!(&visits[0], HistoryEntry::Browse(entry) if entry.thread_id == 2));
        for (target, sent_at, body) in [
            (None, 40, "first"),
            (Some(5), 50, "reply"),
            (None, 30, "older"),
            (None, 40, "last"),
        ] {
            store
                .record_post(&PostingEntry {
                    target,
                    sent_at,
                    body: body.into(),
                    ..PostingEntry::default()
                })
                .unwrap();
        }
        let threads = store.list(HistoryCategory::Threads, 10, 0).unwrap();
        let bodies: Vec<_> = threads
            .iter()
            .map(|entry| match entry {
                HistoryEntry::Posted(entry) => entry.body.as_str(),
                _ => panic!("expected sent entry"),
            })
            .collect();
        assert_eq!(bodies, ["last", "first", "older"]);
        let replies = store.list(HistoryCategory::Replies, 10, 0).unwrap();
        assert!(matches!(&replies[0], HistoryEntry::Posted(entry) if entry.target == Some(5)));
        assert_eq!(replies.len(), 1);
    }

    #[test]
    fn damaged_cache_is_evicted_and_does_not_damage_history() {
        let db = TestDb::new();
        let store = db.open();
        store
            .record_visit(10, "title", "preview", 4, "forum")
            .unwrap();
        store.cache_page(10, 1, &post(10)).unwrap();
        store
            .connection()
            .unwrap()
            .execute("UPDATE cached_pages SET post = 'not JSON'", [])
            .unwrap();
        assert!(store.cached_page(10, 1).unwrap().is_none());
        assert_eq!(store.last_visit(10).unwrap().unwrap().cached_pages, 0);
        store.cache_page(10, 1, &post(10)).unwrap();
        store
            .connection()
            .unwrap()
            .execute("UPDATE cached_pages SET post = '{}'", [])
            .unwrap();
        assert!(store.clear_thread_cache(10).unwrap().is_empty());
        assert_eq!(store.last_visit(10).unwrap().unwrap().title, "title");
    }

    #[test]
    fn replacing_and_failed_replacing_or_clearing_preserve_complete_snapshots() {
        let db = TestDb::new();
        let store = db.open();
        store.cache_page(10, 1, &post(10)).unwrap();
        let replacement = post(20);
        store.cache_page(10, 1, &replacement).unwrap();
        assert_eq!(store.cached_page(10, 1).unwrap(), Some(replacement.clone()));
        store
            .connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER reject_replacement BEFORE UPDATE ON cached_pages
             BEGIN SELECT RAISE(ABORT, 'test failure'); END;
             CREATE TRIGGER reject_clear BEFORE DELETE ON cached_pages
             BEGIN SELECT RAISE(ABORT, 'test failure'); END;",
            )
            .unwrap();
        assert!(store.cache_page(10, 1, &post(30)).is_err());
        assert!(store.clear_thread_cache(10).is_err());
        assert_eq!(store.cached_page(10, 1).unwrap(), Some(replacement));
    }

    #[test]
    fn invalid_ids_and_positions_are_rejected_without_partial_changes() {
        let db = TestDb::new();
        let store = db.open();
        assert!(store.record_visit(u64::MAX, "", "", 1, "").is_err());
        assert!(store.cache_page(1, 0, &post(1)).is_err());
        store.record_visit(1, "", "", 1, "").unwrap();
        let position = ReadingPosition {
            offset: f32::NAN,
            ..ReadingPosition::default()
        };
        assert!(store.save_position(1, &position).is_err());
        assert_eq!(
            store.last_visit(1).unwrap().unwrap().position,
            ReadingPosition::default()
        );
        assert!(
            store
                .record_post(&PostingEntry {
                    target: Some(u64::MAX),
                    ..PostingEntry::default()
                })
                .is_err()
        );
        assert!(
            store
                .list(HistoryCategory::Replies, 10, 0)
                .unwrap()
                .is_empty()
        );
    }

    #[cfg(unix)]
    #[test]
    fn database_and_new_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let db = TestDb::new();
        let _store = db.open();
        assert_eq!(
            fs::metadata(&db.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(db.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
