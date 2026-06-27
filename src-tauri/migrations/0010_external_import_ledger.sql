CREATE TABLE IF NOT EXISTS external_import_ledger (
  importer_id TEXT NOT NULL,
  entity_key TEXT NOT NULL,
  provider TEXT NOT NULL,
  handle TEXT NOT NULL,
  source_id TEXT,
  account_id TEXT,
  imported_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (importer_id, entity_key)
);

CREATE INDEX IF NOT EXISTS idx_external_import_ledger_provider
  ON external_import_ledger (provider, handle);
