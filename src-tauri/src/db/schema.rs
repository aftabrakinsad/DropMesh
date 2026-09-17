/// Initial database schema — creates all tables for v1
pub const MIGRATION_001: &str = r#"
    -- Device identity (this device)
    CREATE TABLE IF NOT EXISTS device_self (
        id              TEXT PRIMARY KEY,
        name            TEXT NOT NULL,
        type            TEXT NOT NULL CHECK(type IN ('laptop','phone','tablet','desktop')),
        public_key      BLOB NOT NULL,
        private_key_ref TEXT NOT NULL,  -- reference to keychain entry
        created_at      TEXT NOT NULL DEFAULT (datetime('now'))
    );

    -- Trust group
    CREATE TABLE IF NOT EXISTS trust_group (
        group_id            TEXT PRIMARY KEY,
        group_key_encrypted BLOB NOT NULL,
        created_at          TEXT NOT NULL DEFAULT (datetime('now'))
    );

    -- Paired devices in the trust group
    CREATE TABLE IF NOT EXISTS paired_devices (
        device_id   TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        type        TEXT NOT NULL CHECK(type IN ('laptop','phone','tablet','desktop')),
        public_key  BLOB NOT NULL,
        group_id    TEXT NOT NULL REFERENCES trust_group(group_id),
        paired_at   TEXT NOT NULL DEFAULT (datetime('now')),
        last_seen_at TEXT
    );

    -- Transfer history
    CREATE TABLE IF NOT EXISTS transfers (
        transfer_id      TEXT PRIMARY KEY,
        direction        TEXT NOT NULL CHECK(direction IN ('sent','received')),
        peer_device_id   TEXT NOT NULL,
        file_name        TEXT NOT NULL,
        file_size        INTEGER NOT NULL,
        mime_type        TEXT,
        sha256           TEXT NOT NULL,
        status           TEXT NOT NULL CHECK(status IN ('queued','in_progress','completed','failed','cancelled')),
        chunks_total     INTEGER NOT NULL,
        chunks_completed INTEGER NOT NULL DEFAULT 0,
        started_at       TEXT NOT NULL DEFAULT (datetime('now')),
        completed_at     TEXT,
        error_message    TEXT
    );

    -- Storage configuration
    CREATE TABLE IF NOT EXISTS storage_config (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    -- Indexes for common queries
    CREATE INDEX IF NOT EXISTS idx_transfers_status ON transfers(status);
    CREATE INDEX IF NOT EXISTS idx_transfers_peer ON transfers(peer_device_id);
    CREATE INDEX IF NOT EXISTS idx_paired_devices_group ON paired_devices(group_id);
"#;

/// Migration 002: Async file queue for store-and-forward delivery
pub const MIGRATION_002: &str = r#"
    -- Async file queue: files waiting to be delivered
    CREATE TABLE IF NOT EXISTS file_queue (
        queue_id         TEXT PRIMARY KEY,
        target_device_id TEXT NOT NULL,
        file_name        TEXT NOT NULL,
        file_size        INTEGER NOT NULL,
        mime_type        TEXT,
        sha256           TEXT NOT NULL,
        staging_path     TEXT NOT NULL,      -- path to encrypted staged file
        status           TEXT NOT NULL DEFAULT 'pending'
                         CHECK(status IN ('pending','delivering','delivered','failed','expired','cancelled')),
        retry_count      INTEGER NOT NULL DEFAULT 0,
        max_retries      INTEGER NOT NULL DEFAULT 10,
        created_at       TEXT NOT NULL DEFAULT (datetime('now')),
        expires_at       TEXT NOT NULL,       -- default 7 days from creation
        delivered_at     TEXT,
        last_attempt_at  TEXT,
        error_message    TEXT
    );

    -- Index for the delivery watcher: find pending items for a specific device
    CREATE INDEX IF NOT EXISTS idx_queue_target_status
        ON file_queue(target_device_id, status);

    -- Index for cleanup: find expired items
    CREATE INDEX IF NOT EXISTS idx_queue_expires
        ON file_queue(expires_at) WHERE status = 'pending';

    -- Index for UI: show all pending items ordered by creation
    CREATE INDEX IF NOT EXISTS idx_queue_pending
        ON file_queue(status, created_at) WHERE status IN ('pending', 'delivering');
"#;
