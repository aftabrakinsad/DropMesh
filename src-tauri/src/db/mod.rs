pub mod schema;

use rusqlite::{Connection, Result};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// Database wrapper managing the SQLite connection
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Initialize the database at the app's data directory
    pub fn new(app: &AppHandle) -> Result<Self> {
        let data_dir = app
            .path()
            .app_data_dir()
            .expect("Failed to resolve app data directory");

        std::fs::create_dir_all(&data_dir)
            .expect("Failed to create app data directory");

        let db_path: PathBuf = data_dir.join("dropmesh.db");
        let conn = Connection::open(&db_path)?;

        // Enable WAL mode for better concurrent read performance
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;

        log::info!("Database opened at {:?}", db_path);
        Ok(Self { conn })
    }

    /// Run all pending migrations
    pub fn run_migrations(&self) -> Result<()> {
        self.conn.execute_batch(schema::MIGRATION_001)?;
        self.conn.execute_batch(schema::MIGRATION_002)?;
        log::info!("Database migrations complete");
        Ok(())
    }

    /// Get a reference to the underlying connection
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
