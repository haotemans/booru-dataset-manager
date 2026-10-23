CREATE TABLE collections_v4 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider TEXT NOT NULL CHECK(provider = 'danbooru'),
    source_tag_id INTEGER CHECK(source_tag_id > 0),
    category INTEGER NOT NULL CHECK(category IN (1, 4)),
    tag TEXT NOT NULL CHECK(length(tag) > 0),
    remote_post_count INTEGER CHECK(remote_post_count >= 0),
    fetched_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK((source_tag_id IS NULL) = (remote_post_count IS NULL)),
    UNIQUE(provider, source_tag_id),
    UNIQUE(provider, tag)
);
CREATE TEMP TABLE collection_sequence AS SELECT seq FROM sqlite_sequence WHERE name='collections';
INSERT INTO collections_v4 SELECT * FROM collections;
DROP TABLE collections;
ALTER TABLE collections_v4 RENAME TO collections;
UPDATE sqlite_sequence SET seq=max(seq,coalesce((SELECT seq FROM collection_sequence),0)) WHERE name='collections';
INSERT INTO sqlite_sequence(name,seq) SELECT 'collections',seq FROM collection_sequence WHERE NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='collections');
DROP TABLE collection_sequence;
CREATE INDEX collection_media_by_media ON collection_media(media_id, collection_id);
CREATE INDEX media_sources_by_media ON media_sources(media_id);
PRAGMA user_version = 4;
