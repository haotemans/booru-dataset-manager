use bdm_domain::catalog::{AddCollectionResult, CollectionKind, CollectionRecord, SourceTag};
use bdm_ports::{CatalogError, CollectionRepository};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

const MIGRATION: &str = include_str!("migrations/001_collections.sql");
const MEDIA_MIGRATION: &str = include_str!("migrations/002_media.sql");
const WORKFLOW_MIGRATION: &str = include_str!("migrations/003_workflows.sql");
const DISCOVERY_MIGRATION: &str = include_str!("migrations/004_discovered_artists.sql");

pub struct SqliteCatalog {
    pub(crate) connection: Connection,
    pub(crate) media_lock: std::sync::Arc<std::sync::Mutex<()>>,
}

impl SqliteCatalog {
    pub fn open(path: &Path) -> Result<Self, CatalogError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|_| CatalogError::Storage)?;
        }
        let connection = Connection::open(path).map_err(|_| CatalogError::Storage)?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|_| CatalogError::Schema)?;
        if (1..4).contains(&version) {
            let backup = path.with_extension(format!("pre-v{version}.sqlite3"));
            if !backup.exists() {
                connection
                    .backup(rusqlite::DatabaseName::Main, &backup, None)
                    .map_err(|_| CatalogError::Storage)?;
            }
        }
        let mut catalog = Self::initialize(connection)?;
        catalog.media_lock = crate::coordination::media_lock(path);
        Ok(catalog)
    }

    pub fn in_memory() -> Result<Self, CatalogError> {
        Self::initialize(Connection::open_in_memory().map_err(|_| CatalogError::Storage)?)
    }

    fn initialize(mut connection: Connection) -> Result<Self, CatalogError> {
        connection
            .busy_timeout(Duration::from_secs(3))
            .map_err(|_| CatalogError::Storage)?;
        connection
            // Migration 4 preserves collection IDs while rebuilding nullable source fields.
            .pragma_update(None, "foreign_keys", false)
            .map_err(|_| CatalogError::Storage)?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| CatalogError::Storage)?;
        let version: i64 = tx
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|_| CatalogError::Schema)?;
        let checksum = format!(
            "{:x}",
            Sha256::digest(MIGRATION.replace("\r\n", "\n").as_bytes())
        );
        match version {
            0 => {
                let tables: i64 = tx.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |row| row.get(0)).map_err(|_| CatalogError::Schema)?;
                if tables != 0 {
                    return Err(CatalogError::Schema);
                }
                tx.execute_batch(MIGRATION)
                    .map_err(|_| CatalogError::Schema)?;
                tx.execute(
                    "INSERT INTO schema_migrations(version, checksum) VALUES(1, ?1)",
                    [&checksum],
                )
                .map_err(|_| CatalogError::Schema)?;
            }
            1..=4 => {
                let saved: String = tx
                    .query_row(
                        "SELECT checksum FROM schema_migrations WHERE version=1",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(|_| CatalogError::Schema)?;
                if saved != checksum {
                    return Err(CatalogError::Schema);
                }
            }
            _ => return Err(CatalogError::Schema),
        }
        let media_checksum = format!(
            "{:x}",
            Sha256::digest(MEDIA_MIGRATION.replace("\r\n", "\n").as_bytes())
        );
        if version < 2 {
            tx.execute_batch(MEDIA_MIGRATION)
                .map_err(|_| CatalogError::Schema)?;
            tx.execute(
                "INSERT INTO schema_migrations(version, checksum) VALUES(2, ?1)",
                [&media_checksum],
            )
            .map_err(|_| CatalogError::Schema)?;
        } else {
            let saved: String = tx
                .query_row(
                    "SELECT checksum FROM schema_migrations WHERE version=2",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| CatalogError::Schema)?;
            if saved != media_checksum {
                return Err(CatalogError::Schema);
            }
        }
        let workflow_checksum = format!(
            "{:x}",
            Sha256::digest(WORKFLOW_MIGRATION.replace("\r\n", "\n").as_bytes())
        );
        if version < 3 {
            tx.execute_batch(WORKFLOW_MIGRATION)
                .map_err(|_| CatalogError::Schema)?;
            tx.execute(
                "INSERT INTO schema_migrations(version, checksum) VALUES(3, ?1)",
                [&workflow_checksum],
            )
            .map_err(|_| CatalogError::Schema)?;
        } else {
            let saved: String = tx
                .query_row(
                    "SELECT checksum FROM schema_migrations WHERE version=3",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| CatalogError::Schema)?;
            if saved != workflow_checksum {
                return Err(CatalogError::Schema);
            }
        }
        let discovery_checksum = format!(
            "{:x}",
            Sha256::digest(DISCOVERY_MIGRATION.replace("\r\n", "\n").as_bytes())
        );
        if version < 4 {
            tx.execute_batch(DISCOVERY_MIGRATION)
                .map_err(|_| CatalogError::Schema)?;
            tx.execute(
                "INSERT INTO schema_migrations(version, checksum) VALUES(4, ?1)",
                [&discovery_checksum],
            )
            .map_err(|_| CatalogError::Schema)?;
        } else {
            let saved: String = tx
                .query_row(
                    "SELECT checksum FROM schema_migrations WHERE version=4",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| CatalogError::Schema)?;
            if saved != discovery_checksum {
                return Err(CatalogError::Schema);
            }
        }
        let broken: bool = tx
            .prepare("PRAGMA foreign_key_check")
            .and_then(|mut query| Ok(query.query([])?.next()?.is_some()))
            .map_err(|_| CatalogError::Schema)?;
        if broken {
            return Err(CatalogError::Schema);
        }
        let integrity: String = tx
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(|_| CatalogError::Schema)?;
        if integrity != "ok" {
            return Err(CatalogError::Schema);
        }
        tx.commit().map_err(|_| CatalogError::Storage)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(|_| CatalogError::Storage)?;
        Ok(Self {
            connection,
            media_lock: Default::default(),
        })
    }
}

fn record(row: &rusqlite::Row<'_>) -> rusqlite::Result<CollectionRecord> {
    let category: u8 = row.get(2)?;
    let kind = match category {
        1 => CollectionKind::Artist,
        4 => CollectionKind::Character,
        _ => {
            return Err(rusqlite::Error::IntegralValueOutOfRange(
                2,
                i64::from(category),
            ))
        }
    };
    let name: String = row.get(3)?;
    let source_id: Option<i64> = row.get(1)?;
    let post_count: Option<i64> = row.get(4)?;
    Ok(CollectionRecord {
        id: row.get(0)?,
        kind,
        name: name.clone(),
        source: source_id.map(|id| SourceTag {
            id,
            category,
            name,
            post_count: post_count.expect("paired source identity and count constraint"),
        }),
    })
}

impl CollectionRepository for SqliteCatalog {
    fn list(&self) -> Result<Vec<CollectionRecord>, CatalogError> {
        self.connection.prepare("SELECT id, source_tag_id, category, tag, remote_post_count FROM collections ORDER BY id")
            .and_then(|mut query| query.query_map([], record)?.collect()).map_err(|_| CatalogError::Storage)
    }

    fn add(
        &mut self,
        kind: CollectionKind,
        tag: &SourceTag,
    ) -> Result<AddCollectionResult, CatalogError> {
        if tag.category != kind.category() {
            return Err(CatalogError::WrongCategory);
        }
        if tag.id <= 0 || tag.post_count < 0 || tag.name.is_empty() {
            return Err(CatalogError::InvalidResponse);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| CatalogError::Storage)?;
        let existing = tx.query_row("SELECT id, source_tag_id, category, tag, remote_post_count FROM collections WHERE provider='danbooru' AND (source_tag_id=?1 OR tag=?2)", params![tag.id, tag.name], record).optional().map_err(|_| CatalogError::Storage)?;
        if let Some(record) = existing {
            if record.kind != kind
                || record
                    .source
                    .as_ref()
                    .is_some_and(|source| source.id != tag.id)
            {
                return Err(CatalogError::Changed);
            }
            if record.source.is_none() {
                tx.execute("UPDATE collections SET source_tag_id=?2, remote_post_count=?3, fetched_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1", params![record.id, tag.id, tag.post_count]).map_err(|_| CatalogError::Storage)?;
                tx.commit().map_err(|_| CatalogError::Storage)?;
                return Ok(AddCollectionResult {
                    record: CollectionRecord {
                        source: Some(tag.clone()),
                        ..record
                    },
                    created: false,
                });
            }
            return Ok(AddCollectionResult {
                record,
                created: false,
            });
        }
        tx.execute("INSERT INTO collections(provider, source_tag_id, category, tag, remote_post_count) VALUES('danbooru', ?1, ?2, ?3, ?4)", params![tag.id, tag.category, tag.name, tag.post_count]).map_err(|_| CatalogError::Storage)?;
        let record = CollectionRecord {
            id: tx.last_insert_rowid(),
            kind,
            name: tag.name.clone(),
            source: Some(tag.clone()),
        };
        tx.commit().map_err(|_| CatalogError::Storage)?;
        Ok(AddCollectionResult {
            record,
            created: true,
        })
    }

    fn delete_empty(&mut self, kind: CollectionKind, ids: &[i64]) -> Result<(), CatalogError> {
        if ids.is_empty() || ids.iter().any(|id| *id <= 0) {
            return Err(CatalogError::Changed);
        }
        // Check the persisted associations in the same transaction as deletion.
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| CatalogError::Storage)?;
        let version: i64 = tx
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|_| CatalogError::Schema)?;
        if version != 4 {
            return Err(CatalogError::Schema);
        }
        for id in ids {
            let count: i64 = tx
                .query_row(
                    "SELECT count(*) FROM collection_media WHERE collection_id=?1",
                    [id],
                    |row| row.get(0),
                )
                .map_err(|_| CatalogError::Storage)?;
            if count != 0 {
                return Err(CatalogError::Changed);
            }
            if tx
                .execute(
                    "DELETE FROM collections WHERE id=?1 AND category=?2",
                    params![id, kind.category()],
                )
                .map_err(|_| CatalogError::Storage)?
                != 1
            {
                return Err(CatalogError::Changed);
            }
        }
        tx.commit().map_err(|_| CatalogError::Storage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag() -> SourceTag {
        SourceTag {
            id: 41,
            name: "synthetic_character".into(),
            category: 4,
            post_count: 80,
        }
    }

    #[test]
    fn reopen_idempotence_delete_and_atomic_scope() {
        let path = std::env::temp_dir().join(format!(
            "bdm-catalog-test-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let mut repo = SqliteCatalog::open(&path).unwrap();
            assert!(repo.list().unwrap().is_empty());
            let first = repo.add(CollectionKind::Character, &tag()).unwrap();
            assert!(first.created);
            assert!(!repo.add(CollectionKind::Character, &tag()).unwrap().created);
            assert_eq!(repo.list().unwrap().len(), 1);
            assert_eq!(
                repo.delete_empty(CollectionKind::Character, &[first.record.id, 999]),
                Err(CatalogError::Changed)
            );
            assert_eq!(repo.list().unwrap().len(), 1);
            assert_eq!(
                repo.delete_empty(CollectionKind::Artist, &[first.record.id]),
                Err(CatalogError::Changed)
            );
        }
        {
            let mut repo = SqliteCatalog::open(&path).unwrap();
            let all = repo.list().unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].source, Some(tag()));
            repo.delete_empty(CollectionKind::Character, &[all[0].id])
                .unwrap();
        }
        assert!(SqliteCatalog::open(&path)
            .unwrap()
            .list()
            .unwrap()
            .is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn incompatible_versions_and_changed_migrations_are_preserved() {
        let repo = SqliteCatalog::in_memory().unwrap();
        repo.connection
            .execute("UPDATE schema_migrations SET checksum='tampered'", [])
            .unwrap();
        assert!(matches!(
            SqliteCatalog::initialize(repo.connection),
            Err(CatalogError::Schema)
        ));
        let connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 9).unwrap();
        assert!(matches!(
            SqliteCatalog::initialize(connection),
            Err(CatalogError::Schema)
        ));
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE old_data(value TEXT); INSERT INTO old_data VALUES('preserve');",
            )
            .unwrap();
        assert!(matches!(
            SqliteCatalog::initialize(connection),
            Err(CatalogError::Schema)
        ));
    }

    #[test]
    fn concurrent_initialization_and_add_reuse_one_record() {
        let path = std::env::temp_dir().join(format!(
            "bdm-race-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let mut repo = SqliteCatalog::open(&path).unwrap();
                    repo.add(CollectionKind::Character, &tag()).unwrap()
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.created).count(), 1);
        assert_eq!(results[0].record.id, results[1].record.id);
        {
            let repo = SqliteCatalog::open(&path).unwrap();
            assert_eq!(repo.list().unwrap().len(), 1);
            assert_eq!(
                repo.connection
                    .pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
        std::fs::remove_file(path).unwrap();
    }
}
