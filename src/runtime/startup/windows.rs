// Windows StartupManager via schtasks (scheduled tasks).

use std::process::Command;

use super::{quote_arg, ServiceConfig, ServiceStatus, StartupManager};

pub(crate) struct Schtasks;

impl Schtasks {
    pub fn new() -> Self {
        Self
    }
}

impl StartupManager for Schtasks {
    fn install(&self, cfg: &ServiceConfig) -> anyhow::Result<()> {
        let mut tr = quote_arg(&cfg.binary.display().to_string());
        for a in &cfg.args {
            tr.push(' ');
            tr.push_str(&quote_arg(a));
        }
        let out = Command::new("schtasks")
            .args([
                "/Create", "/F", "/SC", "ONLOGON", "/RL", "HIGHEST", "/TN", &cfg.label, "/TR", &tr,
            ])
            .output()?;
        if !out.status.success() {
            anyhow::bail!(
                "schtasks /Create failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }

    fn start(&self, label: &str) -> anyhow::Result<()> {
        let out = Command::new("schtasks")
            .args(["/Run", "/TN", label])
            .output()?;
        if !out.status.success() {
            anyhow::bail!(
                "schtasks /Run failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }

    fn stop(&self, label: &str) -> anyhow::Result<()> {
        let _ = Command::new("schtasks")
            .args(["/End", "/TN", label])
            .output();
        Ok(())
    }

    fn status(&self, label: &str) -> anyhow::Result<ServiceStatus> {
        let out = Command::new("schtasks")
            .args(["/Query", "/TN", label, "/FO", "CSV", "/NH"])
            .output()?;
        if !out.status.success() {
            return Ok(ServiceStatus::NotInstalled);
        }
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let fields: Vec<&str> = line.split(',').map(|f| f.trim_matches('"')).collect();
            if fields.len() >= 3 {
                return Ok(match fields[2] {
                    "Running" => ServiceStatus::Running { pid: None },
                    "Ready" | "Queued" | "Disabled" => ServiceStatus::Stopped,
                    other => ServiceStatus::Unknown(other.to_string()),
                });
            }
        }
        Ok(ServiceStatus::Unknown("no status row".into()))
    }

    fn uninstall(&self, label: &str) -> anyhow::Result<()> {
        let out = Command::new("schtasks")
            .args(["/Delete", "/F", "/TN", label])
            .output()?;
        if !out.status.success() {
            // Treat "task does not exist" as success.
            let err = String::from_utf8_lossy(&out.stderr);
            if err.contains("cannot find") || err.contains("does not exist") {
                return Ok(());
            }
            anyhow::bail!("schtasks /Delete failed: {}", err);
        }
        Ok(())
    }
}
