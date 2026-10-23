CREATE TABLE media (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    md5 TEXT NOT NULL UNIQUE,
    metadata TEXT NOT NULL,
    keep INTEGER NOT NULL DEFAULT 0 CHECK(keep IN (0,1))
);
CREATE TABLE media_sources (
    post_id INTEGER PRIMARY KEY CHECK(post_id > 0),
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    metadata TEXT NOT NULL
);
CREATE TABLE collection_media (
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    PRIMARY KEY(collection_id, media_id)
);
CREATE TABLE ingest_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    metadata TEXT NOT NULL
);
CREATE TABLE ingest_outcomes (
    job_id INTEGER NOT NULL REFERENCES ingest_jobs(id) ON DELETE CASCADE,
    post_id INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    PRIMARY KEY(job_id, post_id)
);
CREATE TABLE import_journal (
    md5 TEXT PRIMARY KEY,
    metadata TEXT NOT NULL
);
CREATE TABLE analysis_jobs (
    media_id INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
    state TEXT NOT NULL DEFAULT 'pending',
    reason TEXT NOT NULL DEFAULT 'DINOv3 runtime not connected'
);
PRAGMA user_version = 2;
