-- Initial schema for the first public Teatro source release.
-- This baseline supports fresh installations only; private pre-release migration history is intentionally omitted.

CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('admin', 'readonly')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE library_roots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    root_path TEXT NOT NULL UNIQUE,
    writable INTEGER NOT NULL DEFAULT 1 CHECK (writable IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE platforms (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    slug TEXT NOT NULL UNIQUE,
    fs_slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    display_name TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    action TEXT NOT NULL,
    entity_type TEXT,
    entity_id INTEGER,
    metadata_json TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE roms (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    platform_id INTEGER NOT NULL REFERENCES platforms(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    summary TEXT,
    regions_json TEXT NOT NULL DEFAULT '[]',
    url_cover TEXT,
    path_cover_large TEXT,
    path_cover_small TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(platform_id, slug)
);

CREATE TABLE rom_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rom_id INTEGER NOT NULL REFERENCES roms(id) ON DELETE CASCADE,
    root_id INTEGER NOT NULL REFERENCES library_roots(id) ON DELETE RESTRICT,
    relative_path TEXT NOT NULL,
    file_name TEXT NOT NULL,
    file_size_bytes INTEGER NOT NULL DEFAULT 0,
    sha256 TEXT,
    is_primary INTEGER NOT NULL DEFAULT 1 CHECK (is_primary IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), group_id INTEGER REFERENCES rom_file_groups(id) ON DELETE SET NULL, original_file_name TEXT, role TEXT NOT NULL DEFAULT 'content' CHECK (
    role IN (
        'content',
        'launch_manifest',
        'descriptor',
        'disc_image',
        'track',
        'archive_volume',
        'manual',
        'patch',
        'dlc',
        'metadata_sidecar'
    )
), sort_index INTEGER NOT NULL DEFAULT 0, disc_index INTEGER, track_index INTEGER, launchable INTEGER NOT NULL DEFAULT 1 CHECK (launchable IN (0, 1)), metadata_json TEXT NOT NULL DEFAULT '{}', crc32 TEXT, md5 TEXT, sha1 TEXT, hash_status TEXT NOT NULL DEFAULT 'pending' CHECK (
    hash_status IN ('pending', 'hashing', 'complete', 'failed')
), hashed_at TEXT, hash_error TEXT,
    UNIQUE(root_id, relative_path)
);

CREATE TABLE rom_metadata (
    rom_id INTEGER PRIMARY KEY REFERENCES roms(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    schema_version INTEGER NOT NULL DEFAULT 1,
    fetched_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE cover_assets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rom_id INTEGER REFERENCES roms(id) ON DELETE CASCADE,
    resource_path TEXT NOT NULL UNIQUE,
    media_type TEXT,
    file_size_bytes INTEGER,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE api_tokens (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    token_prefix TEXT NOT NULL,
    scopes_json TEXT NOT NULL DEFAULT '["read"]',
    expires_at TEXT,
    revoked_at TEXT,
    last_used_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE igdb_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    client_id TEXT,
    client_secret TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE rom_file_groups (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rom_id INTEGER NOT NULL REFERENCES roms(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    group_key TEXT,
    disc_index INTEGER,
    disc_count INTEGER,
    launchable INTEGER NOT NULL DEFAULT 0 CHECK (launchable IN (0, 1)),
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE rom_file_dependencies (
    parent_file_id INTEGER NOT NULL REFERENCES rom_files(id) ON DELETE CASCADE,
    child_file_id INTEGER NOT NULL REFERENCES rom_files(id) ON DELETE CASCADE,
    dependency_kind TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (parent_file_id, child_file_id, dependency_kind)
);

CREATE TABLE dat_sources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT,
    version TEXT,
    author TEXT,
    homepage TEXT,
    imported_file_name TEXT NOT NULL,
    file_sha256 TEXT NOT NULL UNIQUE,
    entry_count INTEGER NOT NULL DEFAULT 0,
    imported_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE dat_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id INTEGER NOT NULL REFERENCES dat_sources(id) ON DELETE CASCADE,
    game_name TEXT NOT NULL,
    description TEXT,
    rom_name TEXT NOT NULL,
    file_size_bytes INTEGER,
    crc32 TEXT,
    md5 TEXT,
    sha1 TEXT,
    sha256 TEXT,
    serial TEXT,
    regions_json TEXT NOT NULL DEFAULT '[]',
    languages_json TEXT NOT NULL DEFAULT '[]',
    metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE rom_file_dat_matches (
    file_id INTEGER NOT NULL REFERENCES rom_files(id) ON DELETE CASCADE,
    dat_entry_id INTEGER NOT NULL REFERENCES dat_entries(id) ON DELETE CASCADE,
    matched_by TEXT NOT NULL,
    verified_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (file_id, dat_entry_id)
);

CREATE TABLE integrity_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL DEFAULT 'hash_and_match' CHECK (kind = 'hash_and_match'),
    status TEXT NOT NULL DEFAULT 'queued' CHECK (
        status IN ('queued', 'running', 'completed', 'failed')
    ),
    rom_id INTEGER REFERENCES roms(id) ON DELETE SET NULL,
    force INTEGER NOT NULL DEFAULT 0 CHECK (force IN (0, 1)),
    total_files INTEGER NOT NULL DEFAULT 0,
    processed_files INTEGER NOT NULL DEFAULT 0,
    hashed_files INTEGER NOT NULL DEFAULT 0,
    matched_files INTEGER NOT NULL DEFAULT 0,
    error_count INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    started_at TEXT,
    completed_at TEXT
);

CREATE TABLE file_operations (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('upload', 'delete', 'cover_replace')),
    state TEXT NOT NULL CHECK (
        state IN ('prepared', 'db_committed', 'cleanup_pending', 'completed', 'failed')
    ),
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE romm_source_settings (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    base_url   TEXT NOT NULL,
    username   TEXT,
    secret     TEXT,
    auth_mode  TEXT NOT NULL DEFAULT 'token' CHECK (auth_mode IN ('token', 'basic')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE romm_remote_index (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    refreshed_at   TEXT NOT NULL,
    game_count     INTEGER NOT NULL CHECK (game_count >= 0),
    platform_count INTEGER NOT NULL CHECK (platform_count >= 0)
);

CREATE TABLE romm_remote_platforms (
    id        INTEGER PRIMARY KEY,
    slug      TEXT NOT NULL,
    name      TEXT NOT NULL,
    rom_count INTEGER NOT NULL CHECK (rom_count >= 0)
);

CREATE TABLE romm_remote_roms (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL,
    platform_id     INTEGER,
    platform_slug   TEXT,
    platform_name   TEXT,
    fs_name         TEXT,
    file_size_bytes INTEGER CHECK (file_size_bytes IS NULL OR file_size_bytes >= 0),
    has_cover       INTEGER NOT NULL CHECK (has_cover IN (0, 1)),
    file_count      INTEGER NOT NULL CHECK (file_count >= 0)
);

CREATE TABLE browser_sessions (
    token_hash TEXT PRIMARY KEY NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    password_hash TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);

INSERT INTO platforms (id, slug, fs_slug, name, display_name) VALUES
    (1, 'nes', 'nes', 'Nintendo Entertainment System', 'Nintendo Entertainment System'),
    (2, 'snes', 'snes', 'Nintendo SNES (Super Nintendo)', 'Nintendo SNES (Super Nintendo)'),
    (3, 'n64', 'n64', 'Nintendo 64', 'Nintendo 64'),
    (4, 'gba', 'gba', 'Nintendo Game Boy Advance', 'Nintendo Game Boy Advance'),
    (5, 'genesis', 'genesis', 'Sega Genesis', 'Sega Genesis'),
    (6, 'ps2', 'ps2', 'Sony PlayStation 2', 'Sony PlayStation 2'),
    (7, 'psp', 'psp', 'Sony PlayStation Portable', 'Sony PlayStation Portable'),
    (10, '3do', '3do', '3DO', '3DO'),
    (15, 'amigacd32', 'amigacd32', 'Commodore Amiga CD32', 'Commodore Amiga CD32'),
    (26, 'atarijaguar', 'atarijaguar', 'Atari Jaguar', 'Atari Jaguar'),
    (27, 'atarijaguarcd', 'atarijaguarcd', 'Atari Jaguar CD', 'Atari Jaguar CD'),
    (28, 'atarilynx', 'atarilynx', 'Atari Lynx', 'Atari Lynx'),
    (35, 'cdimono1', 'cdimono1', 'Philips CD-i', 'Philips CD-i'),
    (40, 'colecovision', 'colecovision', 'ColecoVision', 'ColecoVision'),
    (47, 'dreamcast', 'dreamcast', 'Sega Dreamcast', 'Sega Dreamcast'),
    (50, 'famicom', 'famicom', 'Nintendo Family Computer', 'Nintendo Family Computer'),
    (53, 'fds', 'fds', 'Nintendo Famicom Disk System', 'Nintendo Famicom Disk System'),
    (57, 'gamegear', 'gamegear', 'Sega Game Gear', 'Sega Game Gear'),
    (58, 'gb', 'gb', 'Nintendo Game Boy', 'Nintendo Game Boy'),
    (60, 'gbc', 'gbc', 'Nintendo Game Boy Color', 'Nintendo Game Boy Color'),
    (61, 'gc', 'gc', 'Nintendo GameCube', 'Nintendo GameCube'),
    (64, 'intellivision', 'intellivision', 'Mattel Electronics Intellivision', 'Mattel Electronics Intellivision'),
    (73, 'mastersystem', 'mastersystem', 'Sega Master System', 'Sega Master System'),
    (74, 'megacd', 'megacd', 'Sega Mega-CD', 'Sega Mega-CD'),
    (75, 'megacdjp', 'megacdjp', 'Sega Mega-CD', 'Sega Mega-CD'),
    (76, 'megadrive', 'megadrive', 'Sega Mega Drive', 'Sega Mega Drive'),
    (89, 'n3ds', 'n3ds', 'Nintendo 3DS', 'Nintendo 3DS'),
    (91, 'n64dd', 'n64dd', 'Nintendo 64DD', 'Nintendo 64DD'),
    (94, 'nds', 'nds', 'Nintendo DS', 'Nintendo DS'),
    (95, 'neogeo', 'neogeo', 'SNK Neo Geo', 'SNK Neo Geo'),
    (96, 'neogeocd', 'neogeocd', 'SNK Neo Geo CD', 'SNK Neo Geo CD'),
    (99, 'ngp', 'ngp', 'SNK Neo Geo Pocket', 'SNK Neo Geo Pocket'),
    (100, 'ngpc', 'ngpc', 'SNK Neo Geo Pocket Color', 'SNK Neo Geo Pocket Color'),
    (108, 'pcengine', 'pcengine', 'NEC PC Engine', 'NEC PC Engine'),
    (109, 'pcenginecd', 'pcenginecd', 'NEC PC Engine CD', 'NEC PC Engine CD'),
    (110, 'pcfx', 'pcfx', 'NEC PC-FX', 'NEC PC-FX'),
    (115, 'ps3', 'ps3', 'Sony PlayStation 3', 'Sony PlayStation 3'),
    (116, 'ps4', 'ps4', 'Sony PlayStation 4', 'Sony PlayStation 4'),
    (118, 'psvita', 'psvita', 'Sony PlayStation Vita', 'Sony PlayStation Vita'),
    (119, 'psx', 'psx', 'Sony PlayStation', 'Sony PlayStation'),
    (121, 'satellaview', 'satellaview', 'Nintendo Satellaview', 'Nintendo Satellaview'),
    (122, 'saturn', 'saturn', 'Sega Saturn', 'Sega Saturn'),
    (123, 'saturnjp', 'saturnjp', 'Sega Saturn', 'Sega Saturn'),
    (125, 'sega32x', 'sega32x', 'Sega Mega Drive 32X', 'Sega Mega Drive 32X'),
    (126, 'sega32xjp', 'sega32xjp', 'Sega Super 32X', 'Sega Super 32X'),
    (127, 'sega32xna', 'sega32xna', 'Sega Genesis 32X', 'Sega Genesis 32X'),
    (128, 'segacd', 'segacd', 'Sega CD', 'Sega CD'),
    (129, 'sfc', 'sfc', 'Nintendo SFC (Super Famicom)', 'Nintendo SFC (Super Famicom)'),
    (131, 'sgb', 'sgb', 'Nintendo Super Game Boy', 'Nintendo Super Game Boy'),
    (133, 'snesna', 'snesna', 'Nintendo SNES (Super Nintendo)', 'Nintendo SNES (Super Nintendo)'),
    (139, 'supergrafx', 'supergrafx', 'NEC SuperGrafx', 'NEC SuperGrafx'),
    (141, 'switch', 'switch', 'Nintendo Switch', 'Nintendo Switch'),
    (144, 'tg-cd', 'tg-cd', 'NEC TurboGrafx-CD', 'NEC TurboGrafx-CD'),
    (145, 'tg16', 'tg16', 'NEC TurboGrafx-16', 'NEC TurboGrafx-16'),
    (154, 'virtualboy', 'virtualboy', 'Nintendo Virtual Boy', 'Nintendo Virtual Boy'),
    (155, 'wii', 'wii', 'Nintendo Wii', 'Nintendo Wii'),
    (156, 'wiiu', 'wiiu', 'Nintendo Wii U', 'Nintendo Wii U'),
    (157, 'wonderswan', 'wonderswan', 'Bandai WonderSwan', 'Bandai WonderSwan'),
    (158, 'wonderswancolor', 'wonderswancolor', 'Bandai WonderSwan Color', 'Bandai WonderSwan Color'),
    (161, 'xbox', 'xbox', 'Microsoft Xbox', 'Microsoft Xbox'),
    (162, 'xbox360', 'xbox360', 'Microsoft Xbox 360', 'Microsoft Xbox 360'),
    (166, 'win', 'win', 'Windows', 'Windows');

CREATE INDEX idx_users_username ON users(username COLLATE NOCASE);

CREATE INDEX idx_platforms_display_name ON platforms(display_name);

CREATE INDEX idx_audit_log_created_at ON audit_log(created_at);

CREATE INDEX idx_audit_log_actor_user_id ON audit_log(actor_user_id);

CREATE INDEX idx_audit_log_action ON audit_log(action);

CREATE INDEX idx_roms_platform_id ON roms(platform_id);

CREATE INDEX idx_roms_name ON roms(name COLLATE NOCASE);

CREATE INDEX idx_rom_files_rom_id ON rom_files(rom_id);

CREATE INDEX idx_rom_files_root_id ON rom_files(root_id);

CREATE INDEX idx_cover_assets_rom_id ON cover_assets(rom_id);

CREATE INDEX idx_api_tokens_user_id ON api_tokens(user_id);

CREATE INDEX idx_api_tokens_token_hash ON api_tokens(token_hash);

CREATE INDEX idx_api_tokens_token_prefix ON api_tokens(token_prefix);

CREATE INDEX idx_api_tokens_revoked_at ON api_tokens(revoked_at);

CREATE INDEX idx_rom_file_groups_rom_id ON rom_file_groups(rom_id);

CREATE INDEX idx_rom_file_groups_group_key ON rom_file_groups(group_key);

CREATE INDEX idx_rom_files_group_id ON rom_files(group_id);

CREATE INDEX idx_rom_files_role ON rom_files(role);

CREATE INDEX idx_rom_files_launchable ON rom_files(launchable);

CREATE INDEX idx_rom_file_dependencies_child_file_id ON rom_file_dependencies(child_file_id);

CREATE INDEX idx_rom_files_crc32 ON rom_files(crc32) WHERE crc32 IS NOT NULL;

CREATE INDEX idx_rom_files_md5 ON rom_files(md5) WHERE md5 IS NOT NULL;

CREATE INDEX idx_rom_files_sha1 ON rom_files(sha1) WHERE sha1 IS NOT NULL;

CREATE INDEX idx_rom_files_sha256 ON rom_files(sha256) WHERE sha256 IS NOT NULL;

CREATE INDEX idx_rom_files_hash_status ON rom_files(hash_status);

CREATE INDEX idx_dat_entries_source_id ON dat_entries(source_id);

CREATE INDEX idx_dat_entries_crc32 ON dat_entries(crc32) WHERE crc32 IS NOT NULL;

CREATE INDEX idx_dat_entries_md5 ON dat_entries(md5) WHERE md5 IS NOT NULL;

CREATE INDEX idx_dat_entries_sha1 ON dat_entries(sha1) WHERE sha1 IS NOT NULL;

CREATE INDEX idx_dat_entries_sha256 ON dat_entries(sha256) WHERE sha256 IS NOT NULL;

CREATE INDEX idx_rom_file_dat_matches_entry_id ON rom_file_dat_matches(dat_entry_id);

CREATE INDEX idx_integrity_jobs_status ON integrity_jobs(status);

CREATE INDEX idx_integrity_jobs_rom_id ON integrity_jobs(rom_id);

CREATE INDEX idx_rom_files_rom_order
ON rom_files (rom_id, launchable, is_primary, sort_index, disc_index, track_index, id);

CREATE INDEX idx_file_operations_reconciliation
ON file_operations (state, created_at)
WHERE state != 'completed';

CREATE INDEX idx_file_operations_history
ON file_operations (created_at);

CREATE UNIQUE INDEX idx_integrity_jobs_single_active
ON integrity_jobs ((1))
WHERE status IN ('queued', 'running');

CREATE INDEX idx_romm_remote_roms_platform_name
    ON romm_remote_roms(platform_id, name COLLATE NOCASE, id);

CREATE INDEX idx_romm_remote_roms_name
    ON romm_remote_roms(name COLLATE NOCASE, id);

CREATE INDEX browser_sessions_user_id ON browser_sessions(user_id);

CREATE INDEX browser_sessions_expires_at ON browser_sessions(expires_at);

CREATE TRIGGER validate_roms_json_insert
BEFORE INSERT ON roms
WHEN json_valid(NEW.regions_json) = 0
BEGIN
    SELECT RAISE(ABORT, 'roms.regions_json must contain valid JSON');
END;

CREATE TRIGGER validate_roms_json_update
BEFORE UPDATE OF regions_json ON roms
WHEN json_valid(NEW.regions_json) = 0
BEGIN
    SELECT RAISE(ABORT, 'roms.regions_json must contain valid JSON');
END;

CREATE TRIGGER validate_rom_metadata_json_insert
BEFORE INSERT ON rom_metadata
WHEN json_valid(NEW.metadata_json) = 0
BEGIN
    SELECT RAISE(ABORT, 'rom_metadata.metadata_json must contain valid JSON');
END;

CREATE TRIGGER validate_rom_metadata_json_update
BEFORE UPDATE OF metadata_json ON rom_metadata
WHEN json_valid(NEW.metadata_json) = 0
BEGIN
    SELECT RAISE(ABORT, 'rom_metadata.metadata_json must contain valid JSON');
END;

CREATE TRIGGER validate_api_token_scopes_insert
BEFORE INSERT ON api_tokens
WHEN json_valid(NEW.scopes_json) = 0 OR json_type(NEW.scopes_json) != 'array'
BEGIN
    SELECT RAISE(ABORT, 'api_tokens.scopes_json must contain a JSON array');
END;

CREATE TRIGGER validate_api_token_scopes_update
BEFORE UPDATE OF scopes_json ON api_tokens
WHEN json_valid(NEW.scopes_json) = 0 OR json_type(NEW.scopes_json) != 'array'
BEGIN
    SELECT RAISE(ABORT, 'api_tokens.scopes_json must contain a JSON array');
END;

CREATE TRIGGER validate_rom_file_group_insert
BEFORE INSERT ON rom_file_groups
WHEN NEW.kind NOT IN ('single', 'playlist', 'disc', 'track_set')
    OR NEW.disc_index < 0
    OR NEW.disc_count < 0
    OR json_valid(NEW.metadata_json) = 0
BEGIN
    SELECT RAISE(ABORT, 'invalid ROM file group');
END;

CREATE TRIGGER validate_rom_file_group_update
BEFORE UPDATE ON rom_file_groups
WHEN NEW.kind NOT IN ('single', 'playlist', 'disc', 'track_set')
    OR NEW.disc_index < 0
    OR NEW.disc_count < 0
    OR json_valid(NEW.metadata_json) = 0
BEGIN
    SELECT RAISE(ABORT, 'invalid ROM file group');
END;

CREATE TRIGGER validate_rom_file_insert
BEFORE INSERT ON rom_files
WHEN NEW.file_size_bytes < 0
    OR NEW.sort_index < 0
    OR NEW.disc_index < 0
    OR NEW.track_index < 0
    OR json_valid(NEW.metadata_json) = 0
    OR (
        NEW.group_id IS NOT NULL
        AND NOT EXISTS (
            SELECT 1 FROM rom_file_groups g
            WHERE g.id = NEW.group_id AND g.rom_id = NEW.rom_id
        )
    )
BEGIN
    SELECT RAISE(ABORT, 'invalid ROM file');
END;

CREATE TRIGGER validate_rom_file_update
BEFORE UPDATE ON rom_files
WHEN NEW.file_size_bytes < 0
    OR NEW.sort_index < 0
    OR NEW.disc_index < 0
    OR NEW.track_index < 0
    OR json_valid(NEW.metadata_json) = 0
    OR (
        NEW.group_id IS NOT NULL
        AND NOT EXISTS (
            SELECT 1 FROM rom_file_groups g
            WHERE g.id = NEW.group_id AND g.rom_id = NEW.rom_id
        )
    )
BEGIN
    SELECT RAISE(ABORT, 'invalid ROM file');
END;

CREATE TRIGGER validate_rom_file_dependency_insert
BEFORE INSERT ON rom_file_dependencies
WHEN NEW.dependency_kind NOT IN ('playlist_entry', 'cue_file', 'gdi_track')
    OR NEW.sort_index < 0
    OR NOT EXISTS (
        SELECT 1
        FROM rom_files parent
        JOIN rom_files child ON child.id = NEW.child_file_id
        WHERE parent.id = NEW.parent_file_id AND parent.rom_id = child.rom_id
    )
BEGIN
    SELECT RAISE(ABORT, 'invalid ROM file dependency');
END;

CREATE TRIGGER validate_rom_file_dependency_update
BEFORE UPDATE ON rom_file_dependencies
WHEN NEW.dependency_kind NOT IN ('playlist_entry', 'cue_file', 'gdi_track')
    OR NEW.sort_index < 0
    OR NOT EXISTS (
        SELECT 1
        FROM rom_files parent
        JOIN rom_files child ON child.id = NEW.child_file_id
        WHERE parent.id = NEW.parent_file_id AND parent.rom_id = child.rom_id
    )
BEGIN
    SELECT RAISE(ABORT, 'invalid ROM file dependency');
END;

CREATE TRIGGER prevent_rom_file_owner_update
BEFORE UPDATE OF rom_id ON rom_files
WHEN NEW.rom_id != OLD.rom_id
BEGIN
    SELECT RAISE(ABORT, 'rom_files.rom_id is immutable');
END;

CREATE TRIGGER prevent_rom_file_group_owner_update
BEFORE UPDATE OF rom_id ON rom_file_groups
WHEN NEW.rom_id != OLD.rom_id
BEGIN
    SELECT RAISE(ABORT, 'rom_file_groups.rom_id is immutable');
END;

CREATE TRIGGER validate_dat_entry_json_insert
BEFORE INSERT ON dat_entries
WHEN json_valid(NEW.regions_json) = 0
    OR json_type(NEW.regions_json) != 'array'
    OR json_valid(NEW.languages_json) = 0
    OR json_type(NEW.languages_json) != 'array'
BEGIN
    SELECT RAISE(ABORT, 'dat_entries region/language metadata must contain JSON arrays');
END;

CREATE TRIGGER validate_dat_entry_json_update
BEFORE UPDATE OF regions_json, languages_json ON dat_entries
WHEN json_valid(NEW.regions_json) = 0
    OR json_type(NEW.regions_json) != 'array'
    OR json_valid(NEW.languages_json) = 0
    OR json_type(NEW.languages_json) != 'array'
BEGIN
    SELECT RAISE(ABORT, 'dat_entries region/language metadata must contain JSON arrays');
END;

CREATE TRIGGER validate_rom_file_dat_match_method_insert
BEFORE INSERT ON rom_file_dat_matches
WHEN NEW.matched_by NOT IN ('crc32', 'md5', 'sha1', 'sha256')
BEGIN
    SELECT RAISE(ABORT, 'invalid rom_file_dat_matches.matched_by');
END;

CREATE TRIGGER validate_rom_file_dat_match_method_update
BEFORE UPDATE OF matched_by ON rom_file_dat_matches
WHEN NEW.matched_by NOT IN ('crc32', 'md5', 'sha1', 'sha256')
BEGIN
    SELECT RAISE(ABORT, 'invalid rom_file_dat_matches.matched_by');
END;
