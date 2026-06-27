ALTER TABLE source_profiles ADD COLUMN importer_id TEXT;
ALTER TABLE source_profiles ADD COLUMN imported_at TEXT;

UPDATE source_profiles
SET importer_id = (
        SELECT ledger.importer_id
        FROM external_import_ledger ledger
        WHERE ledger.source_id = source_profiles.id
        ORDER BY ledger.imported_at DESC
        LIMIT 1
    ),
    imported_at = (
        SELECT ledger.imported_at
        FROM external_import_ledger ledger
        WHERE ledger.source_id = source_profiles.id
        ORDER BY ledger.imported_at DESC
        LIMIT 1
    )
WHERE EXISTS (
    SELECT 1
    FROM external_import_ledger ledger
    WHERE ledger.source_id = source_profiles.id
);
