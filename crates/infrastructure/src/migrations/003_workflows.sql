CREATE TABLE tag_rules (id INTEGER PRIMARY KEY CHECK(id=1), metadata TEXT NOT NULL);
INSERT INTO tag_rules VALUES(1, '{}');
CREATE TABLE text_artifacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('tags','caption')),
    content TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    provenance TEXT NOT NULL,
    state TEXT NOT NULL,
    reviewed INTEGER NOT NULL DEFAULT 0,
    is_current INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE UNIQUE INDEX current_artifact ON text_artifacts(media_id,kind) WHERE is_current=1;
CREATE TABLE visual_features (
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    version TEXT NOT NULL,
    phash TEXT NOT NULL,
    embedding TEXT,
    PRIMARY KEY(media_id,version)
);
CREATE TABLE duplicate_candidates (
    first_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    second_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    metadata TEXT NOT NULL,
    PRIMARY KEY(first_id,second_id)
);
CREATE TABLE visual_buckets (
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    version TEXT NOT NULL,
    bucket TEXT NOT NULL,
    PRIMARY KEY(media_id,version,bucket)
);
CREATE INDEX visual_bucket_lookup ON visual_buckets(version,bucket,media_id);
CREATE TABLE workflow_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    metadata TEXT NOT NULL
);
CREATE TABLE api_rate_state (scope TEXT PRIMARY KEY, metadata TEXT NOT NULL);
CREATE TABLE media_quality (
    media_id INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
    aesthetic REAL,
    watermark_free INTEGER,
    provenance TEXT NOT NULL
);
CREATE TABLE deletion_journal (media_id INTEGER PRIMARY KEY, metadata TEXT NOT NULL);
CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    action TEXT NOT NULL,
    metadata TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
PRAGMA user_version = 3;
