// FORGE key-value storage — issue #48
// redb-backed persistent store for agent memory and data.store/data.get.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::config::StorageConfig;

static LEGACY_WARN: OnceLock<()> = OnceLock::new();

/// Expand `~` (home dir) and `${VAR}` tokens in a path string.
///
/// Matches the existing conventions in `config.rs` for consistency.
fn expand_path(raw: &str) -> PathBuf {
    let expanded_env = if raw.starts_with("${") {
        if let Some(end) = raw.find('}') {
            let var_name = &raw[2..end];
            let rest = &raw[end + 1..];
            match std::env::var(var_name) {
                Ok(val) => format!("{val}{rest}"),
                Err(_) => raw.to_string(),
            }
        } else {
            raw.to_string()
        }
    } else {
        raw.to_string()
    };

    if let Some(stripped) = expanded_env.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    } else if expanded_env == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(expanded_env)
}

const FORGE_KV: TableDefinition<&str, &str> = TableDefinition::new("forge_kv");

/// ACID key-value store backed by redb.
pub struct ForgeStorage {
    db: Database,
}

/// Shared handle to a ForgeStorage instance.
pub type SharedStorage = Arc<ForgeStorage>;

impl ForgeStorage {
    /// Resolve the storage root directory using the following precedence:
    /// 1. `FORGE_STORAGE_ROOT` env var
    /// 2. `[storage] root` from config
    /// 3. `knowledge_store_path` (backward compat for agents declaring `knowledge { store_path }`)
    /// 4. `./.forge-data` (legacy default; emits a one-shot warning)
    pub fn resolve_root(
        config: Option<&StorageConfig>,
        knowledge_store_path: Option<&str>,
    ) -> PathBuf {
        if let Ok(env_root) = std::env::var("FORGE_STORAGE_ROOT") {
            if !env_root.is_empty() {
                return expand_path(&env_root);
            }
        }
        if let Some(root) = config.and_then(|c| c.root.as_deref()) {
            return expand_path(root);
        }
        if let Some(ks) = knowledge_store_path {
            return expand_path(ks);
        }
        LEGACY_WARN.get_or_init(|| {
            eprintln!(
                "warning: no [storage] root configured; using legacy default './.forge-data'. \
                 Set [storage] root in forge.config.toml to silence this warning."
            );
        });
        PathBuf::from(".forge-data")
    }

    /// Open `<root>/<filename>` using `resolve_root`. Creates the directory if missing.
    pub fn open_from_config(
        config: Option<&StorageConfig>,
        knowledge_store_path: Option<&str>,
        filename: &str,
    ) -> Result<Self, StorageError> {
        let root = Self::resolve_root(config, knowledge_store_path);
        std::fs::create_dir_all(&root).map_err(StorageError::Io)?;
        Self::open(&root.join(filename))
    }

    /// Open (or create) a redb database at the given path.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let db = Database::create(path).map_err(|e| StorageError::Open(Box::new(e)))?;

        // Ensure the table exists by running an empty write transaction.
        let txn = db
            .begin_write()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        txn.open_table(FORGE_KV)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        txn.commit()
            .map_err(|e| StorageError::Commit(Box::new(e)))?;

        Ok(Self { db })
    }

    /// Store a key-value pair. Overwrites any existing value.
    pub fn store(&self, key: &str, value: &str) -> Result<(), StorageError> {
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        {
            let mut table = txn
                .open_table(FORGE_KV)
                .map_err(|e| StorageError::Table(Box::new(e)))?;
            table
                .insert(key, value)
                .map_err(|e| StorageError::Write(Box::new(e)))?;
        }
        txn.commit()
            .map_err(|e| StorageError::Commit(Box::new(e)))?;
        Ok(())
    }

    /// Get a value by key. Returns None if the key does not exist.
    pub fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        let txn = self
            .db
            .begin_read()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        let table = txn
            .open_table(FORGE_KV)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        match table
            .get(key)
            .map_err(|e| StorageError::Read(Box::new(e)))?
        {
            Some(val) => Ok(Some(val.value().to_string())),
            None => Ok(None),
        }
    }

    /// Delete a key. Returns true if the key existed.
    pub fn delete(&self, key: &str) -> Result<bool, StorageError> {
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        let existed;
        {
            let mut table = txn
                .open_table(FORGE_KV)
                .map_err(|e| StorageError::Table(Box::new(e)))?;
            existed = table
                .remove(key)
                .map_err(|e| StorageError::Write(Box::new(e)))?
                .is_some();
        }
        txn.commit()
            .map_err(|e| StorageError::Commit(Box::new(e)))?;
        Ok(existed)
    }

    /// List all keys that start with the given prefix.
    pub fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        let txn = self
            .db
            .begin_read()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        let table = txn
            .open_table(FORGE_KV)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        let mut keys = Vec::new();
        for entry in table.iter().map_err(|e| StorageError::Read(Box::new(e)))? {
            let (k, _v) = entry.map_err(|e| StorageError::Read(Box::new(e)))?;
            let key = k.value();
            if key.starts_with(prefix) {
                keys.push(key.to_string());
            }
        }
        Ok(keys)
    }

    /// List all keys matching prefix with their value sizes (in bytes).
    /// Runs in a single read transaction for consistency.
    pub fn list_with_sizes(&self, prefix: &str) -> Result<Vec<(String, usize)>, StorageError> {
        let txn = self
            .db
            .begin_read()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        let table = txn
            .open_table(FORGE_KV)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        let mut entries = Vec::new();
        for entry in table.iter().map_err(|e| StorageError::Read(Box::new(e)))? {
            let (k, v) = entry.map_err(|e| StorageError::Read(Box::new(e)))?;
            let key = k.value();
            if key.starts_with(prefix) {
                entries.push((key.to_string(), v.value().len()));
            }
        }
        Ok(entries)
    }
}

// ── Errors ──────────────────────────────────────────────────────

#[derive(Debug)]
pub enum StorageError {
    Open(Box<redb::DatabaseError>),
    Transaction(Box<redb::TransactionError>),
    Table(Box<redb::TableError>),
    Commit(Box<redb::CommitError>),
    Write(Box<redb::StorageError>),
    Read(Box<redb::StorageError>),
    Io(std::io::Error),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Open(e) => write!(f, "storage open: {e}"),
            StorageError::Transaction(e) => write!(f, "storage transaction: {e}"),
            StorageError::Table(e) => write!(f, "storage table: {e}"),
            StorageError::Commit(e) => write!(f, "storage commit: {e}"),
            StorageError::Write(e) => write!(f, "storage write: {e}"),
            StorageError::Read(e) => write!(f, "storage read: {e}"),
            StorageError::Io(e) => write!(f, "storage io: {e}"),
        }
    }
}

impl std::error::Error for StorageError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_storage() -> (tempfile::TempDir, ForgeStorage) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.redb");
        let storage = ForgeStorage::open(&db_path).unwrap();
        (dir, storage)
    }

    #[test]
    fn store_and_get() {
        let (_dir, storage) = temp_storage();
        storage.store("key1", "value1").unwrap();
        assert_eq!(storage.get("key1").unwrap(), Some("value1".to_string()));
    }

    #[test]
    fn get_missing_key() {
        let (_dir, storage) = temp_storage();
        assert_eq!(storage.get("nonexistent").unwrap(), None);
    }

    #[test]
    fn overwrite_existing() {
        let (_dir, storage) = temp_storage();
        storage.store("key1", "v1").unwrap();
        storage.store("key1", "v2").unwrap();
        assert_eq!(storage.get("key1").unwrap(), Some("v2".to_string()));
    }

    #[test]
    fn delete_existing() {
        let (_dir, storage) = temp_storage();
        storage.store("key1", "v1").unwrap();
        assert!(storage.delete("key1").unwrap());
        assert_eq!(storage.get("key1").unwrap(), None);
    }

    #[test]
    fn delete_missing() {
        let (_dir, storage) = temp_storage();
        assert!(!storage.delete("nonexistent").unwrap());
    }

    #[test]
    fn list_with_prefix() {
        let (_dir, storage) = temp_storage();
        storage.store("agent:foo:memory", "{}").unwrap();
        storage.store("agent:bar:memory", "{}").unwrap();
        storage.store("other:key", "{}").unwrap();

        let mut keys = storage.list("agent:").unwrap();
        keys.sort();
        assert_eq!(keys, vec!["agent:bar:memory", "agent:foo:memory"]);
    }

    #[test]
    fn list_with_sizes_returns_key_and_byte_len() {
        let (_dir, storage) = temp_storage();
        storage.store("agent:foo:memory", r#"{"x":1}"#).unwrap();
        storage.store("agent:bar:memory", "{}").unwrap();
        storage.store("other:key", "hello").unwrap();

        let mut entries = storage.list_with_sizes("agent:").unwrap();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], ("agent:bar:memory".to_string(), 2)); // "{}"
        assert_eq!(entries[1], ("agent:foo:memory".to_string(), 7)); // r#"{"x":1}"#
    }

    #[test]
    fn persistence_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.redb");

        {
            let storage = ForgeStorage::open(&db_path).unwrap();
            storage.store("key1", "persisted").unwrap();
        }

        let storage = ForgeStorage::open(&db_path).unwrap();
        assert_eq!(storage.get("key1").unwrap(), Some("persisted".to_string()));
    }
}
