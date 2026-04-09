// FORGE command manager — background process lifecycle (issue #162)
//
// Manages background command processes with UUID-based handles.
// Processes are spawned with piped stdout/stderr; reader tasks accumulate
// output lines into shared buffers that agents can poll via command.output().

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::runtime::confidence::{ConfidentValue, Value};

// ── Types ────────────────────────────────────────────────────────────────────

pub type HandleId = String;
pub type SharedCommandManager = Arc<Mutex<CommandManager>>;

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessStatus {
    Running,
    Completed { exit_code: i32, success: bool },
    Cancelled,
    TimedOut,
}

impl ProcessStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProcessStatus::Running => "running",
            ProcessStatus::Completed { .. } => "completed",
            ProcessStatus::Cancelled => "cancelled",
            ProcessStatus::TimedOut => "timed_out",
        }
    }
}

/// Shared interior state between the manager and background reader tasks.
pub struct ProcessState {
    pub id: HandleId,
    pub status: ProcessStatus,
    pub stdout_buf: Vec<String>,
    pub stderr_buf: Vec<String>,
    pub cmd_display: String,
    pub started_at: Instant,
}

// ── CommandManager ───────────────────────────────────────────────────────────

pub struct CommandManager {
    processes: HashMap<HandleId, Arc<Mutex<ProcessState>>>,
    /// Senders to request kill from the watcher task.
    kill_senders: HashMap<HandleId, oneshot::Sender<()>>,
}

impl Default for CommandManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandManager {
    pub fn new() -> Self {
        Self {
            processes: HashMap::new(),
            kill_senders: HashMap::new(),
        }
    }

    /// Spawn a background process. Returns a UUID handle immediately.
    ///
    /// The child must have stdout/stderr set to `Stdio::piped()` before calling.
    /// Reader tasks buffer output lines; a watcher task collects exit status.
    pub fn spawn_background(
        &mut self,
        mut child: Child,
        cmd_display: String,
        timeout: Option<Duration>,
    ) -> Result<HandleId, String> {
        let handle_id = Uuid::new_v4().to_string();

        // Take piped streams from the child
        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| "child stdout not piped".to_string())?;
        let child_stderr = child
            .stderr
            .take()
            .ok_or_else(|| "child stderr not piped".to_string())?;

        let state = Arc::new(Mutex::new(ProcessState {
            id: handle_id.clone(),
            status: ProcessStatus::Running,
            stdout_buf: Vec::new(),
            stderr_buf: Vec::new(),
            cmd_display: cmd_display.clone(),
            started_at: Instant::now(),
        }));

        // Kill channel: cancel sends (), watcher receives and kills child
        let (kill_tx, kill_rx) = oneshot::channel::<()>();

        self.processes.insert(handle_id.clone(), Arc::clone(&state));
        self.kill_senders.insert(handle_id.clone(), kill_tx);

        // Spawn stdout reader task
        let state_for_stdout = Arc::clone(&state);
        tokio::spawn(async move {
            let reader = BufReader::new(child_stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                state_for_stdout.lock().unwrap().stdout_buf.push(line);
            }
        });

        // Spawn stderr reader task
        let state_for_stderr = Arc::clone(&state);
        tokio::spawn(async move {
            let reader = BufReader::new(child_stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                state_for_stderr.lock().unwrap().stderr_buf.push(line);
            }
        });

        // Spawn watcher task — handles: natural exit, cancel, and timeout
        let state_for_wait = Arc::clone(&state);
        let timeout_dur = timeout;
        tokio::spawn(async move {
            // Create a future that sleeps for the timeout duration, or never
            // completes if no timeout is set.
            let timeout_fut = async {
                match timeout_dur {
                    Some(dur) => tokio::time::sleep(dur).await,
                    None => std::future::pending::<()>().await,
                }
            };

            tokio::select! {
                exit = child.wait() => {
                    let mut s = state_for_wait.lock().unwrap();
                    if s.status == ProcessStatus::Running {
                        match exit {
                            Ok(status) => {
                                s.status = ProcessStatus::Completed {
                                    exit_code: status.code().unwrap_or(-1),
                                    success: status.success(),
                                };
                            }
                            Err(_) => {
                                s.status = ProcessStatus::Completed {
                                    exit_code: -1,
                                    success: false,
                                };
                            }
                        }
                    }
                }
                _ = kill_rx => {
                    // Cancel requested
                    {
                        let mut s = state_for_wait.lock().unwrap();
                        if s.status != ProcessStatus::Running {
                            return;
                        }
                        s.status = ProcessStatus::Cancelled;
                    }
                    let _ = child.kill().await;
                }
                _ = timeout_fut => {
                    // Timeout expired
                    {
                        let mut s = state_for_wait.lock().unwrap();
                        if s.status != ProcessStatus::Running {
                            return;
                        }
                        s.status = ProcessStatus::TimedOut;
                    }
                    let _ = child.kill().await;
                }
            }
        });

        Ok(handle_id)
    }

    /// Get the status of a background process as a FORGE Record.
    pub fn status(&self, handle: &str) -> Result<ConfidentValue, String> {
        let state = self
            .processes
            .get(handle)
            .ok_or_else(|| format!("unknown command handle: {}", handle))?;
        let s = state.lock().unwrap();

        let mut fields = HashMap::new();
        fields.insert(
            "status".to_string(),
            ConfidentValue::from_exec(Value::Text(s.status.as_str().to_string()), 0.9),
        );

        match &s.status {
            ProcessStatus::Completed { exit_code, success } => {
                fields.insert(
                    "exit_code".to_string(),
                    ConfidentValue::from_exec(Value::Number(*exit_code as f64), 0.9),
                );
                fields.insert(
                    "success".to_string(),
                    ConfidentValue::from_exec(Value::Bool(*success), 0.9),
                );
            }
            _ => {
                fields.insert(
                    "exit_code".to_string(),
                    ConfidentValue::deterministic(Value::Unit),
                );
                fields.insert(
                    "success".to_string(),
                    ConfidentValue::deterministic(Value::Unit),
                );
            }
        }

        Ok(ConfidentValue::from_exec(Value::Record(fields), 0.9))
    }

    /// Get buffered output from a background process as a FORGE Record.
    pub fn output(&self, handle: &str) -> Result<ConfidentValue, String> {
        let state = self
            .processes
            .get(handle)
            .ok_or_else(|| format!("unknown command handle: {}", handle))?;
        let s = state.lock().unwrap();

        let complete = s.status != ProcessStatus::Running;
        let confidence = if complete { 0.9 } else { 0.5 };

        let mut fields = HashMap::new();
        fields.insert(
            "stdout".to_string(),
            ConfidentValue::from_exec(Value::Text(s.stdout_buf.join("\n")), confidence),
        );
        fields.insert(
            "stderr".to_string(),
            ConfidentValue::from_exec(Value::Text(s.stderr_buf.join("\n")), confidence),
        );
        fields.insert(
            "complete".to_string(),
            ConfidentValue::from_exec(Value::Bool(complete), 0.9),
        );

        Ok(ConfidentValue::from_exec(Value::Record(fields), confidence))
    }

    /// Cancel a background process. Sends kill signal via the oneshot channel.
    pub fn cancel(&mut self, handle: &str) -> Result<(), String> {
        let state = self
            .processes
            .get(handle)
            .ok_or_else(|| format!("unknown command handle: {}", handle))?;
        {
            let s = state.lock().unwrap();
            if s.status != ProcessStatus::Running {
                return Ok(());
            }
        }

        if let Some(tx) = self.kill_senders.remove(handle) {
            let _ = tx.send(());
        }

        Ok(())
    }

    /// Shut down all running background processes.
    pub fn shutdown_all(&mut self) {
        let running_handles: Vec<HandleId> = self
            .processes
            .iter()
            .filter(|(_, s)| {
                let state = s.lock().unwrap();
                state.status == ProcessStatus::Running
            })
            .map(|(id, _)| id.clone())
            .collect();

        for handle in running_handles {
            if let Some(tx) = self.kill_senders.remove(&handle) {
                let _ = tx.send(());
            }
        }
    }
}
