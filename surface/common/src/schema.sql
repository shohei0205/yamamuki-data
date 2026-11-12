PRAGMA application_id = 1498235970;
PRAGMA user_version = 1;

CREATE TABLE datasets (
    dataset_key INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE CHECK(length(id) > 0),
    kind TEXT NOT NULL CHECK(kind IN ('surface', 'terrain', 'points')),
    title TEXT NOT NULL CHECK(length(title) > 0),
    data_version TEXT NOT NULL CHECK(length(data_version) > 0),
    format_version INTEGER NOT NULL CHECK(format_version > 0),
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json) AND json_type(metadata_json) = 'object')
) STRICT;

CREATE TABLE surface_layers (
    dataset_key INTEGER NOT NULL REFERENCES datasets,
    layer_key INTEGER NOT NULL,
    id TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('rgb', 'mask', 'hillshade', 'coverage')),
    draw_order INTEGER NOT NULL,
    source_names_json TEXT NOT NULL CHECK(json_valid(source_names_json) AND json_type(source_names_json) = 'array'),
    settings_json TEXT NOT NULL CHECK(json_valid(settings_json) AND json_type(settings_json) = 'object'),
    PRIMARY KEY(dataset_key, layer_key),
    UNIQUE(dataset_key, id)
) WITHOUT ROWID, STRICT;

CREATE TABLE surface_levels (
    dataset_key INTEGER NOT NULL,
    layer_key INTEGER NOT NULL,
    z INTEGER NOT NULL CHECK(z BETWEEN 0 AND 30),
    lod TEXT NOT NULL,
    tile_pixels INTEGER NOT NULL CHECK(tile_pixels BETWEEN 1 AND 4096),
    PRIMARY KEY(dataset_key, layer_key, z),
    UNIQUE(dataset_key, layer_key, lod),
    FOREIGN KEY(dataset_key, layer_key) REFERENCES surface_layers
) WITHOUT ROWID, STRICT;

CREATE TABLE surface_tiles (
    dataset_key INTEGER NOT NULL,
    layer_key INTEGER NOT NULL,
    z INTEGER NOT NULL,
    x INTEGER NOT NULL CHECK(x >= 0 AND x < (1 << z)),
    y INTEGER NOT NULL CHECK(y >= 0 AND y < (1 << z)),
    encoding INTEGER NOT NULL CHECK(encoding IN (1, 2, 3)),
    payload BLOB NOT NULL CHECK(length(payload) > 0),
    PRIMARY KEY(dataset_key, layer_key, z, x, y),
    FOREIGN KEY(dataset_key, layer_key, z) REFERENCES surface_levels
) WITHOUT ROWID, STRICT;

CREATE TABLE terrain_levels (
    dataset_key INTEGER NOT NULL REFERENCES datasets,
    level_key INTEGER NOT NULL,
    z INTEGER NOT NULL CHECK(z BETWEEN 0 AND 30),
    settings_json TEXT NOT NULL CHECK(json_valid(settings_json) AND json_type(settings_json) = 'object'),
    PRIMARY KEY(dataset_key, level_key)
) WITHOUT ROWID, STRICT;

CREATE TABLE terrain_tiles (
    dataset_key INTEGER NOT NULL,
    level_key INTEGER NOT NULL,
    x INTEGER NOT NULL CHECK(x >= 0),
    y INTEGER NOT NULL CHECK(y >= 0),
    encoding TEXT NOT NULL CHECK(length(encoding) > 0),
    payload BLOB NOT NULL CHECK(length(payload) > 0),
    PRIMARY KEY(dataset_key, level_key, x, y),
    FOREIGN KEY(dataset_key, level_key) REFERENCES terrain_levels
) WITHOUT ROWID, STRICT;

CREATE TABLE point_records (
    dataset_key INTEGER NOT NULL REFERENCES datasets,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    point_id TEXT NOT NULL CHECK(length(point_id) > 0),
    payload_json TEXT NOT NULL CHECK(
        json_valid(payload_json) AND json_type(payload_json) = 'object'
        AND json_type(payload_json, '$.id') IS 'text'
        AND json_extract(payload_json, '$.id') = point_id
    ),
    PRIMARY KEY(dataset_key, ordinal),
    UNIQUE(dataset_key, point_id)
) WITHOUT ROWID, STRICT;
