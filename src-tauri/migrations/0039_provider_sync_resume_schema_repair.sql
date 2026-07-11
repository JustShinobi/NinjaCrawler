-- Reparo idempotente para builds de desenvolvimento que registraram a versao
-- 38 antes de as tabelas operacionais de retomada existirem.
CREATE TABLE IF NOT EXISTS provider_sync_resume_cursors (
    provider TEXT NOT NULL,
    source_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    section TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'completed')),
    cursor TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (provider, source_id, account_id, scope, section),
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS provider_sync_account_holds (
    provider TEXT NOT NULL,
    account_id TEXT NOT NULL,
    hold_until TEXT NOT NULL,
    reason TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (provider, account_id),
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);
