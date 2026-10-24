-- Worker database only; measurements are portable JSON objects, never outlines.
PRAGMA user_version = 1;
CREATE TABLE IF NOT EXISTS sources (
 path TEXT PRIMARY KEY, sha256 TEXT, size INTEGER, mtime_ns INTEGER,
 status TEXT NOT NULL, error TEXT, object_path TEXT, scan_key TEXT
);
CREATE INDEX IF NOT EXISTS sources_sha256 ON sources(sha256);
CREATE TABLE IF NOT EXISTS objects (
 object_key TEXT PRIMARY KEY, sha256 TEXT NOT NULL, path TEXT NOT NULL,
 status TEXT NOT NULL, error TEXT, faces INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS runs (
 id INTEGER PRIMARY KEY, started TEXT NOT NULL, finished TEXT, status TEXT NOT NULL,
 inventory_count INTEGER NOT NULL, completed INTEGER NOT NULL DEFAULT 0
);
