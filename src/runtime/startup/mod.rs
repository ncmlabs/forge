// StartupManager — cross-platform service installation (issue #254).
//
// Abstracts launchctl (macOS), systemd user units (Linux/WSL), and
// schtasks (Windows) behind a single trait so generated FORGE server
// binaries can install themselves as long-running services uniformly.

use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

/// Input to `StartupManager::install`.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    /// Reverse-DNS identifier, e.g. `com.ncmlabs.forge-sensei`.
    pub label: String,
    pub binary: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub working_dir: Option<PathBuf>,
    pub stdout_log: Option<PathBuf>,
    pub stderr_log: Option<PathBuf>,
    /// Restart on exit (LaunchAgent `KeepAlive`, systemd `Restart=always`).
    pub keep_alive: bool,
}

impl ServiceConfig {
    pub fn new(label: impl Into<String>, binary: impl Into<PathBuf>) -> Self {
        Self {
            label: label.into(),
            binary: binary.into(),
            args: Vec::new(),
            env: Vec::new(),
            working_dir: None,
            stdout_log: None,
            stderr_log: None,
            keep_alive: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStatus {
    Running { pid: Option<u32> },
    Stopped,
    NotInstalled,
    Unknown(String),
}

impl ServiceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServiceStatus::Running { .. } => "running",
            ServiceStatus::Stopped => "stopped",
            ServiceStatus::NotInstalled => "not_installed",
            ServiceStatus::Unknown(_) => "unknown",
        }
    }
}

pub trait StartupManager {
    fn install(&self, cfg: &ServiceConfig) -> anyhow::Result<()>;
    fn start(&self, label: &str) -> anyhow::Result<()>;
    fn stop(&self, label: &str) -> anyhow::Result<()>;
    fn status(&self, label: &str) -> anyhow::Result<ServiceStatus>;
    fn uninstall(&self, label: &str) -> anyhow::Result<()>;
}

/// Returns the service manager for the current platform.
pub fn current() -> Box<dyn StartupManager> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::Launchctl::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::Systemd::new())
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::Schtasks::new())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Box::new(Unsupported)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
struct Unsupported;

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
impl StartupManager for Unsupported {
    fn install(&self, _: &ServiceConfig) -> anyhow::Result<()> {
        anyhow::bail!("StartupManager not supported on this platform")
    }
    fn start(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("StartupManager not supported on this platform")
    }
    fn stop(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("StartupManager not supported on this platform")
    }
    fn status(&self, _: &str) -> anyhow::Result<ServiceStatus> {
        anyhow::bail!("StartupManager not supported on this platform")
    }
    fn uninstall(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("StartupManager not supported on this platform")
    }
}

/// Resolve the home directory or return a clear error.
pub(crate) fn home_dir() -> anyhow::Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))
}

/// XML-escape text for plist bodies.
pub(crate) fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Platform-agnostic helper: format a command-and-args string for display.
#[allow(dead_code)]
pub(crate) fn quote_arg(s: &str) -> String {
    if s.is_empty() || s.chars().any(|c| c.is_whitespace() || c == '"') {
        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{}\"", escaped)
    } else {
        s.to_string()
    }
}

// Pure text renderers — implemented here (not in per-OS modules) so they
// compile and can be unit-tested on every host. The per-OS modules use
// these to produce the files they drop on disk.

#[allow(dead_code)]
pub(crate) fn render_plist(cfg: &ServiceConfig) -> String {
    use std::fmt::Write;
    let mut prog_args = String::new();
    writeln!(
        prog_args,
        "    <string>{}</string>",
        xml_escape(&cfg.binary.display().to_string())
    )
    .unwrap();
    for arg in &cfg.args {
        writeln!(prog_args, "    <string>{}</string>", xml_escape(arg)).unwrap();
    }

    let mut env_block = String::new();
    if !cfg.env.is_empty() {
        env_block.push_str("  <key>EnvironmentVariables</key>\n  <dict>\n");
        for (k, v) in &cfg.env {
            writeln!(
                env_block,
                "    <key>{}</key>\n    <string>{}</string>",
                xml_escape(k),
                xml_escape(v)
            )
            .unwrap();
        }
        env_block.push_str("  </dict>\n");
    }

    let wd_block = cfg
        .working_dir
        .as_ref()
        .map(|p| {
            format!(
                "  <key>WorkingDirectory</key>\n  <string>{}</string>\n",
                xml_escape(&p.display().to_string())
            )
        })
        .unwrap_or_default();

    let stdout_block = cfg
        .stdout_log
        .as_ref()
        .map(|p| {
            format!(
                "  <key>StandardOutPath</key>\n  <string>{}</string>\n",
                xml_escape(&p.display().to_string())
            )
        })
        .unwrap_or_default();

    let stderr_block = cfg
        .stderr_log
        .as_ref()
        .map(|p| {
            format!(
                "  <key>StandardErrorPath</key>\n  <string>{}</string>\n",
                xml_escape(&p.display().to_string())
            )
        })
        .unwrap_or_default();

    let keep_alive = if cfg.keep_alive {
        "<true/>"
    } else {
        "<false/>"
    };

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
{prog_args}  </array>
{env_block}{wd_block}  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  {keep_alive}
{stdout_block}{stderr_block}</dict>
</plist>
"#,
        label = xml_escape(&cfg.label),
        prog_args = prog_args,
        env_block = env_block,
        wd_block = wd_block,
        stdout_block = stdout_block,
        stderr_block = stderr_block,
        keep_alive = keep_alive,
    )
}

#[allow(dead_code)]
pub(crate) fn render_unit(cfg: &ServiceConfig) -> String {
    let mut exec = quote_arg(&cfg.binary.display().to_string());
    for a in &cfg.args {
        exec.push(' ');
        exec.push_str(&quote_arg(a));
    }

    let mut body = String::new();
    body.push_str("[Unit]\n");
    body.push_str(&format!("Description=FORGE service: {}\n", cfg.label));
    body.push_str("After=network.target\n\n");

    body.push_str("[Service]\n");
    body.push_str(&format!("ExecStart={}\n", exec));
    if let Some(wd) = &cfg.working_dir {
        body.push_str(&format!("WorkingDirectory={}\n", wd.display()));
    }
    for (k, v) in &cfg.env {
        body.push_str(&format!(
            "Environment=\"{}={}\"\n",
            k,
            v.replace('"', "\\\"")
        ));
    }
    if let Some(p) = &cfg.stdout_log {
        body.push_str(&format!("StandardOutput=append:{}\n", p.display()));
    }
    if let Some(p) = &cfg.stderr_log {
        body.push_str(&format!("StandardError=append:{}\n", p.display()));
    }
    if cfg.keep_alive {
        body.push_str("Restart=always\nRestartSec=3\n");
    }
    body.push_str("\n[Install]\nWantedBy=default.target\n");
    body
}

#[allow(dead_code)]
pub(crate) fn render_schtasks_cmd(cfg: &ServiceConfig) -> String {
    let mut tr = quote_arg(&cfg.binary.display().to_string());
    for a in &cfg.args {
        tr.push(' ');
        tr.push_str(&quote_arg(a));
    }
    format!(
        "schtasks /Create /F /SC ONLOGON /RL HIGHEST /TN {} /TR {}",
        quote_arg(&cfg.label),
        quote_arg(&tr)
    )
}

/// Default user-unit path for systemd (Linux).
#[allow(dead_code)]
pub(crate) fn systemd_unit_path(label: &str) -> anyhow::Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| home_dir().map(|h| h.join(".config")))?;
    Ok(base.join("systemd/user").join(format!("{}.service", label)))
}

/// Default plist path for launchctl (macOS).
#[allow(dead_code)]
pub(crate) fn launchagent_plist_path(label: &str) -> anyhow::Result<PathBuf> {
    Ok(home_dir()?
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", label)))
}

#[allow(dead_code)]
pub(crate) fn ensure_parent(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ServiceConfig {
        ServiceConfig {
            label: "com.ncmlabs.forge-sensei".into(),
            binary: PathBuf::from("/usr/local/bin/forge-sensei-server"),
            args: vec![
                "--host".into(),
                "127.0.0.1".into(),
                "--port".into(),
                "3000".into(),
            ],
            env: vec![("FORGE_CONFIG".into(), "/tmp/config.toml".into())],
            working_dir: Some(PathBuf::from("/tmp/wd")),
            stdout_log: Some(PathBuf::from("/tmp/out.log")),
            stderr_log: Some(PathBuf::from("/tmp/err.log")),
            keep_alive: true,
        }
    }

    #[test]
    fn plist_contains_label_and_args() {
        let s = render_plist(&sample());
        assert!(s.contains("<string>com.ncmlabs.forge-sensei</string>"));
        assert!(s.contains("<string>/usr/local/bin/forge-sensei-server</string>"));
        assert!(s.contains("<string>--host</string>"));
        assert!(s.contains("<string>127.0.0.1</string>"));
        assert!(s.contains("<key>FORGE_CONFIG</key>"));
        assert!(s.contains("<key>WorkingDirectory</key>"));
        assert!(s.contains("<key>StandardOutPath</key>"));
        assert!(s.contains("<key>StandardErrorPath</key>"));
        assert!(s.contains("<key>KeepAlive</key>\n  <true/>"));
    }

    #[test]
    fn plist_without_optional_blocks() {
        let mut cfg = sample();
        cfg.env.clear();
        cfg.working_dir = None;
        cfg.stdout_log = None;
        cfg.stderr_log = None;
        cfg.keep_alive = false;
        let s = render_plist(&cfg);
        assert!(!s.contains("EnvironmentVariables"));
        assert!(!s.contains("WorkingDirectory"));
        assert!(!s.contains("StandardOutPath"));
        assert!(s.contains("<key>KeepAlive</key>\n  <false/>"));
    }

    #[test]
    fn unit_file_has_execstart_and_restart() {
        let s = render_unit(&sample());
        assert!(s.contains("[Unit]"));
        assert!(s.contains("[Service]"));
        assert!(
            s.contains("ExecStart=/usr/local/bin/forge-sensei-server --host 127.0.0.1 --port 3000")
        );
        assert!(s.contains("Environment=\"FORGE_CONFIG=/tmp/config.toml\""));
        assert!(s.contains("WorkingDirectory=/tmp/wd"));
        assert!(s.contains("StandardOutput=append:/tmp/out.log"));
        assert!(s.contains("StandardError=append:/tmp/err.log"));
        assert!(s.contains("Restart=always"));
        assert!(s.contains("WantedBy=default.target"));
    }

    #[test]
    fn unit_file_no_restart_when_keepalive_false() {
        let mut cfg = sample();
        cfg.keep_alive = false;
        let s = render_unit(&cfg);
        assert!(!s.contains("Restart=always"));
    }

    #[test]
    fn schtasks_command_quotes_args() {
        let s = render_schtasks_cmd(&sample());
        assert!(s.starts_with("schtasks /Create /F /SC ONLOGON /RL HIGHEST"));
        assert!(s.contains("/TN com.ncmlabs.forge-sensei"));
        assert!(s.contains("/usr/local/bin/forge-sensei-server"));
    }

    #[test]
    fn xml_escape_handles_special_chars() {
        assert_eq!(xml_escape("a<b&c>\"d'e"), "a&lt;b&amp;c&gt;&quot;d&apos;e");
    }

    #[test]
    fn service_status_strings() {
        assert_eq!(ServiceStatus::Running { pid: Some(42) }.as_str(), "running");
        assert_eq!(ServiceStatus::Stopped.as_str(), "stopped");
        assert_eq!(ServiceStatus::NotInstalled.as_str(), "not_installed");
        assert_eq!(ServiceStatus::Unknown("x".into()).as_str(), "unknown");
    }
}
