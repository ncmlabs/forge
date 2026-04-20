// FORGE key-value storage — issue #48
// redb-backed persistent store for agent memory and data.store/data.get.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

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

/// Per-agent schedule rows for `WakeService` (issue #332).
/// Key format: `"{agent_name}:{schedule_name}"`.
/// Value: JSON-encoded `ScheduleState`.
const FORGE_SCHEDULES: TableDefinition<&str, &str> = TableDefinition::new("forge_schedules");

/// Correlation rows for `CorrelationDriver` (issue #334).
/// Key format: `"{agent_name}:{field_name}:{field_value}"`.
/// Value: target agent alias to rehydrate when the correlated event arrives.
const FORGE_CORRELATIONS: TableDefinition<&str, &str> = TableDefinition::new("forge_correlations");

/// Per-`(agent, trigger)` HMAC secret for `WebhookDriver` (issue #335).
/// Key format: `"{agent_name}:{trigger_name}"`. Value: hex-encoded 32 random
/// bytes. Written only via the `forge wake` CLI; never emitted to logs or
/// tracer events. The verifier reads a secret exactly once per inbound request.
const FORGE_WAKE_SECRETS: TableDefinition<&str, &str> = TableDefinition::new("forge_wake_secrets");

/// Terminal status of the most recent schedule dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleStatus {
    Pending,
    Success,
    Error,
    SkippedConcurrent,
    SkippedBudget,
    Halted,
}

/// Persistent state for one (agent, schedule) pair.
///
/// Backed by the `FORGE_SCHEDULES` redb table. All mutations open a write txn;
/// redb serializes concurrent writers within a process, which is exactly the
/// ordering `WakeService` relies on for its transactional claim.
///
/// Cross-process isolation is enforced by redb at the database-open layer
/// (a second `Database::create` on the same file fails with "Database already
/// open"). This is a stronger guarantee than a cooperative lock file — no two
/// `forge serve` processes can share `.forge-data/` at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleState {
    pub next_run_at_ms: u64,
    #[serde(default)]
    pub last_run_at_ms: Option<u64>,
    #[serde(default = "default_status")]
    pub last_status: ScheduleStatus,
    #[serde(default)]
    pub consecutive_errors: u32,
    #[serde(default)]
    pub claimed_by: Option<String>,
    #[serde(default)]
    pub claim_expires_at_ms: Option<u64>,
}

fn default_status() -> ScheduleStatus {
    ScheduleStatus::Pending
}

impl ScheduleState {
    pub fn fresh(next_run_at_ms: u64) -> Self {
        Self {
            next_run_at_ms,
            last_run_at_ms: None,
            last_status: ScheduleStatus::Pending,
            consecutive_errors: 0,
            claimed_by: None,
            claim_expires_at_ms: None,
        }
    }

    pub fn is_claim_live(&self, now_ms: u64) -> bool {
        matches!((self.claimed_by.as_ref(), self.claim_expires_at_ms),
            (Some(_), Some(exp)) if exp > now_ms)
    }
}

/// Outcome of `try_claim_schedule`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// This caller now holds the claim.
    Claimed { state: ScheduleState },
    /// Another holder has a live claim; caller must not fire.
    Lost {
        winner: String,
        claim_expires_at_ms: u64,
    },
    /// No row exists yet (schedule not registered).
    NotRegistered,
}

fn schedule_key(agent: &str, schedule: &str) -> String {
    format!("{agent}:{schedule}")
}

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

        // Ensure both tables exist by running an empty write transaction.
        let txn = db
            .begin_write()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        txn.open_table(FORGE_KV)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        txn.open_table(FORGE_SCHEDULES)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        txn.open_table(FORGE_CORRELATIONS)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        txn.open_table(FORGE_WAKE_SECRETS)
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

    // ── Schedule state (issue #332) ─────────────────────────────────

    /// Upsert a schedule's full state. Used on registration, after a successful
    /// fire, and when releasing or transitioning a claim.
    pub fn upsert_schedule_state(
        &self,
        agent: &str,
        schedule: &str,
        state: &ScheduleState,
    ) -> Result<(), StorageError> {
        let key = schedule_key(agent, schedule);
        let value = serde_json::to_string(state).map_err(StorageError::Json)?;
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        {
            let mut table = txn
                .open_table(FORGE_SCHEDULES)
                .map_err(|e| StorageError::Table(Box::new(e)))?;
            table
                .insert(key.as_str(), value.as_str())
                .map_err(|e| StorageError::Write(Box::new(e)))?;
        }
        txn.commit()
            .map_err(|e| StorageError::Commit(Box::new(e)))?;
        Ok(())
    }

    /// Fetch a schedule's state, or None if no row exists.
    pub fn get_schedule_state(
        &self,
        agent: &str,
        schedule: &str,
    ) -> Result<Option<ScheduleState>, StorageError> {
        let key = schedule_key(agent, schedule);
        let txn = self
            .db
            .begin_read()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        let table = txn
            .open_table(FORGE_SCHEDULES)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        match table
            .get(key.as_str())
            .map_err(|e| StorageError::Read(Box::new(e)))?
        {
            Some(val) => {
                let s: ScheduleState =
                    serde_json::from_str(val.value()).map_err(StorageError::Json)?;
                Ok(Some(s))
            }
            None => Ok(None),
        }
    }

    /// Delete a schedule row. Returns true if the row existed.
    pub fn delete_schedule(&self, agent: &str, schedule: &str) -> Result<bool, StorageError> {
        let key = schedule_key(agent, schedule);
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        let existed;
        {
            let mut table = txn
                .open_table(FORGE_SCHEDULES)
                .map_err(|e| StorageError::Table(Box::new(e)))?;
            existed = table
                .remove(key.as_str())
                .map_err(|e| StorageError::Write(Box::new(e)))?
                .is_some();
        }
        txn.commit()
            .map_err(|e| StorageError::Commit(Box::new(e)))?;
        Ok(existed)
    }

    /// List every (schedule_name, state) pair for the given agent.
    pub fn list_schedules_for_agent(
        &self,
        agent: &str,
    ) -> Result<Vec<(String, ScheduleState)>, StorageError> {
        let prefix = format!("{agent}:");
        let txn = self
            .db
            .begin_read()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        let table = txn
            .open_table(FORGE_SCHEDULES)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        let mut out = Vec::new();
        for entry in table.iter().map_err(|e| StorageError::Read(Box::new(e)))? {
            let (k, v) = entry.map_err(|e| StorageError::Read(Box::new(e)))?;
            let key = k.value();
            if let Some(schedule_name) = key.strip_prefix(&prefix) {
                let state: ScheduleState =
                    serde_json::from_str(v.value()).map_err(StorageError::Json)?;
                out.push((schedule_name.to_string(), state));
            }
        }
        Ok(out)
    }

    /// Attempt to take the claim on a schedule for the next `ttl_ms` window.
    ///
    /// The full read-modify-write executes inside a single redb write
    /// transaction, which redb serializes across concurrent writers within
    /// the process. Two dispatchers racing on the same schedule are guaranteed
    /// exactly one `Claimed` outcome; the loser sees `Lost`.
    ///
    /// Expired claims (stale after a process crash) are overwritten.
    pub fn try_claim_schedule(
        &self,
        agent: &str,
        schedule: &str,
        instance_id: &str,
        now_ms: u64,
        ttl_ms: u64,
    ) -> Result<ClaimOutcome, StorageError> {
        let key = schedule_key(agent, schedule);
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;

        let outcome = {
            let mut table = txn
                .open_table(FORGE_SCHEDULES)
                .map_err(|e| StorageError::Table(Box::new(e)))?;

            let current: Option<ScheduleState> = table
                .get(key.as_str())
                .map_err(|e| StorageError::Read(Box::new(e)))?
                .map(|v| serde_json::from_str(v.value()))
                .transpose()
                .map_err(StorageError::Json)?;

            let Some(mut state) = current else {
                drop(table);
                // Abort errors are cleanup-only; dropping is semantically fine
                // and keeps the error surface narrow.
                let _ = txn.abort();
                return Ok(ClaimOutcome::NotRegistered);
            };

            if state.is_claim_live(now_ms) {
                // Someone else holds a live claim — do not steal it.
                let winner = state.claimed_by.clone().unwrap_or_default();
                let exp = state.claim_expires_at_ms.unwrap_or(now_ms);
                ClaimOutcome::Lost {
                    winner,
                    claim_expires_at_ms: exp,
                }
            } else {
                state.claimed_by = Some(instance_id.to_string());
                state.claim_expires_at_ms = Some(now_ms + ttl_ms);
                let value = serde_json::to_string(&state).map_err(StorageError::Json)?;
                table
                    .insert(key.as_str(), value.as_str())
                    .map_err(|e| StorageError::Write(Box::new(e)))?;
                ClaimOutcome::Claimed { state }
            }
        };

        txn.commit()
            .map_err(|e| StorageError::Commit(Box::new(e)))?;
        Ok(outcome)
    }

    // ── Correlation state (issue #334) ──────────────────────────────

    /// Upsert a single correlation row. Used when an agent's `memory persistent`
    /// write changes a declared correlation field.
    pub fn upsert_correlation(
        &self,
        agent: &str,
        field: &str,
        value: &str,
        target_alias: &str,
    ) -> Result<(), StorageError> {
        let key = correlation_key(agent, field, value);
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        {
            let mut table = txn
                .open_table(FORGE_CORRELATIONS)
                .map_err(|e| StorageError::Table(Box::new(e)))?;
            table
                .insert(key.as_str(), target_alias)
                .map_err(|e| StorageError::Write(Box::new(e)))?;
        }
        txn.commit()
            .map_err(|e| StorageError::Commit(Box::new(e)))?;
        Ok(())
    }

    /// Look up the target agent alias for a given correlation tuple.
    pub fn lookup_correlation(
        &self,
        agent: &str,
        field: &str,
        value: &str,
    ) -> Result<Option<String>, StorageError> {
        let key = correlation_key(agent, field, value);
        let txn = self
            .db
            .begin_read()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        let table = txn
            .open_table(FORGE_CORRELATIONS)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        match table
            .get(key.as_str())
            .map_err(|e| StorageError::Read(Box::new(e)))?
        {
            Some(v) => Ok(Some(v.value().to_string())),
            None => Ok(None),
        }
    }

    /// List every `(agent, schedule, state)` row across the schedule table.
    /// Used by the `/__forge/inspect/schedules` introspection endpoint (#336).
    pub fn list_all_schedules(&self) -> Result<Vec<(String, String, ScheduleState)>, StorageError> {
        let txn = self
            .db
            .begin_read()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        let table = txn
            .open_table(FORGE_SCHEDULES)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        let mut out = Vec::new();
        for entry in table.iter().map_err(|e| StorageError::Read(Box::new(e)))? {
            let (k, v) = entry.map_err(|e| StorageError::Read(Box::new(e)))?;
            let key = k.value();
            // Schedule keys are `"{agent}:{schedule}"` — split on the first `:`.
            if let Some((agent, schedule)) = key.split_once(':') {
                let state: ScheduleState =
                    serde_json::from_str(v.value()).map_err(StorageError::Json)?;
                out.push((agent.to_string(), schedule.to_string(), state));
            }
        }
        Ok(out)
    }

    /// Summarise correlation rows by `(agent, field)` with the count of distinct
    /// values present. The endpoint surface deliberately does not expose the
    /// values themselves — those can be sensitive (thread IDs, user IDs, etc.)
    /// and the count is enough for the "is this agent correlated on X?" view.
    pub fn list_all_correlations(&self) -> Result<Vec<(String, String, u64)>, StorageError> {
        let txn = self
            .db
            .begin_read()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        let table = txn
            .open_table(FORGE_CORRELATIONS)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        let mut counts: std::collections::BTreeMap<(String, String), u64> =
            std::collections::BTreeMap::new();
        for entry in table.iter().map_err(|e| StorageError::Read(Box::new(e)))? {
            let (k, _v) = entry.map_err(|e| StorageError::Read(Box::new(e)))?;
            // Keys are `"{agent}:{field}:{value}"` — split on the first two `:`.
            let key = k.value();
            let mut parts = key.splitn(3, ':');
            if let (Some(agent), Some(field), Some(_value)) =
                (parts.next(), parts.next(), parts.next())
            {
                *counts
                    .entry((agent.to_string(), field.to_string()))
                    .or_insert(0) += 1;
            }
        }
        Ok(counts
            .into_iter()
            .map(|((agent, field), count)| (agent, field, count))
            .collect())
    }

    /// Atomically persist an agent's memory blob alongside any correlation
    /// upserts derived from that memory. Both writes land in a single redb
    /// write transaction — there is no window where memory and correlation
    /// state can diverge.
    ///
    /// `rows` contains `(agent, field, value, target_alias)` tuples for every
    /// declared correlation whose value is present in this memory snapshot.
    pub fn store_memory_with_correlations(
        &self,
        mem_key: &str,
        mem_json: &str,
        rows: &[(String, String, String, String)],
    ) -> Result<(), StorageError> {
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        {
            let mut kv = txn
                .open_table(FORGE_KV)
                .map_err(|e| StorageError::Table(Box::new(e)))?;
            kv.insert(mem_key, mem_json)
                .map_err(|e| StorageError::Write(Box::new(e)))?;
        }
        if !rows.is_empty() {
            let mut corr = txn
                .open_table(FORGE_CORRELATIONS)
                .map_err(|e| StorageError::Table(Box::new(e)))?;
            for (agent, field, value, target) in rows {
                let key = correlation_key(agent, field, value);
                corr.insert(key.as_str(), target.as_str())
                    .map_err(|e| StorageError::Write(Box::new(e)))?;
            }
        }
        txn.commit()
            .map_err(|e| StorageError::Commit(Box::new(e)))?;
        Ok(())
    }
}

fn correlation_key(agent: &str, field: &str, value: &str) -> String {
    format!("{agent}:{field}:{value}")
}

fn wake_secret_key(agent: &str, trigger: &str) -> String {
    format!("{agent}:{trigger}")
}

impl ForgeStorage {
    // ── Wake-webhook secrets (issue #335) ──────────────────────────────
    //
    // Secrets are hex-encoded bytes shared with an external caller. The
    // storage layer is deliberately narrow: upsert, lookup, list, delete.
    // Nothing here logs or serializes the secret value. The `list_` path
    // returns identifiers only.

    /// Store or replace the HMAC secret for `(agent, trigger)`.
    pub fn upsert_wake_secret(
        &self,
        agent: &str,
        trigger: &str,
        secret: &str,
    ) -> Result<(), StorageError> {
        let key = wake_secret_key(agent, trigger);
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        {
            let mut table = txn
                .open_table(FORGE_WAKE_SECRETS)
                .map_err(|e| StorageError::Table(Box::new(e)))?;
            table
                .insert(key.as_str(), secret)
                .map_err(|e| StorageError::Write(Box::new(e)))?;
        }
        txn.commit()
            .map_err(|e| StorageError::Commit(Box::new(e)))?;
        Ok(())
    }

    /// Read the HMAC secret for `(agent, trigger)`, or `None` if not registered.
    pub fn lookup_wake_secret(
        &self,
        agent: &str,
        trigger: &str,
    ) -> Result<Option<String>, StorageError> {
        let key = wake_secret_key(agent, trigger);
        let txn = self
            .db
            .begin_read()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        let table = txn
            .open_table(FORGE_WAKE_SECRETS)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        match table
            .get(key.as_str())
            .map_err(|e| StorageError::Read(Box::new(e)))?
        {
            Some(v) => Ok(Some(v.value().to_string())),
            None => Ok(None),
        }
    }

    /// List every registered `(agent, trigger)` pair. Secret material is
    /// deliberately excluded from the return type.
    pub fn list_wake_triggers(&self) -> Result<Vec<(String, String)>, StorageError> {
        let txn = self
            .db
            .begin_read()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        let table = txn
            .open_table(FORGE_WAKE_SECRETS)
            .map_err(|e| StorageError::Table(Box::new(e)))?;
        let mut out = Vec::new();
        for row in table.iter().map_err(|e| StorageError::Read(Box::new(e)))? {
            let (k, _) = row.map_err(|e| StorageError::Read(Box::new(e)))?;
            let key = k.value().to_string();
            if let Some((agent, trigger)) = key.split_once(':') {
                out.push((agent.to_string(), trigger.to_string()));
            }
        }
        Ok(out)
    }

    /// Delete the HMAC secret for `(agent, trigger)`. Returns `true` if a row
    /// was removed.
    pub fn delete_wake_secret(&self, agent: &str, trigger: &str) -> Result<bool, StorageError> {
        let key = wake_secret_key(agent, trigger);
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StorageError::Transaction(Box::new(e)))?;
        let removed;
        {
            let mut table = txn
                .open_table(FORGE_WAKE_SECRETS)
                .map_err(|e| StorageError::Table(Box::new(e)))?;
            removed = table
                .remove(key.as_str())
                .map_err(|e| StorageError::Write(Box::new(e)))?
                .is_some();
        }
        txn.commit()
            .map_err(|e| StorageError::Commit(Box::new(e)))?;
        Ok(removed)
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
    Json(serde_json::Error),
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
            StorageError::Json(e) => write!(f, "storage json: {e}"),
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

    // ── Schedule state tests (issue #332) ────────────────────────────

    #[test]
    fn schedule_state_upsert_and_get_round_trip() {
        let (_dir, storage) = temp_storage();
        let state = ScheduleState {
            next_run_at_ms: 1_700_000_000_000,
            last_run_at_ms: Some(1_699_999_000_000),
            last_status: ScheduleStatus::Success,
            consecutive_errors: 0,
            claimed_by: None,
            claim_expires_at_ms: None,
        };
        storage
            .upsert_schedule_state("sensei", "mastery_review", &state)
            .unwrap();
        let loaded = storage
            .get_schedule_state("sensei", "mastery_review")
            .unwrap();
        assert_eq!(loaded.as_ref(), Some(&state));
    }

    #[test]
    fn schedule_state_get_missing_is_none() {
        let (_dir, storage) = temp_storage();
        assert!(storage
            .get_schedule_state("ghost", "absent")
            .unwrap()
            .is_none());
    }

    #[test]
    fn schedule_state_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("schedules.redb");
        let state = ScheduleState::fresh(42);
        {
            let storage = ForgeStorage::open(&db_path).unwrap();
            storage.upsert_schedule_state("a", "b", &state).unwrap();
        }
        let storage = ForgeStorage::open(&db_path).unwrap();
        assert_eq!(storage.get_schedule_state("a", "b").unwrap(), Some(state));
    }

    #[test]
    fn list_schedules_for_agent_returns_only_that_agent() {
        let (_dir, storage) = temp_storage();
        storage
            .upsert_schedule_state("a", "s1", &ScheduleState::fresh(1))
            .unwrap();
        storage
            .upsert_schedule_state("a", "s2", &ScheduleState::fresh(2))
            .unwrap();
        storage
            .upsert_schedule_state("b", "s1", &ScheduleState::fresh(3))
            .unwrap();

        let mut rows = storage.list_schedules_for_agent("a").unwrap();
        rows.sort_by(|x, y| x.0.cmp(&y.0));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "s1");
        assert_eq!(rows[1].0, "s2");
        assert_eq!(storage.list_schedules_for_agent("b").unwrap().len(), 1);
    }

    #[test]
    fn delete_schedule_returns_prior_existence() {
        let (_dir, storage) = temp_storage();
        storage
            .upsert_schedule_state("a", "s1", &ScheduleState::fresh(1))
            .unwrap();
        assert!(storage.delete_schedule("a", "s1").unwrap());
        assert!(!storage.delete_schedule("a", "s1").unwrap());
        assert!(storage.get_schedule_state("a", "s1").unwrap().is_none());
    }

    #[test]
    fn list_all_schedules_returns_every_agent() {
        let (_dir, storage) = temp_storage();
        storage
            .upsert_schedule_state("a", "s1", &ScheduleState::fresh(1))
            .unwrap();
        storage
            .upsert_schedule_state("a", "s2", &ScheduleState::fresh(2))
            .unwrap();
        storage
            .upsert_schedule_state("b", "s1", &ScheduleState::fresh(3))
            .unwrap();

        let rows = storage.list_all_schedules().unwrap();
        assert_eq!(rows.len(), 3);
        let mut keys: Vec<(String, String)> = rows
            .iter()
            .map(|(a, s, _)| (a.clone(), s.clone()))
            .collect();
        keys.sort();
        assert!(keys.contains(&("a".to_string(), "s1".to_string())));
        assert!(keys.contains(&("a".to_string(), "s2".to_string())));
        assert!(keys.contains(&("b".to_string(), "s1".to_string())));
    }

    #[test]
    fn list_all_correlations_groups_by_agent_field() {
        let (_dir, storage) = temp_storage();
        storage
            .upsert_correlation("sensei", "thread_id", "T1", "alias-1")
            .unwrap();
        storage
            .upsert_correlation("sensei", "thread_id", "T2", "alias-2")
            .unwrap();
        storage
            .upsert_correlation("sensei", "user_id", "U1", "alias-3")
            .unwrap();
        storage
            .upsert_correlation("bot", "thread_id", "T1", "alias-4")
            .unwrap();

        let mut rows = storage.list_all_correlations().unwrap();
        rows.sort();
        assert_eq!(rows.len(), 3);
        assert!(rows.contains(&("bot".to_string(), "thread_id".to_string(), 1)));
        assert!(rows.contains(&("sensei".to_string(), "thread_id".to_string(), 2)));
        assert!(rows.contains(&("sensei".to_string(), "user_id".to_string(), 1)));
    }

    #[test]
    fn try_claim_on_unregistered_schedule_reports_not_registered() {
        let (_dir, storage) = temp_storage();
        let outcome = storage
            .try_claim_schedule("a", "s1", "inst-1", 1_000, 5_000)
            .unwrap();
        assert_eq!(outcome, ClaimOutcome::NotRegistered);
    }

    #[test]
    fn try_claim_succeeds_on_fresh_row_and_loses_on_contention() {
        let (_dir, storage) = temp_storage();
        storage
            .upsert_schedule_state("a", "s1", &ScheduleState::fresh(0))
            .unwrap();

        let first = storage
            .try_claim_schedule("a", "s1", "inst-1", 1_000, 5_000)
            .unwrap();
        match first {
            ClaimOutcome::Claimed { state } => {
                assert_eq!(state.claimed_by.as_deref(), Some("inst-1"));
                assert_eq!(state.claim_expires_at_ms, Some(6_000));
            }
            other => panic!("expected Claimed, got {other:?}"),
        }

        let second = storage
            .try_claim_schedule("a", "s1", "inst-2", 1_500, 5_000)
            .unwrap();
        match second {
            ClaimOutcome::Lost {
                winner,
                claim_expires_at_ms,
            } => {
                assert_eq!(winner, "inst-1");
                assert_eq!(claim_expires_at_ms, 6_000);
            }
            other => panic!("expected Lost, got {other:?}"),
        }
    }

    #[test]
    fn try_claim_reclaims_expired_stale_claim() {
        let (_dir, storage) = temp_storage();
        let mut state = ScheduleState::fresh(0);
        state.claimed_by = Some("dead-inst".into());
        state.claim_expires_at_ms = Some(500);
        storage.upsert_schedule_state("a", "s1", &state).unwrap();

        // Current time is past the expiration — the old claim is dead.
        let outcome = storage
            .try_claim_schedule("a", "s1", "inst-new", 10_000, 5_000)
            .unwrap();
        match outcome {
            ClaimOutcome::Claimed { state } => {
                assert_eq!(state.claimed_by.as_deref(), Some("inst-new"));
                assert_eq!(state.claim_expires_at_ms, Some(15_000));
            }
            other => panic!("expected Claimed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn concurrent_claims_are_serialized_with_exactly_one_winner() {
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(ForgeStorage::open(&dir.path().join("race.redb")).unwrap());
        storage
            .upsert_schedule_state("a", "s1", &ScheduleState::fresh(0))
            .unwrap();

        // Spawn many tasks racing on the same schedule. redb's write-txn
        // serialization gives us exactly one Claimed outcome.
        let mut tasks = Vec::new();
        for i in 0..16 {
            let s = storage.clone();
            tasks.push(tokio::spawn(async move {
                s.try_claim_schedule("a", "s1", &format!("inst-{i}"), 1_000, 60_000)
                    .unwrap()
            }));
        }
        let mut claimed = 0;
        let mut lost = 0;
        for t in tasks {
            match t.await.unwrap() {
                ClaimOutcome::Claimed { .. } => claimed += 1,
                ClaimOutcome::Lost { .. } => lost += 1,
                ClaimOutcome::NotRegistered => panic!("pre-registered above"),
            }
        }
        assert_eq!(claimed, 1, "exactly one task must win the claim");
        assert_eq!(lost, 15);
    }

    #[test]
    fn is_claim_live_honours_expiration() {
        let mut s = ScheduleState::fresh(0);
        s.claimed_by = Some("x".into());
        s.claim_expires_at_ms = Some(100);
        assert!(s.is_claim_live(50));
        assert!(!s.is_claim_live(100));
        assert!(!s.is_claim_live(200));

        s.claimed_by = None;
        s.claim_expires_at_ms = Some(10_000);
        assert!(!s.is_claim_live(0), "no claimer → not live");
    }

    // ── Correlation state tests (issue #334) ─────────────────────────

    #[test]
    fn correlation_upsert_and_lookup_round_trip() {
        let (_dir, storage) = temp_storage();
        storage
            .upsert_correlation("slack_specialist", "thread_ts", "T1", "slack_specialist")
            .unwrap();
        let hit = storage
            .lookup_correlation("slack_specialist", "thread_ts", "T1")
            .unwrap();
        assert_eq!(hit, Some("slack_specialist".to_string()));
    }

    #[test]
    fn correlation_lookup_miss_returns_none() {
        let (_dir, storage) = temp_storage();
        assert!(storage
            .lookup_correlation("a", "thread_ts", "T-new")
            .unwrap()
            .is_none());
    }

    #[test]
    fn correlation_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("correlations.redb");
        {
            let storage = ForgeStorage::open(&db_path).unwrap();
            storage
                .upsert_correlation("a", "thread_ts", "T1", "a")
                .unwrap();
        }
        let storage = ForgeStorage::open(&db_path).unwrap();
        assert_eq!(
            storage.lookup_correlation("a", "thread_ts", "T1").unwrap(),
            Some("a".to_string())
        );
    }

    #[test]
    fn store_memory_with_correlations_commits_both_or_neither() {
        let (_dir, storage) = temp_storage();

        // Happy path: memory + correlation row land together.
        let rows = vec![(
            "slack_specialist".to_string(),
            "thread_ts".to_string(),
            "T1".to_string(),
            "slack_specialist".to_string(),
        )];
        storage
            .store_memory_with_correlations(
                "agent:slack_specialist:memory",
                r#"{"thread_ts":"T1"}"#,
                &rows,
            )
            .unwrap();

        assert_eq!(
            storage.get("agent:slack_specialist:memory").unwrap(),
            Some(r#"{"thread_ts":"T1"}"#.to_string())
        );
        assert_eq!(
            storage
                .lookup_correlation("slack_specialist", "thread_ts", "T1")
                .unwrap(),
            Some("slack_specialist".to_string())
        );

        // Empty-rows path writes memory with no correlation side-effects.
        storage
            .store_memory_with_correlations("agent:other:memory", r#"{"foo":"bar"}"#, &[])
            .unwrap();
        assert_eq!(
            storage.get("agent:other:memory").unwrap(),
            Some(r#"{"foo":"bar"}"#.to_string())
        );
        assert!(storage
            .lookup_correlation("other", "thread_ts", "T1")
            .unwrap()
            .is_none());
    }

    // ── Wake-secret state tests (issue #335) ─────────────────────────

    #[test]
    fn wake_secret_upsert_and_lookup_round_trip() {
        let (_dir, storage) = temp_storage();
        storage
            .upsert_wake_secret("mastermind", "pr_merged", "deadbeefcafe")
            .unwrap();
        assert_eq!(
            storage
                .lookup_wake_secret("mastermind", "pr_merged")
                .unwrap(),
            Some("deadbeefcafe".to_string())
        );
    }

    #[test]
    fn wake_secret_lookup_miss_returns_none() {
        let (_dir, storage) = temp_storage();
        assert!(storage
            .lookup_wake_secret("unknown", "pr_merged")
            .unwrap()
            .is_none());
    }

    #[test]
    fn wake_secret_upsert_replaces_prior_value() {
        let (_dir, storage) = temp_storage();
        storage.upsert_wake_secret("a", "t", "v1").unwrap();
        storage.upsert_wake_secret("a", "t", "v2").unwrap();
        assert_eq!(
            storage.lookup_wake_secret("a", "t").unwrap(),
            Some("v2".to_string())
        );
    }

    #[test]
    fn wake_secret_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("secrets.redb");
        {
            let storage = ForgeStorage::open(&db_path).unwrap();
            storage.upsert_wake_secret("a", "t", "s").unwrap();
        }
        let storage = ForgeStorage::open(&db_path).unwrap();
        assert_eq!(
            storage.lookup_wake_secret("a", "t").unwrap(),
            Some("s".to_string())
        );
    }

    #[test]
    fn wake_secret_list_returns_pairs_without_values() {
        let (_dir, storage) = temp_storage();
        storage.upsert_wake_secret("a", "t1", "secret-a1").unwrap();
        storage.upsert_wake_secret("a", "t2", "secret-a2").unwrap();
        storage.upsert_wake_secret("b", "t1", "secret-b1").unwrap();

        let mut pairs = storage.list_wake_triggers().unwrap();
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                ("a".to_string(), "t1".to_string()),
                ("a".to_string(), "t2".to_string()),
                ("b".to_string(), "t1".to_string()),
            ]
        );
        // list never returns secret bytes — nothing in the serialized form
        // can contain "secret-".
        let serialized = format!("{:?}", pairs);
        assert!(!serialized.contains("secret-"));
    }

    #[test]
    fn wake_secret_delete_reports_prior_existence() {
        let (_dir, storage) = temp_storage();
        storage.upsert_wake_secret("a", "t", "s").unwrap();
        assert!(storage.delete_wake_secret("a", "t").unwrap());
        assert!(!storage.delete_wake_secret("a", "t").unwrap());
        assert!(storage.lookup_wake_secret("a", "t").unwrap().is_none());
    }
}
