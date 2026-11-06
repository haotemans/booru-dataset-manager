CREATE TABLE collections_v6 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider TEXT NOT NULL DEFAULT 'danbooru' CHECK(provider IN ('danbooru','gelbooru')),
    source_tag_id INTEGER CHECK(source_tag_id > 0),
    category INTEGER NOT NULL CHECK(category IN (0,1,4)),
    tag TEXT NOT NULL CHECK(length(tag)>0),
    remote_post_count INTEGER CHECK(remote_post_count>=0),
    fetched_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    CHECK((source_tag_id IS NULL)=(remote_post_count IS NULL)),
    CHECK(category!=0 OR source_tag_id IS NULL),
    UNIQUE(provider,source_tag_id),
    UNIQUE(provider,category,tag)
);
CREATE TEMP TABLE collection_sequence AS SELECT seq FROM sqlite_sequence WHERE name='collections';
INSERT INTO collections_v6 SELECT * FROM collections;
DROP TABLE collections;
ALTER TABLE collections_v6 RENAME TO collections;
UPDATE sqlite_sequence SET seq=max(seq,coalesce((SELECT seq FROM collection_sequence),0)) WHERE name='collections';
INSERT INTO sqlite_sequence(name,seq) SELECT 'collections',seq FROM collection_sequence WHERE NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='collections');
DROP TABLE collection_sequence;
CREATE TABLE media_sources_v6 (
    post_id INTEGER NOT NULL CHECK(post_id>0),
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    metadata TEXT NOT NULL,
    provider TEXT NOT NULL DEFAULT 'danbooru' CHECK(provider IN ('danbooru','gelbooru')),
    PRIMARY KEY(provider,post_id)
);
INSERT INTO media_sources_v6(post_id,media_id,metadata) SELECT post_id,media_id,metadata FROM media_sources;
DROP TABLE media_sources;
ALTER TABLE media_sources_v6 RENAME TO media_sources;
CREATE INDEX media_sources_by_media ON media_sources(media_id);
CREATE TABLE ingest_outcomes_v6 (
    job_id INTEGER NOT NULL REFERENCES ingest_jobs(id) ON DELETE CASCADE,
    post_id INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    provider TEXT NOT NULL DEFAULT 'danbooru' CHECK(provider IN ('danbooru','gelbooru')),
    PRIMARY KEY(job_id,provider,post_id)
);
INSERT INTO ingest_outcomes_v6(job_id,post_id,outcome) SELECT job_id,post_id,outcome FROM ingest_outcomes;
DROP TABLE ingest_outcomes;
ALTER TABLE ingest_outcomes_v6 RENAME TO ingest_outcomes;
PRAGMA user_version=6;
