// Linux StartupManager via systemd user units.

use std::process::Command;

use super::{
    ensure_parent, render_unit, systemd_unit_path, ServiceConfig, ServiceStatus, StartupManager,
};

pub(crate) struct Systemd;

impl Systemd {
    pub fn new() -> Self {
        Self
    }

    fn preflight() -> anyhow::Result<()> {
        // systemctl must exist; `systemctl --user` needs a user bus, which
        // typically requires $XDG_RUNTIME_DIR to be set.
        if which("systemctl").is_none() {
            anyhow::bail!(
                "systemctl not found. systemd is required for service management on Linux"
            );
        }
        if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
            anyhow::bail!(
                "XDG_RUNTIME_DIR is not set. systemd user session unavailable (common on WSL without systemd)"
            );
        }
        Ok(())
    }
}

impl StartupManager for Systemd {
    fn install(&self, cfg: &ServiceConfig) -> anyhow::Result<()> {
        Self::preflight()?;
        let unit_path = systemd_unit_path(&cfg.label)?;
        ensure_parent(&unit_path)?;
        std::fs::write(&unit_path, render_unit(cfg))?;

        run_systemctl(["daemon-reload"])?;
        run_systemctl(["enable", "--now", &format!("{}.service", cfg.label)])?;
        Ok(())
    }

    fn start(&self, label: &str) -> anyhow::Result<()> {
        Self::preflight()?;
        run_systemctl(["start", &format!("{}.service", label)])
    }

    fn stop(&self, label: &str) -> anyhow::Result<()> {
        Self::preflight()?;
        run_systemctl(["stop", &format!("{}.service", label)])
    }

    fn status(&self, label: &str) -> anyhow::Result<ServiceStatus> {
        Self::preflight()?;
        let unit_path = systemd_unit_path(label)?;
        if !unit_path.exists() {
            return Ok(ServiceStatus::NotInstalled);
        }
        let out = Command::new("systemctl")
            .args(["--user", "is-active", &format!("{}.service", label)])
            .output()?;
        let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
        match state.as_str() {
            "active" | "activating" => {
                let pid_out = Command::new("systemctl")
                    .args([
                        "--user",
                        "show",
                        "-p",
                        "MainPID",
                        "--value",
                        &format!("{}.service", label),
                    ])
                    .output()?;
                let pid = String::from_utf8_lossy(&pid_out.stdout)
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|p| *p != 0);
                Ok(ServiceStatus::Running { pid })
            }
            "inactive" | "failed" | "deactivating" => Ok(ServiceStatus::Stopped),
            other if other.is_empty() => Ok(ServiceStatus::Stopped),
            other => Ok(ServiceStatus::Unknown(other.to_string())),
        }
    }

    fn uninstall(&self, label: &str) -> anyhow::Result<()> {
        Self::preflight()?;
        let unit_file = format!("{}.service", label);
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "--now", &unit_file])
            .output();
        let unit_path = systemd_unit_path(label)?;
        if unit_path.exists() {
            std::fs::remove_file(&unit_path)?;
        }
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();
        Ok(())
    }
}

fn run_systemctl<I, S>(args: I) -> anyhow::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user");
    for a in args {
        cmd.arg(a);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        anyhow::bail!(
            "systemctl --user failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

fn which(bin: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(bin);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
