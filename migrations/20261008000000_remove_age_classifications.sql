-- Remove retired age-classification metadata without changing other game metadata.
UPDATE rom_metadata
SET metadata_json = json_remove(metadata_json, '$.age_ratings', '$.romm.age_ratings'),
    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE json_type(metadata_json, '$.age_ratings') IS NOT NULL
   OR json_type(metadata_json, '$.romm.age_ratings') IS NOT NULL;
