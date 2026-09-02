// #447 — no silent mock fallback. Config-level contract tests.
//
// Production entrypoints (run/serve/send/check) use
// `ForgeConfig::try_load_or_default()`: a missing config file with no
// explicit mock selection is a hard error, never a silent mock provider.
use forge::config::ForgeConfig;

/// Serialize env mutation across tests in this binary: config lookups touch
/// process-global state (env vars + cwd).
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct IsolatedEnv {
    _lock: std::sync::MutexGuard<'static, ()>,
    old_cwd: std::path::PathBuf,
}

impl IsolatedEnv {
    fn new() -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        // Clear every env knob the loader consults.
        for var in [
            "FORGE_CONFIG",
            "FORGE_APP_CONFIG",
            "FORGE_MOCK",
            "FORGE_PROVIDER",
        ] {
            std::env::remove_var(var);
        }
        // Never read the developer's ~/.forge/config.toml.
        std::env::set_var("HOME", tmp.path());
        IsolatedEnv {
            _lock: lock,
            old_cwd,
        }
    }
}

impl Drop for IsolatedEnv {
    fn drop(&mut self) {
        for var in [
            "FORGE_CONFIG",
            "FORGE_APP_CONFIG",
            "FORGE_MOCK",
            "FORGE_PROVIDER",
        ] {
            std::env::remove_var(var);
        }
        let _ = std::env::set_current_dir(&self.old_cwd);
    }
}

/// Missing config + no explicit mock ⇒ hard error mentioning mock and the way out.
#[test]
fn missing_config_without_explicit_mock_is_hard_error() {
    let _env = IsolatedEnv::new();
    let result = ForgeConfig::try_load_or_default();
    assert!(
        result.is_err(),
        "missing config without explicit mock must hard-error, got default={:?}",
        result.ok().map(|c| c.llm.default)
    );
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("mock"),
        "error must explain the mock escape hatch: {msg}"
    );
    assert!(
        msg.contains("FORGE_MOCK") || msg.contains("FORGE_PROVIDER"),
        "error must name the explicit-selection env vars: {msg}"
    );
}

/// FORGE_MOCK=1 is an explicit mock selection: fallback allowed.
#[test]
fn missing_config_with_forge_mock_1_falls_back_explicitly() {
    let _env = IsolatedEnv::new();
    std::env::set_var("FORGE_MOCK", "1");
    let result = ForgeConfig::try_load_or_default();
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(result.unwrap().llm.default, "mock");
}

/// FORGE_PROVIDER=mock is the other explicit selection path.
#[test]
fn missing_config_with_forge_provider_mock_falls_back_explicitly() {
    let _env = IsolatedEnv::new();
    std::env::set_var("FORGE_PROVIDER", "mock");
    let result = ForgeConfig::try_load_or_default();
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(result.unwrap().llm.default, "mock");
}

/// A pinned FORGE_CONFIG that does not exist is a hard error in production.
#[test]
fn pinned_forge_config_missing_is_hard_error() {
    let _env = IsolatedEnv::new();
    std::env::set_var("FORGE_CONFIG", "/definitely/not/a/real/config.toml");
    let result = ForgeConfig::try_load_or_default();
    assert!(
        result.is_err(),
        "pinned-but-missing FORGE_CONFIG must hard-error; silent mock would mask a broken launch"
    );
}

/// A pinned config that exists but fails validation is surfaced, not swallowed.
#[test]
fn pinned_forge_config_broken_is_hard_error() {
    let _env = IsolatedEnv::new();
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bad.config.toml");
    std::fs::write(
        &path,
        "[llm]\ndefault = \"no-such-provider\"\n\n[providers.mock]\ntype = \"mock\"\n",
    )
    .unwrap();
    std::env::set_var("FORGE_CONFIG", path.display().to_string());
    let result = ForgeConfig::try_load_or_default();
    assert!(
        result.is_err(),
        "broken config must hard-error (bad default provider)"
    );
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("no-such-provider"),
        "error must name the offending provider: {msg}"
    );
}

/// A real config file loads normally — mock never involved.
#[test]
fn valid_config_loads_normally() {
    let _env = IsolatedEnv::new();
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("forge.config.toml");
    std::fs::write(
        &path,
        "[llm]\ndefault = \"mock\"\n\n[providers.mock]\ntype = \"mock\"\n",
    )
    .unwrap();
    std::env::set_var("FORGE_CONFIG", path.display().to_string());
    let result = ForgeConfig::try_load_or_default();
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(result.unwrap().llm.default, "mock");
}
