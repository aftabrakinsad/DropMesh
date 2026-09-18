use crate::db::Database;
use crate::models::transfer::StorageStats;
use std::path::{Path, PathBuf};

/// Default storage quota: 2 GB
const DEFAULT_QUOTA: u64 = 2_147_483_648;

/// Default file size cap: 2 GB
const DEFAULT_FILE_SIZE_CAP: u64 = 2_147_483_648;

/// Manages the user-allocated storage folder and quota
pub struct StorageManager {
    folder_path: PathBuf,
    quota_bytes: u64,
    file_size_cap: u64,
}

impl StorageManager {
    pub fn sanitize_filename(filename: &str) -> Option<String> {
        use std::path::{Component, Path};

        let path = Path::new(filename);
        if path.is_absolute() {
            return None;
        }

        let mut saw_normal = false;
        for component in path.components() {
            match component {
                Component::Normal(_) => saw_normal = true,
                _ => return None,
            }
        }
        if !saw_normal {
            return None;
        }

        let basename = path.file_name()?.to_string_lossy().trim().to_string();
        if basename.is_empty() {
            return None;
        }
        Some(basename)
    }

    pub fn received_root(&self) -> PathBuf {
        self.folder_path.join("received")
    }

    /// Initialize the storage manager, loading config from DB or using defaults
    pub fn new(db: &Database) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Try to load existing config
        let folder_path = Self::get_config(db, "folder_path")
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("DropMesh")
                    .to_string_lossy()
                    .to_string()
            });

        let quota_bytes = Self::get_config(db, "quota_bytes")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_QUOTA);

        let file_size_cap = Self::get_config(db, "file_size_cap_bytes")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_FILE_SIZE_CAP);

        let folder = PathBuf::from(&folder_path);

        // Create the folder structure
        Self::ensure_folder_structure(&folder)?;

        log::info!(
            "Storage initialized: {} (quota: {} MB)",
            folder_path,
            quota_bytes / 1_048_576
        );

        Ok(Self {
            folder_path: folder,
            quota_bytes,
            file_size_cap,
        })
    }

    /// Create the required subdirectory structure
    fn ensure_folder_structure(base: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let dirs = ["incoming", "received", "staging", ".dropmesh"];
        for dir in &dirs {
            std::fs::create_dir_all(base.join(dir))?;
        }
        Ok(())
    }

    /// Get a config value from the database
    fn get_config(db: &Database, key: &str) -> Option<String> {
        db.conn()
            .query_row(
                "SELECT value FROM storage_config WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .ok()
    }

    /// Save a config value to the database
    fn set_config(
        db: &Database,
        key: &str,
        value: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        db.conn().execute(
            "INSERT OR REPLACE INTO storage_config (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// Calculate the total size of files in a directory (non-recursive for top-level)
    fn dir_size(path: &Path) -> u64 {
        if !path.exists() {
            return 0;
        }
        walkdir::WalkDir::new(path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum()
    }

    /// Get current storage usage statistics
    pub fn get_stats(&self) -> StorageStats {
        let incoming_size = Self::dir_size(&self.folder_path.join("incoming"));
        let received_size = Self::dir_size(&self.folder_path.join("received"));
        let staging_size = Self::dir_size(&self.folder_path.join("staging"));
        let used = incoming_size + received_size + staging_size;

        let file_count = std::fs::read_dir(self.folder_path.join("received"))
            .map(|entries| entries.filter_map(|e| e.ok()).count() as u32)
            .unwrap_or(0);

        StorageStats {
            folder_path: self.folder_path.to_string_lossy().to_string(),
            quota_bytes: self.quota_bytes,
            used_bytes: used,
            file_size_cap_bytes: self.file_size_cap,
            file_count,
        }
    }

    /// Check if there's enough space for an incoming file
    pub fn can_accept_file(&self, file_size: u64) -> bool {
        if file_size > self.file_size_cap {
            return false;
        }
        let stats = self.get_stats();
        stats.used_bytes + file_size <= self.quota_bytes
    }

    /// Get the path for incoming chunks of a transfer
    pub fn incoming_path(&self, transfer_id: &str) -> PathBuf {
        self.folder_path.join("incoming").join(transfer_id)
    }

    /// Get the path for a received file
    pub fn received_path(&self, filename: &str) -> PathBuf {
        let safe_name = Self::sanitize_filename(filename)
            .unwrap_or_else(|| "received_file".to_string());
        let received_dir = self.received_root();
        let base = received_dir.join(safe_name);

        // Handle filename conflicts
        if !base.exists() {
            return base;
        }

        let stem = base.file_stem().unwrap_or_default().to_string_lossy();
        let ext = base.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();

        for i in 1..1000 {
            let candidate = received_dir.join(format!("{} ({}){}", stem, i, ext));
            if !candidate.exists() {
                return candidate;
            }
        }

        // Fallback: append UUID
        received_dir.join(format!(
            "{}_{}{}",
            stem,
            uuid::Uuid::new_v4(),
            ext
        ))
    }

    /// Get the staging path for a file being sent
    pub fn staging_path(&self, filename: &str) -> PathBuf {
        self.folder_path.join("staging").join(filename)
    }

    /// Update the storage folder path
    pub fn set_folder_path(
        &mut self,
        db: &Database,
        new_path: PathBuf,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Self::ensure_folder_structure(&new_path)?;
        Self::set_config(db, "folder_path", &new_path.to_string_lossy())?;
        self.folder_path = new_path;
        Ok(())
    }

    /// Update the storage quota
    pub fn set_quota(
        &mut self,
        db: &Database,
        quota_bytes: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Self::set_config(db, "quota_bytes", &quota_bytes.to_string())?;
        self.quota_bytes = quota_bytes;
        Ok(())
    }

    /// Update the file size cap
    pub fn set_file_size_cap(
        &mut self,
        db: &Database,
        cap_bytes: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Self::set_config(db, "file_size_cap_bytes", &cap_bytes.to_string())?;
        self.file_size_cap = cap_bytes;
        Ok(())
    }

    /// Clean up partial transfers (orphaned incoming directories)
    pub fn cleanup_partial_transfers(&self) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
        let incoming = self.folder_path.join("incoming");
        let mut cleaned = 0;

        if let Ok(entries) = std::fs::read_dir(&incoming) {
            for entry in entries.filter_map(|e| e.ok()) {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    std::fs::remove_dir_all(entry.path())?;
                    cleaned += 1;
                }
            }
        }

        if cleaned > 0 {
            log::info!("Cleaned up {} partial transfers", cleaned);
        }
        Ok(cleaned)
    }
}
