-- 配布用の自然近傍制御点DB。作業用の点索引・補助情報・ログは含めない。
PRAGMA page_size = 4096;
PRAGMA encoding = 'UTF-8';
PRAGMA application_id = 0x594E4E31;
PRAGMA user_version = 1;
PRAGMA foreign_keys = ON;
CREATE TABLE metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE generations (
    generation_key INTEGER PRIMARY KEY CHECK (generation_key > 0),
    generation_id TEXT NOT NULL UNIQUE,
    config_sha256 BLOB NOT NULL CHECK (length(config_sha256) = 32),
    source_sha256 BLOB NOT NULL CHECK (length(source_sha256) = 32),
    created_utc TEXT NOT NULL
);
CREATE TABLE levels (
    z INTEGER PRIMARY KEY CHECK (z BETWEEN 8 AND 14),
    error_m REAL NOT NULL CHECK (error_m > 0),
    parent_z INTEGER REFERENCES levels(z),
    CHECK (parent_z IS NULL OR parent_z = z - 1)
);
CREATE TABLE tiles (
    z INTEGER NOT NULL REFERENCES levels(z),
    x INTEGER NOT NULL CHECK (x >= 0 AND x < (1 << z)),
    y INTEGER NOT NULL CHECK (y >= 0 AND y < (1 << z)),
    generation_key INTEGER NOT NULL REFERENCES generations(generation_key),
    point_count INTEGER NOT NULL CHECK (point_count BETWEEN 1 AND 262144),
    owned_point_count INTEGER NOT NULL CHECK (owned_point_count BETWEEN 0 AND point_count),
    min_qz INTEGER NOT NULL,
    max_qz INTEGER NOT NULL CHECK (max_qz >= min_qz),
    dependency_count INTEGER NOT NULL CHECK (dependency_count BETWEEN 1 AND 16),
    xyz_sha256 BLOB NOT NULL CHECK (length(xyz_sha256) = 32),
    payload BLOB NOT NULL CHECK (length(payload) BETWEEN 24 AND 16777216),
    PRIMARY KEY (z,x,y)
) WITHOUT ROWID;
CREATE TABLE tile_parents (
    z INTEGER NOT NULL,
    x INTEGER NOT NULL,
    y INTEGER NOT NULL,
    slot INTEGER NOT NULL CHECK (slot BETWEEN 0 AND 3),
    parent_z INTEGER NOT NULL,
    parent_x INTEGER NOT NULL,
    parent_y INTEGER NOT NULL,
    PRIMARY KEY (z,x,y,slot),
    UNIQUE (z,x,y,parent_z,parent_x,parent_y),
    FOREIGN KEY (z,x,y) REFERENCES tiles(z,x,y) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (parent_z,parent_x,parent_y) REFERENCES tiles(z,x,y) DEFERRABLE INITIALLY DEFERRED,
    CHECK (parent_z = z - 1)
) WITHOUT ROWID;
CREATE TABLE coverage (
    root_x INTEGER NOT NULL CHECK (root_x BETWEEN 0 AND 255),
    root_y INTEGER NOT NULL CHECK (root_y BETWEEN 0 AND 255),
    generation_key INTEGER NOT NULL REFERENCES generations(generation_key),
    coverage_sha256 BLOB NOT NULL CHECK (length(coverage_sha256) = 32),
    payload BLOB NOT NULL,
    PRIMARY KEY (root_x,root_y)
) WITHOUT ROWID;
