PRAGMA foreign_keys = OFF;

ALTER TABLE connector_runtimes RENAME TO connector_runtimes_legacy;

CREATE TABLE connector_runtimes (
    key TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    management_mode TEXT NOT NULL,
    bundled_version TEXT NOT NULL,
    active_version TEXT,
    active_path TEXT,
    custom_path TEXT,
    latest_version TEXT,
    latest_asset_url TEXT,
    latest_checked_at TEXT,
    update_status TEXT NOT NULL,
    pending_version TEXT,
    pending_path TEXT,
    progress_percent INTEGER,
    progress_detail TEXT,
    last_error TEXT,
    updated_at TEXT NOT NULL,
    CHECK (management_mode IN ('managed', 'custom'))
);

INSERT INTO connector_runtimes (
    key,
    display_name,
    management_mode,
    bundled_version,
    active_version,
    active_path,
    custom_path,
    latest_version,
    latest_asset_url,
    latest_checked_at,
    update_status,
    pending_version,
    pending_path,
    progress_percent,
    progress_detail,
    last_error,
    updated_at
)
SELECT
    key,
    display_name,
    management_mode,
    bundled_version,
    active_version,
    active_path,
    custom_path,
    latest_version,
    latest_asset_url,
    latest_checked_at,
    update_status,
    pending_version,
    pending_path,
    progress_percent,
    progress_detail,
    last_error,
    updated_at
FROM connector_runtimes_legacy;

DROP TABLE connector_runtimes_legacy;

PRAGMA foreign_keys = ON;
