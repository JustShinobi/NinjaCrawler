CREATE TABLE IF NOT EXISTS provider_accounts (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    display_name TEXT NOT NULL,
    auth_mode TEXT NOT NULL,
    auth_state TEXT NOT NULL,
    capabilities_json TEXT NOT NULL DEFAULT '[]',
    last_validated_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS provider_account_sessions (
    account_id TEXT PRIMARY KEY,
    auth_mode TEXT NOT NULL,
    session_format TEXT NOT NULL,
    session_hint TEXT NOT NULL DEFAULT '',
    fingerprint TEXT NOT NULL,
    secret_ref TEXT NOT NULL,
    expires_at TEXT,
    imported_at TEXT NOT NULL,
    last_validated_at TEXT,
    last_validation_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS embedded_auth_flows (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    window_label TEXT NOT NULL UNIQUE,
    start_url TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS source_profiles (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    handle TEXT NOT NULL,
    display_name TEXT NOT NULL,
    account_id TEXT,
    labels_json TEXT NOT NULL DEFAULT '[]',
    ready_for_download INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS source_sync_runs (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    tool TEXT NOT NULL,
    trigger TEXT NOT NULL,
    status TEXT NOT NULL,
    summary TEXT NOT NULL,
    command_preview TEXT NOT NULL,
    degraded_capabilities_json TEXT NOT NULL DEFAULT '[]',
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS scheduler_sets (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_plans (
    id TEXT PRIMARY KEY,
    scheduler_set_id TEXT NOT NULL,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    mode TEXT NOT NULL,
    interval_minutes INTEGER NOT NULL,
    startup_delay_minutes INTEGER NOT NULL DEFAULT 0,
    notification_mode TEXT NOT NULL,
    target_filter TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (scheduler_set_id) REFERENCES scheduler_sets(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS feed_sessions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    item_count INTEGER NOT NULL DEFAULT 0,
    last_updated_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS feed_collections (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    item_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS media_items (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    source_id TEXT NOT NULL,
    session_id TEXT,
    source_handle TEXT NOT NULL,
    media_type TEXT NOT NULL,
    captured_at TEXT NOT NULL,
    file_path TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
    FOREIGN KEY (session_id) REFERENCES feed_sessions(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS saved_filters (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    expression TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS saved_views (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    mode TEXT NOT NULL,
    thumbnail_size TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
