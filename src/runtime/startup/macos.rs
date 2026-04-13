// macOS StartupManager via launchctl (user domain).

use std::process::Command;

use super::{
    ensure_parent, launchagent_plist_path, render_plist, ServiceConfig, ServiceStatus,
    StartupManager,
};

pub(crate) struct Launchctl;

impl Launchctl {
    pub fn new() -> Self {
        Self
    }

    fn domain() -> String {
        // Use `gui/$UID` domain so plist is loaded into the user's GUI
        // session, matching scripts/install-sensei-server.sh.
        let uid = unsafe { libc_getuid() };
        format!("gui/{}", uid)
    }

    fn target(label: &str) -> String {
        format!("{}/{}", Self::domain(), label)
    }
}

impl StartupManager for Launchctl {
    fn install(&self, cfg: &ServiceConfig) -> anyhow::Result<()> {
        let plist_path = launchagent_plist_path(&cfg.label)?;
        ensure_parent(&plist_path)?;
        std::fs::write(&plist_path, render_plist(cfg))?;

        // Best-effort unload in case of stale registration.
        let _ = Command::new("launchctl")
            .args(["bootout", &Self::target(&cfg.label)])
            .output();

        let out = Command::new("launchctl")
            .args([
                "bootstrap",
                &Self::domain(),
                &plist_path.display().to_string(),
            ])
            .output()?;
        if !out.status.success() {
            anyhow::bail!(
                "launchctl bootstrap failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let _ = Command::new("launchctl")
            .args(["kickstart", "-k", &Self::target(&cfg.label)])
            .output();
        Ok(())
    }

    fn start(&self, label: &str) -> anyhow::Result<()> {
        let plist_path = launchagent_plist_path(label)?;
        // Bootstrap is a no-op if already loaded.
        let _ = Command::new("launchctl")
            .args([
                "bootstrap",
                &Self::domain(),
                &plist_path.display().to_string(),
            ])
            .output();
        let out = Command::new("launchctl")
            .args(["kickstart", "-k", &Self::target(label)])
            .output()?;
        if !out.status.success() {
            anyhow::bail!(
                "launchctl kickstart failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }

    fn stop(&self, label: &str) -> anyhow::Result<()> {
        let _ = Command::new("launchctl")
            .args(["bootout", &Self::target(label)])
            .output();
        Ok(())
    }

    fn status(&self, label: &str) -> anyhow::Result<ServiceStatus> {
        let plist_path = launchagent_plist_path(label)?;
        if !plist_path.exists() {
            return Ok(ServiceStatus::NotInstalled);
        }
        let out = Command::new("launchctl")
            .args(["print", &Self::target(label)])
            .output()?;
        if !out.status.success() {
            return Ok(ServiceStatus::Stopped);
        }
        let text = String::from_utf8_lossy(&out.stdout);
        // `launchctl print` emits `pid = <n>` when running.
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("pid = ") {
                if let Ok(pid) = rest.trim().parse::<u32>() {
                    return Ok(ServiceStatus::Running { pid: Some(pid) });
                }
            }
        }
        // Registered but not running.
        Ok(ServiceStatus::Stopped)
    }

    fn uninstall(&self, label: &str) -> anyhow::Result<()> {
        let _ = Command::new("launchctl")
            .args(["bootout", &Self::target(label)])
            .output();
        let plist_path = launchagent_plist_path(label)?;
        if plist_path.exists() {
            std::fs::remove_file(&plist_path)?;
        }
        Ok(())
    }
}

// Avoid adding a `libc` dep for a single call.
extern "C" {
    fn getuid() -> u32;
}
#[inline]
unsafe fn libc_getuid() -> u32 {
    unsafe { getuid() }
}
