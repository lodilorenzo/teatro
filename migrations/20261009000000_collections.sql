CREATE TABLE collections (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 100),
    slug TEXT NOT NULL UNIQUE COLLATE NOCASE CHECK (length(slug) BETWEEN 1 AND 80),
    lan_party_zone INTEGER NOT NULL DEFAULT 0 CHECK (lan_party_zone IN (0, 1))
);

CREATE TABLE collection_roms (
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    rom_id INTEGER NOT NULL REFERENCES roms(id) ON DELETE CASCADE,
    PRIMARY KEY (collection_id, rom_id)
);
CREATE INDEX collection_roms_rom_id ON collection_roms(rom_id);
