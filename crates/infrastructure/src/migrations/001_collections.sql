CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    checksum TEXT NOT NULL,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE TABLE collections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider TEXT NOT NULL CHECK(provider = 'danbooru'),
    source_tag_id INTEGER NOT NULL CHECK(source_tag_id > 0),
    category INTEGER NOT NULL CHECK(category IN (1, 4)),
    tag TEXT NOT NULL CHECK(length(tag) > 0),
    remote_post_count INTEGER NOT NULL CHECK(remote_post_count >= 0),
    fetched_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(provider, source_tag_id),
    UNIQUE(provider, tag)
);
PRAGMA user_version = 1;
