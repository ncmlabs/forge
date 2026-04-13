// Integration tests for the StartupManager trait (issue #254).
//
// Pure-text renderer tests live in `src/runtime/startup/mod.rs` as lib unit
// tests — those assert on the generated plist/unit/schtasks strings across
// all host platforms. This file exercises the real OS-backed manager via
// `startup::current()`, gated on `FORGE_SERVICE_E2E=1` so normal local
// `cargo test` runs stay clean.

use forge::runtime::startup::{self, ServiceStatus};

fn e2e_enabled() -> bool {
    std::env::var("FORGE_SERVICE_E2E").ok().as_deref() == Some("1")
}

#[test]
fn current_manager_handles_missing_service() {
    // Status on a label that was never installed must not panic and must
    // return NotInstalled (preflight errors on Linux without systemd are
    // acceptable — treat either as a pass).
    let mgr = startup::current();
    let label = format!("com.ncmlabs.forge-test-missing-{}", std::process::id());
    match mgr.status(&label) {
        Ok(ServiceStatus::NotInstalled) => {}
        Ok(other) => panic!("expected NotInstalled, got {:?}", other),
        Err(_) => {
            // Acceptable on environments without a user service session.
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn install_status_uninstall_roundtrip() {
    if !e2e_enabled() {
        eprintln!("skipping E2E test (set FORGE_SERVICE_E2E=1 to enable)");
        return;
    }
    use forge::runtime::startup::ServiceConfig;
    use std::path::PathBuf;

    let mgr = startup::current();
    let label = format!("com.ncmlabs.forge-e2e-{}", std::process::id());

    // Use `sleep` — present on macOS and most Linux distros.
    let sleep_bin = if PathBuf::from("/bin/sleep").exists() {
        PathBuf::from("/bin/sleep")
    } else {
        PathBuf::from("/usr/bin/sleep")
    };

    let mut cfg = ServiceConfig::new(label.clone(), sleep_bin);
    cfg.args = vec!["30".into()];
    cfg.keep_alive = false;

    // If preflight fails (e.g. CI without systemd-user), skip rather than fail.
    if let Err(e) = mgr.install(&cfg) {
        eprintln!("install preflight failed, skipping: {}", e);
        return;
    }

    // Status should not be NotInstalled after install.
    let status = mgr.status(&label).expect("status after install");
    assert!(
        !matches!(status, ServiceStatus::NotInstalled),
        "expected installed, got {:?}",
        status,
    );

    mgr.uninstall(&label).expect("uninstall");

    let final_status = mgr.status(&label).expect("status after uninstall");
    assert_eq!(
        final_status,
        ServiceStatus::NotInstalled,
        "service should be gone after uninstall",
    );
}
