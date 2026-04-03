// FORGE key-value storage — issue #48
// redb-backed persistent store for agent memory and data.store/data.get.

use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableTable, TableDefinition};

const FORGE_KV: TableDefinition<&str, &str> = TableDefinition::new("forge_kv");

/// ACID key-value store backed by redb.
pub struct ForgeStorage {
    db: Database,
}

/// Shared handle to a ForgeStorage instance.
pub type SharedStorage = Arc<ForgeStorage>;

impl ForgeStorage {
    /// Open (or create) a redb database at the given path.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let db = Database::create(path).map_err(StorageError::Open)?;

        // Ensure the table exists by running an empty write transaction.
        let txn = db.begin_write().map_err(StorageError::Transaction)?;
        txn.open_table(FORGE_KV).map_err(StorageError::Table)?;
        txn.commit().map_err(StorageError::Commit)?;

        Ok(Self { db })
    }

    /// Store a key-value pair. Overwrites any existing value.
    pub fn store(&self, key: &str, value: &str) -> Result<(), StorageError> {
        let txn = self.db.begin_write().map_err(StorageError::Transaction)?;
        {
            let mut table = txn.open_table(FORGE_KV).map_err(StorageError::Table)?;
            table.insert(key, value).map_err(StorageError::Write)?;
        }
        txn.commit().map_err(StorageError::Commit)?;
        Ok(())
    }

    /// Get a value by key. Returns None if the key does not exist.
    pub fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        let txn = self.db.begin_read().map_err(StorageError::Transaction)?;
        let table = txn.open_table(FORGE_KV).map_err(StorageError::Table)?;
        match table.get(key).map_err(StorageError::Read)? {
            Some(val) => Ok(Some(val.value().to_string())),
            None => Ok(None),
        }
    }

    /// Delete a key. Returns true if the key existed.
    pub fn delete(&self, key: &str) -> Result<bool, StorageError> {
        let txn = self.db.begin_write().map_err(StorageError::Transaction)?;
        let existed;
        {
            let mut table = txn.open_table(FORGE_KV).map_err(StorageError::Table)?;
            existed = table.remove(key).map_err(StorageError::Write)?.is_some();
        }
        txn.commit().map_err(StorageError::Commit)?;
        Ok(existed)
    }

    /// List all keys that start with the given prefix.
    pub fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        let txn = self.db.begin_read().map_err(StorageError::Transaction)?;
        let table = txn.open_table(FORGE_KV).map_err(StorageError::Table)?;
        let mut keys = Vec::new();
        for entry in table.iter().map_err(StorageError::Read)? {
            let (k, _v) = entry.map_err(StorageError::Read)?;
            let key = k.value();
            if key.starts_with(prefix) {
                keys.push(key.to_string());
            }
        }
        Ok(keys)
    }
}

// ── Errors ──────────────────────────────────────────────────────

#[derive(Debug)]
pub enum StorageError {
    Open(redb::DatabaseError),
    Transaction(redb::TransactionError),
    Table(redb::TableError),
    Commit(redb::CommitError),
    Write(redb::StorageError),
    Read(redb::StorageError),
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
