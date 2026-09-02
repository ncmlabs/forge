// Integration tests for unified storage root resolution (issue #253).
//
// Validates precedence:
//   1. FORGE_STORAGE_ROOT env var
//   2. [storage] root in config
//   3. knowledge.store_path (backward compat)
//   4. ./.forge-data (legacy default)

use forge::config::StorageConfig;
use forge::runtime::storage::ForgeStorage;
use std::sync::Mutex;

// Env-var tests share a global lock so they don't stomp on each other when
// cargo runs them in parallel.
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard;
impl EnvGuard {
    fn set(val: &str) -> Self {
        std::env::set_var("FORGE_STORAGE_ROOT", val);
        Self
    }
    fn unset() -> Self {
        std::env::remove_var("FORGE_STORAGE_ROOT");
        Self
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        std::env::remove_var("FORGE_STORAGE_ROOT");
    }
}

#[test]
fn env_var_wins_over_config_and_knowledge() {
    let _lock = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let env_path = tmp.path().join("from_env");
    let _guard = EnvGuard::set(env_path.to_str().unwrap());

    let cfg = StorageConfig {
        root: Some("/not/used".to_string()),
    };
    let root = ForgeStorage::resolve_root(Some(&cfg), Some("/also/not/used"));
    assert_eq!(root, env_path);
}

#[test]
fn config_wins_over_knowledge_store_path() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::unset();

    let cfg = StorageConfig {
        root: Some("/tmp/forge-config-root".to_string()),
    };
    let root = ForgeStorage::resolve_root(Some(&cfg), Some("/tmp/knowledge-root"));
    assert_eq!(root, std::path::PathBuf::from("/tmp/forge-config-root"));
}

#[test]
fn knowledge_store_path_used_when_no_config() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::unset();

    let root = ForgeStorage::resolve_root(None, Some("/tmp/knowledge-fallback"));
    assert_eq!(root, std::path::PathBuf::from("/tmp/knowledge-fallback"));
}

#[test]
fn legacy_default_when_nothing_configured() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::unset();

    let root = ForgeStorage::resolve_root(None, None);
    assert_eq!(root, std::path::PathBuf::from(".forge-data"));
}

#[test]
fn tilde_expansion_in_config_root() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::unset();

    let cfg = StorageConfig {
        root: Some("~/forge-test-home".to_string()),
    };
    let root = ForgeStorage::resolve_root(Some(&cfg), None);
    let expected = dirs::home_dir().unwrap().join("forge-test-home");
    assert_eq!(root, expected);
}

#[test]
fn env_var_expansion_in_config_root() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::unset();
    std::env::set_var("FORGE_TEST_STORAGE_BASE", "/tmp/envbase");

    let cfg = StorageConfig {
        root: Some("${FORGE_TEST_STORAGE_BASE}/sub".to_string()),
    };
    let root = ForgeStorage::resolve_root(Some(&cfg), None);
    assert_eq!(root, std::path::PathBuf::from("/tmp/envbase/sub"));

    std::env::remove_var("FORGE_TEST_STORAGE_BASE");
}

#[test]
fn cli_and_server_share_same_redb_under_unified_root() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::unset();

    let tmp = tempfile::tempdir().unwrap();
    let cfg = StorageConfig {
        root: Some(tmp.path().to_str().unwrap().to_string()),
    };

    // Simulate CLI opening store.redb under the configured root.
    let cli_storage =
        ForgeStorage::open_from_config(Some(&cfg), None, "store.redb").expect("cli open");
    cli_storage
        .store("shared-key", "shared-value")
        .expect("write");
    drop(cli_storage);

    // Re-open from the same config (simulating the server path) — must see the write.
    let server_storage =
        ForgeStorage::open_from_config(Some(&cfg), None, "store.redb").expect("server open");
    assert_eq!(
        server_storage.get("shared-key").unwrap(),
        Some("shared-value".to_string())
    );
}

#[test]
fn wake_cli_and_serve_share_wake_secret_store() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::unset();

    let tmp = tempfile::tempdir().unwrap();
    let cfg = StorageConfig {
        root: Some(tmp.path().to_str().unwrap().to_string()),
    };

    let cli_storage = ForgeStorage::open_wake_from_config(Some(&cfg), None).expect("cli wake open");
    cli_storage
        .upsert_wake_secret("mastermind", "github_issue_opened", "secret")
        .expect("register wake secret");
    drop(cli_storage);

    let serve_storage =
        ForgeStorage::open_wake_from_config(Some(&cfg), None).expect("serve wake open");
    assert_eq!(
        serve_storage
            .lookup_wake_secret("mastermind", "github_issue_opened")
            .unwrap(),
        Some("secret".to_string())
    );

    let runtime_storage =
        ForgeStorage::open_from_config(Some(&cfg), None, "server.redb").expect("server data open");
    assert_eq!(
        runtime_storage
            .lookup_wake_secret("mastermind", "github_issue_opened")
            .unwrap(),
        None,
        "wake secrets must not depend on the per-server runtime database"
    );
}

#[test]
fn empty_env_var_falls_through_to_config() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::set("");

    let cfg = StorageConfig {
        root: Some("/tmp/from-config-when-env-empty".to_string()),
    };
    let root = ForgeStorage::resolve_root(Some(&cfg), None);
    assert_eq!(
        root,
        std::path::PathBuf::from("/tmp/from-config-when-env-empty")
    );
}
