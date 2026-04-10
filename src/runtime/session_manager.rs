// FORGE session manager — session runtime lifecycle (issue #190)
//
// Manages long-running external agent sessions with stable UUIDs,
// persisted state, resumable startup recovery, budget tracking,
// progress listeners, and graceful cancellation escalation.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Notify};
use uuid::Uuid;

use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::event_bus::{EventPayload, SharedEventBus};
use crate::tracer::Tracer;

pub type SessionId = String;
pub type ListenerId = String;
pub type SharedSessionManager = Arc<SessionManager>;
pub type SessionListener = Arc<dyn Fn(SessionEvent) + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionStatus {
    Starting,
    Running,
    Completing,
    Done,
    Failed,
    Cancelled,
}

impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionStatus::Starting => "starting",
            SessionStatus::Running => "running",
            SessionStatus::Completing => "completing",
            SessionStatus::Done => "done",
            SessionStatus::Failed => "failed",
            SessionStatus::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            SessionStatus::Done | SessionStatus::Failed | SessionStatus::Cancelled
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    pub name: String,
    pub agent: String,
    pub prompt: Option<String>,
    pub tools: Vec<String>,
    pub timeout_secs: Option<u64>,
    pub budget_usd: Option<f32>,
    pub gives: Option<String>,
    pub cancel_timeout_secs: u64,
    /// Working directory for sandbox isolation (issue #194).
    #[serde(default)]
    pub working_dir: Option<String>,
}

impl SessionConfig {
    pub fn new(name: impl Into<String>, agent: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            agent: agent.into(),
            prompt: None,
            tools: Vec::new(),
            timeout_secs: None,
            budget_usd: None,
            gives: None,
            cancel_timeout_secs: 5,
            working_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionProgress {
    pub ts: DateTime<Utc>,
    pub payload: ConfidentValue,
    pub cost_delta_usd: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub id: SessionId,
    pub config: SessionConfig,
    pub status: SessionStatus,
    pub external_session_id: Option<String>,
    pub process_id: Option<u32>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub cost_usd: f32,
    pub budget_exceeded: bool,
    pub latest_progress: Option<ConfidentValue>,
    pub progress_events: Vec<SessionProgress>,
    pub output: Option<ConfidentValue>,
    pub error: Option<String>,
}

impl SessionState {
    fn new(id: SessionId, config: SessionConfig) -> Self {
        let now = Utc::now();
        Self {
            id,
            config,
            status: SessionStatus::Starting,
            external_session_id: None,
            process_id: None,
            started_at: now,
            updated_at: now,
            cost_usd: 0.0,
            budget_exceeded: false,
            latest_progress: None,
            progress_events: Vec::new(),
            output: None,
            error: None,
        }
    }

    /// Test-only constructor for building SessionState in adapter tests.
    #[cfg(test)]
    pub fn new_for_test(id: SessionId, config: SessionConfig) -> Self {
        Self::new(id, config)
    }
}

#[derive(Debug, Clone)]
pub enum SessionEvent {
    Spawned {
        session_id: SessionId,
    },
    StateChanged {
        session_id: SessionId,
        status: SessionStatus,
    },
    Progress {
        session_id: SessionId,
        payload: ConfidentValue,
        timestamp: DateTime<Utc>,
        message: Option<String>,
    },
    BudgetUpdated {
        session_id: SessionId,
        total_cost_usd: f32,
    },
    Completed {
        session_id: SessionId,
        result: ConfidentValue,
        duration_secs: f64,
        final_status: SessionStatus,
    },
    Failed {
        session_id: SessionId,
        error: String,
    },
    Cancelled {
        session_id: SessionId,
    },
    ResumeAttempted {
        session_id: SessionId,
    },
    ResumeFailed {
        session_id: SessionId,
        error: String,
    },
}

#[derive(Clone)]
pub enum SessionDriverEvent {
    Progress {
        payload: ConfidentValue,
        cost_delta_usd: f32,
    },
    Cost {
        delta_usd: f32,
    },
    Completing,
    Completed {
        result: ConfidentValue,
    },
    Failed {
        error: String,
    },
    Cancelled,
}

pub struct SessionRuntimeHandle {
    pub external_session_id: Option<String>,
    pub process_id: Option<u32>,
    pub events: mpsc::UnboundedReceiver<SessionDriverEvent>,
    pub controller: Arc<dyn SessionController>,
}

#[async_trait]
pub trait SessionController: Send + Sync {
    async fn request_cancel(&self) -> Result<(), String>;
    async fn force_kill(&self) -> Result<(), String>;
}

pub struct NoopSessionController;

#[async_trait]
impl SessionController for NoopSessionController {
    async fn request_cancel(&self) -> Result<(), String> {
        Ok(())
    }

    async fn force_kill(&self) -> Result<(), String> {
        Ok(())
    }
}

#[async_trait]
pub trait SessionDriver: Send + Sync {
    async fn start(
        &self,
        session_id: &str,
        config: &SessionConfig,
    ) -> Result<SessionRuntimeHandle, String>;

    async fn resume(&self, state: &SessionState) -> Result<Option<SessionRuntimeHandle>, String>;
}

struct LiveSession {
    controller: Arc<dyn SessionController>,
    notify: Arc<Notify>,
}

struct SessionEntry {
    state: SessionState,
    local_listeners: Vec<SessionListener>,
    live: Option<LiveSession>,
}

struct SessionManagerInner {
    sessions: HashMap<SessionId, SessionEntry>,
    listeners: HashMap<ListenerId, SessionListener>,
    drivers: HashMap<String, Arc<dyn SessionDriver>>,
}

#[derive(Clone)]
pub struct SessionManager {
    base_dir: PathBuf,
    tracer: Option<Tracer>,
    event_bus: Arc<Mutex<Option<SharedEventBus>>>,
    polling_cancel: Arc<AtomicBool>,
    inner: Arc<Mutex<SessionManagerInner>>,
    verification_engine: Option<Arc<crate::runtime::verification_engine::VerificationEngine>>,
}

impl SessionManager {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir = base_dir.into();
        let _ = fs::create_dir_all(&base_dir);
        Self {
            base_dir,
            tracer: None,
            event_bus: Arc::new(Mutex::new(None)),
            polling_cancel: Arc::new(AtomicBool::new(false)),
            inner: Arc::new(Mutex::new(SessionManagerInner {
                sessions: HashMap::new(),
                listeners: HashMap::new(),
                drivers: HashMap::new(),
            })),
            verification_engine: None,
        }
    }

    pub fn with_tracer(mut self, tracer: Option<Tracer>) -> Self {
        self.tracer = tracer;
        self
    }

    pub fn with_event_bus(self, bus: SharedEventBus) -> Self {
        *self.event_bus.lock().unwrap() = Some(bus);
        self
    }

    pub fn with_verification_engine(
        mut self,
        engine: crate::runtime::verification_engine::VerificationEngine,
    ) -> Self {
        self.verification_engine = Some(Arc::new(engine));
        self
    }

    /// Set the event bus on an already-constructed (possibly Arc'd) manager.
    pub fn set_event_bus(&self, bus: SharedEventBus) {
        *self.event_bus.lock().unwrap() = Some(bus);
    }

    /// Start polling active sessions at the given interval, publishing
    /// `session.poll` events to the EventBus for each non-terminal session.
    /// Default interval: 5 seconds.
    pub fn start_polling(&self, interval: Duration) -> tokio::task::JoinHandle<()> {
        self.polling_cancel.store(false, Ordering::SeqCst);
        let cancel = self.polling_cancel.clone();
        let inner = self.inner.clone();
        let event_bus = self.event_bus.clone();
        let tracer = self.tracer.clone();

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await; // skip immediate first tick
            loop {
                ticker.tick().await;
                if cancel.load(Ordering::SeqCst) {
                    break;
                }

                let bus = event_bus.lock().unwrap().clone();
                let Some(bus) = bus else {
                    continue;
                };

                let active_sessions: Vec<SessionState> = {
                    let guard = inner.lock().unwrap();
                    guard
                        .sessions
                        .values()
                        .filter(|e| !e.state.status.is_terminal())
                        .map(|e| e.state.clone())
                        .collect()
                };

                if let Some(ref t) = tracer {
                    t.event_emit("session_manager", "session.poll", active_sessions.len());
                }

                let text = |s: &str| ConfidentValue::deterministic(Value::Text(s.to_string()));
                let num = |n: f64| ConfidentValue::deterministic(Value::Number(n));

                for state in &active_sessions {
                    let mut fields = HashMap::new();
                    fields.insert("session_id".to_string(), text(&state.id));
                    fields.insert("status".to_string(), text(state.status.as_str()));
                    fields.insert("cost_usd".to_string(), num(state.cost_usd as f64));
                    fields.insert(
                        "updated_at".to_string(),
                        text(&state.updated_at.to_rfc3339()),
                    );

                    let payload = EventPayload {
                        event_name: "session.poll".to_string(),
                        args: vec![text(&state.id)],
                        source_agent: "session_manager".to_string(),
                        fields,
                    };

                    if let Ok(bus_guard) = bus.try_read() {
                        bus_guard.publish(&payload);
                    }
                }
            }
        })
    }

    /// Stop the polling task.
    pub fn stop_polling(&self) {
        self.polling_cancel.store(true, Ordering::SeqCst);
    }

    pub fn register_driver(&self, name: impl Into<String>, driver: Arc<dyn SessionDriver>) {
        self.inner
            .lock()
            .unwrap()
            .drivers
            .insert(name.into(), driver);
    }

    pub fn add_listener(&self, listener: SessionListener) -> ListenerId {
        let id = Uuid::new_v4().to_string();
        self.inner
            .lock()
            .unwrap()
            .listeners
            .insert(id.clone(), listener);
        id
    }

    pub fn remove_listener(&self, id: &str) {
        self.inner.lock().unwrap().listeners.remove(id);
    }

    pub async fn spawn(&self, config: SessionConfig) -> Result<SessionId, String> {
        self.spawn_with_listeners(config, Vec::new()).await
    }

    pub async fn run_to_completion(
        &self,
        config: SessionConfig,
        local_listeners: Vec<SessionListener>,
    ) -> Result<SessionState, String> {
        let id = self.spawn_with_listeners(config, local_listeners).await?;
        self.wait(&id).await
    }

    async fn spawn_with_listeners(
        &self,
        config: SessionConfig,
        local_listeners: Vec<SessionListener>,
    ) -> Result<SessionId, String> {
        let session_id = Uuid::new_v4().to_string();
        let state = SessionState::new(session_id.clone(), config.clone());
        let driver = {
            let mut inner = self.inner.lock().unwrap();
            let driver = match inner.drivers.get(&config.agent).cloned() {
                Some(d) => d,
                None => {
                    // Fallback: create a generic adapter for the agent name
                    let fallback_config =
                        crate::runtime::adapter_loader::generic_fallback_adapter(&config.agent);
                    let fallback_driver: Arc<dyn SessionDriver> = Arc::new(
                        crate::runtime::session_adapter::ConfigDrivenDriver::new(fallback_config),
                    );
                    inner
                        .drivers
                        .insert(config.agent.clone(), fallback_driver.clone());
                    fallback_driver
                }
            };
            inner.sessions.insert(
                session_id.clone(),
                SessionEntry {
                    state: state.clone(),
                    local_listeners,
                    live: None,
                },
            );
            driver
        };

        self.persist_state(&state)?;
        self.trace_spawned(&session_id);
        self.dispatch_event(SessionEvent::Spawned {
            session_id: session_id.clone(),
        });
        self.dispatch_event(SessionEvent::StateChanged {
            session_id: session_id.clone(),
            status: SessionStatus::Starting,
        });

        match driver.start(&session_id, &config).await {
            Ok(handle) => {
                self.attach_runtime(session_id.clone(), handle, false)
                    .await?;
                Ok(session_id)
            }
            Err(err) => {
                self.mark_failed(&session_id, err.clone()).await;
                Err(err)
            }
        }
    }

    pub fn status(&self, id: &str) -> Option<SessionStatus> {
        self.inner
            .lock()
            .unwrap()
            .sessions
            .get(id)
            .map(|entry| entry.state.status.clone())
    }

    pub fn output(&self, id: &str) -> Option<ConfidentValue> {
        self.inner
            .lock()
            .unwrap()
            .sessions
            .get(id)
            .and_then(|entry| entry.state.output.clone())
    }

    pub fn session_state(&self, id: &str) -> Option<SessionState> {
        self.inner
            .lock()
            .unwrap()
            .sessions
            .get(id)
            .map(|entry| entry.state.clone())
    }

    pub fn cancel(&self, id: &str) -> Result<(), String> {
        let session_id = id.to_string();
        let exists = self
            .inner
            .lock()
            .unwrap()
            .sessions
            .contains_key(&session_id);
        if !exists {
            return Err(format!("unknown session id: {}", id));
        }
        let manager = self.clone();
        tokio::spawn(async move {
            manager.cancel_impl(session_id).await;
        });
        Ok(())
    }

    async fn cancel_impl(&self, session_id: SessionId) {
        let (controller, cancel_timeout, notify, terminal) = {
            let inner = self.inner.lock().unwrap();
            let Some(entry) = inner.sessions.get(&session_id) else {
                return;
            };
            let terminal = entry.state.status.is_terminal();
            let live = entry.live.as_ref().map(|live| {
                (
                    live.controller.clone(),
                    Duration::from_secs(entry.state.config.cancel_timeout_secs),
                    live.notify.clone(),
                )
            });
            match live {
                Some((controller, timeout, notify)) => {
                    (Some(controller), timeout, Some(notify), terminal)
                }
                None => (None, Duration::from_secs(0), None, terminal),
            }
        };

        if terminal {
            return;
        }

        let Some(controller) = controller else {
            self.mark_cancelled(&session_id).await;
            return;
        };

        if let Err(err) = controller.request_cancel().await {
            self.mark_failed(&session_id, format!("cancel request failed: {}", err))
                .await;
            return;
        }

        let Some(notify) = notify else {
            self.mark_cancelled(&session_id).await;
            return;
        };

        if cancel_timeout.is_zero() {
            let _ = controller.force_kill().await;
            self.mark_cancelled(&session_id).await;
            return;
        }

        tokio::select! {
            _ = notify.notified() => {}
            _ = tokio::time::sleep(cancel_timeout) => {
                let _ = controller.force_kill().await;
                self.mark_cancelled(&session_id).await;
            }
        }
    }

    pub async fn wait(&self, id: &str) -> Result<SessionState, String> {
        loop {
            let notify = {
                let inner = self.inner.lock().unwrap();
                let entry = inner
                    .sessions
                    .get(id)
                    .ok_or_else(|| format!("unknown session id: {}", id))?;
                if entry.state.status.is_terminal() {
                    return Ok(entry.state.clone());
                }
                entry.live.as_ref().map(|live| live.notify.clone())
            };

            match notify {
                Some(notify) => notify.notified().await,
                None => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    pub async fn resume_all(&self) -> Vec<SessionId> {
        let mut resumed = Vec::new();
        let Ok(entries) = fs::read_dir(&self.base_dir) else {
            return resumed;
        };

        for dir_entry in entries.flatten() {
            let path = dir_entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(state) = serde_json::from_str::<SessionState>(&contents) else {
                continue;
            };
            let session_id = state.id.clone();

            self.insert_persisted_state(state.clone());
            if state.status.is_terminal() {
                continue;
            }

            self.trace_resume_attempted(&session_id);
            self.dispatch_event(SessionEvent::ResumeAttempted {
                session_id: session_id.clone(),
            });

            let driver = {
                self.inner
                    .lock()
                    .unwrap()
                    .drivers
                    .get(&state.config.agent)
                    .cloned()
            };

            let Some(driver) = driver else {
                let err = format!("unknown session driver: {}", state.config.agent);
                self.trace_resume_failed(&session_id, &err);
                self.dispatch_event(SessionEvent::ResumeFailed {
                    session_id: session_id.clone(),
                    error: err.clone(),
                });
                self.mark_failed(&session_id, err).await;
                continue;
            };

            match driver.resume(&state).await {
                Ok(Some(handle)) => {
                    if self
                        .attach_runtime(session_id.clone(), handle, true)
                        .await
                        .is_ok()
                    {
                        resumed.push(session_id);
                    }
                }
                Ok(None) => {
                    let err = "driver could not resume session".to_string();
                    self.trace_resume_failed(&session_id, &err);
                    self.dispatch_event(SessionEvent::ResumeFailed {
                        session_id: session_id.clone(),
                        error: err.clone(),
                    });
                    self.mark_failed(&session_id, err).await;
                }
                Err(err) => {
                    self.trace_resume_failed(&session_id, &err);
                    self.dispatch_event(SessionEvent::ResumeFailed {
                        session_id: session_id.clone(),
                        error: err.clone(),
                    });
                    self.mark_failed(&session_id, err).await;
                }
            }
        }

        resumed
    }

    fn insert_persisted_state(&self, state: SessionState) {
        self.inner
            .lock()
            .unwrap()
            .sessions
            .entry(state.id.clone())
            .or_insert(SessionEntry {
                state,
                local_listeners: Vec::new(),
                live: None,
            });
    }

    async fn attach_runtime(
        &self,
        session_id: SessionId,
        handle: SessionRuntimeHandle,
        _resumed: bool,
    ) -> Result<(), String> {
        let notify = Arc::new(Notify::new());
        let SessionRuntimeHandle {
            external_session_id,
            process_id,
            mut events,
            controller,
        } = handle;

        let snapshot = {
            let mut inner = self.inner.lock().unwrap();
            let entry = inner
                .sessions
                .get_mut(&session_id)
                .ok_or_else(|| format!("unknown session id: {}", session_id))?;
            entry.state.external_session_id = external_session_id;
            entry.state.process_id = process_id;
            entry.state.updated_at = Utc::now();
            if !entry.state.status.is_terminal() {
                entry.state.status = SessionStatus::Running;
            }
            entry.live = Some(LiveSession {
                controller,
                notify: notify.clone(),
            });
            entry.state.clone()
        };

        self.persist_state(&snapshot)?;
        self.trace_state_changed(&session_id, &SessionStatus::Running);
        self.dispatch_event(SessionEvent::StateChanged {
            session_id: session_id.clone(),
            status: SessionStatus::Running,
        });

        let manager = self.clone();
        let session_id_for_events = session_id.clone();
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                match event {
                    SessionDriverEvent::Progress {
                        payload,
                        cost_delta_usd,
                    } => {
                        manager
                            .handle_progress(&session_id_for_events, payload, cost_delta_usd)
                            .await
                    }
                    SessionDriverEvent::Cost { delta_usd } => {
                        manager
                            .handle_cost_delta(&session_id_for_events, delta_usd)
                            .await
                    }
                    SessionDriverEvent::Completing => {
                        manager
                            .transition_status(&session_id_for_events, SessionStatus::Completing);
                    }
                    SessionDriverEvent::Completed { result } => {
                        manager.mark_completed(&session_id_for_events, result).await;
                        return;
                    }
                    SessionDriverEvent::Failed { error } => {
                        manager.mark_failed(&session_id_for_events, error).await;
                        return;
                    }
                    SessionDriverEvent::Cancelled => {
                        manager.mark_cancelled(&session_id_for_events).await;
                        return;
                    }
                }
            }

            if let Some(status) = manager.status(&session_id_for_events) {
                if !status.is_terminal() {
                    manager
                        .mark_failed(
                            &session_id_for_events,
                            "session driver event stream closed".to_string(),
                        )
                        .await;
                }
            }
        });

        if let Some(timeout_secs) = snapshot.config.timeout_secs {
            let manager = self.clone();
            let session_id_for_timeout = session_id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(timeout_secs)).await;
                manager
                    .mark_timeout_if_still_running(&session_id_for_timeout, timeout_secs)
                    .await;
            });
        }

        Ok(())
    }

    async fn mark_timeout_if_still_running(&self, session_id: &str, timeout_secs: u64) {
        let should_cancel = {
            let mut inner = self.inner.lock().unwrap();
            let Some(entry) = inner.sessions.get_mut(session_id) else {
                return;
            };
            if entry.state.status.is_terminal() {
                return;
            }
            entry.state.error = Some(format!("session timed out after {}s", timeout_secs));
            entry.state.updated_at = Utc::now();
            true
        };

        if should_cancel {
            if let Some(state) = self.session_state(session_id) {
                let _ = self.persist_state(&state);
            }
            let _ = self.cancel(session_id);
        }
    }

    async fn handle_progress(
        &self,
        session_id: &str,
        payload: ConfidentValue,
        cost_delta_usd: f32,
    ) {
        let maybe_snapshot = {
            let mut inner = self.inner.lock().unwrap();
            let Some(entry) = inner.sessions.get_mut(session_id) else {
                return;
            };
            if entry.state.status == SessionStatus::Starting {
                entry.state.status = SessionStatus::Running;
            }
            entry.state.updated_at = Utc::now();
            entry.state.cost_usd += cost_delta_usd.max(0.0);
            entry.state.latest_progress = Some(payload.clone());
            entry.state.progress_events.push(SessionProgress {
                ts: Utc::now(),
                payload: payload.clone(),
                cost_delta_usd,
            });
            Some(entry.state.clone())
        };

        let Some(snapshot) = maybe_snapshot else {
            return;
        };

        let _ = self.persist_state(&snapshot);
        self.trace_progress(session_id, snapshot.cost_usd);
        let message = match &payload.value {
            Value::Text(t) => Some(t.clone()),
            _ => None,
        };
        self.dispatch_event(SessionEvent::Progress {
            session_id: session_id.to_string(),
            payload,
            timestamp: Utc::now(),
            message,
        });
        self.dispatch_event(SessionEvent::BudgetUpdated {
            session_id: session_id.to_string(),
            total_cost_usd: snapshot.cost_usd,
        });
        self.trace_budget_updated(session_id, snapshot.cost_usd);
        self.enforce_budget_if_needed(session_id).await;
    }

    async fn handle_cost_delta(&self, session_id: &str, delta_usd: f32) {
        let maybe_snapshot = {
            let mut inner = self.inner.lock().unwrap();
            let Some(entry) = inner.sessions.get_mut(session_id) else {
                return;
            };
            entry.state.updated_at = Utc::now();
            entry.state.cost_usd += delta_usd.max(0.0);
            Some(entry.state.clone())
        };

        if let Some(snapshot) = maybe_snapshot {
            let _ = self.persist_state(&snapshot);
            self.dispatch_event(SessionEvent::BudgetUpdated {
                session_id: session_id.to_string(),
                total_cost_usd: snapshot.cost_usd,
            });
            self.trace_budget_updated(session_id, snapshot.cost_usd);
            self.enforce_budget_if_needed(session_id).await;
        }
    }

    async fn enforce_budget_if_needed(&self, session_id: &str) {
        let should_cancel = {
            let mut inner = self.inner.lock().unwrap();
            let Some(entry) = inner.sessions.get_mut(session_id) else {
                return;
            };
            let Some(budget) = entry.state.config.budget_usd else {
                return;
            };
            if entry.state.budget_exceeded || entry.state.cost_usd <= budget {
                return;
            }
            entry.state.budget_exceeded = true;
            entry.state.error = Some(format!(
                "session budget exceeded: spent ${:.4} of ${:.4}",
                entry.state.cost_usd, budget
            ));
            true
        };

        if should_cancel {
            let _ = self.cancel(session_id);
        }
    }

    fn transition_status(&self, session_id: &str, status: SessionStatus) {
        let maybe_snapshot = {
            let mut inner = self.inner.lock().unwrap();
            let Some(entry) = inner.sessions.get_mut(session_id) else {
                return;
            };
            if entry.state.status == status {
                return;
            }
            if entry.state.status.is_terminal() {
                return;
            }
            entry.state.status = status.clone();
            entry.state.updated_at = Utc::now();
            Some(entry.state.clone())
        };

        if let Some(snapshot) = maybe_snapshot {
            let _ = self.persist_state(&snapshot);
            self.trace_state_changed(session_id, &status);
            self.dispatch_event(SessionEvent::StateChanged {
                session_id: session_id.to_string(),
                status,
            });
        }
    }

    async fn mark_completed(&self, session_id: &str, result: ConfidentValue) {
        self.transition_status(session_id, SessionStatus::Completing);
        let result = self.inject_cost(result, session_id);
        let result = self.run_verification(result, session_id).await;
        let (snapshot, notify) = {
            let mut inner = self.inner.lock().unwrap();
            let Some(entry) = inner.sessions.get_mut(session_id) else {
                return;
            };
            entry.state.status = SessionStatus::Done;
            entry.state.output = Some(result.clone());
            entry.state.updated_at = Utc::now();
            let notify = entry.live.as_ref().map(|live| live.notify.clone());
            entry.live = None;
            (entry.state.clone(), notify)
        };

        let _ = self.persist_state(&snapshot);
        self.trace_completed(session_id, snapshot.cost_usd);
        let duration_secs =
            (Utc::now() - snapshot.started_at).num_milliseconds().max(0) as f64 / 1000.0;
        self.dispatch_event(SessionEvent::Completed {
            session_id: session_id.to_string(),
            result,
            duration_secs,
            final_status: SessionStatus::Done,
        });
        self.dispatch_event(SessionEvent::StateChanged {
            session_id: session_id.to_string(),
            status: SessionStatus::Done,
        });
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
    }

    async fn mark_failed(&self, session_id: &str, error: String) {
        let (snapshot, notify) = {
            let mut inner = self.inner.lock().unwrap();
            let Some(entry) = inner.sessions.get_mut(session_id) else {
                return;
            };
            entry.state.status = SessionStatus::Failed;
            entry.state.error = Some(error.clone());
            entry.state.updated_at = Utc::now();
            let notify = entry.live.as_ref().map(|live| live.notify.clone());
            entry.live = None;
            (entry.state.clone(), notify)
        };

        let _ = self.persist_state(&snapshot);
        self.trace_failed(session_id, &error);
        self.dispatch_event(SessionEvent::Failed {
            session_id: session_id.to_string(),
            error: error.clone(),
        });
        self.dispatch_event(SessionEvent::StateChanged {
            session_id: session_id.to_string(),
            status: SessionStatus::Failed,
        });
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
    }

    async fn mark_cancelled(&self, session_id: &str) {
        let (snapshot, notify, changed) = {
            let mut inner = self.inner.lock().unwrap();
            let Some(entry) = inner.sessions.get_mut(session_id) else {
                return;
            };
            let changed = entry.state.status != SessionStatus::Cancelled;
            entry.state.status = SessionStatus::Cancelled;
            entry.state.updated_at = Utc::now();
            let notify = entry.live.as_ref().map(|live| live.notify.clone());
            entry.live = None;
            (entry.state.clone(), notify, changed)
        };

        let _ = self.persist_state(&snapshot);
        if changed {
            self.trace_cancelled(session_id);
            self.dispatch_event(SessionEvent::Cancelled {
                session_id: session_id.to_string(),
            });
            self.dispatch_event(SessionEvent::StateChanged {
                session_id: session_id.to_string(),
                status: SessionStatus::Cancelled,
            });
        }
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
    }

    fn inject_cost(&self, result: ConfidentValue, session_id: &str) -> ConfidentValue {
        let cost_usd = self
            .session_state(session_id)
            .map(|state| state.cost_usd)
            .unwrap_or(0.0);

        match result.value {
            Value::Record(mut fields) => {
                if fields.contains_key("cost_usd") {
                    fields.insert(
                        "cost_usd".to_string(),
                        ConfidentValue::deterministic(Value::Number(cost_usd as f64)),
                    );
                    ConfidentValue::from_agent_result(fields)
                } else {
                    ConfidentValue {
                        value: Value::Record(fields),
                        confidence: result.confidence,
                        source: result.source,
                    }
                }
            }
            other => ConfidentValue {
                value: other,
                confidence: result.confidence,
                source: result.source,
            },
        }
    }

    /// Run the verification engine (if configured) on a completed session result.
    /// Updates metadata.verification from Pending to the resolved status.
    async fn run_verification(&self, result: ConfidentValue, session_id: &str) -> ConfidentValue {
        let engine = match &self.verification_engine {
            Some(e) => Arc::clone(e),
            None => return result,
        };

        // Use the session's working_dir (set by `isolate worktree`), or fall
        // back to the process working directory so the ReferenceValidator can
        // check claimed files against the actual project.
        let working_dir = self
            .session_state(session_id)
            .and_then(|s| s.config.working_dir.clone())
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_dir().ok());

        let (fields, claims) =
            crate::runtime::verification_engine::extract_verification_inputs(&result);

        let ctx = crate::runtime::verification_engine::VerificationContext {
            working_dir,
            agent_fields: fields,
            claims,
        };

        let vr = engine.verify(&ctx).await;
        crate::runtime::verification_engine::inject_resolved_verification(result, vr)
    }

    fn state_path(&self, session_id: &str) -> PathBuf {
        self.base_dir.join(format!("{}.json", session_id))
    }

    fn persist_state(&self, state: &SessionState) -> Result<(), String> {
        fs::create_dir_all(&self.base_dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
        fs::write(self.state_path(&state.id), json).map_err(|e| e.to_string())
    }

    fn dispatch_event(&self, event: SessionEvent) {
        let listeners = {
            let inner = self.inner.lock().unwrap();
            let session_id = session_id_of(&event);
            let mut listeners: Vec<SessionListener> = inner.listeners.values().cloned().collect();
            if let Some(session_id) = session_id {
                if let Some(entry) = inner.sessions.get(session_id) {
                    listeners.extend(entry.local_listeners.iter().cloned());
                }
            }
            listeners
        };

        for listener in listeners {
            listener(event.clone());
        }

        // Publish to EventBus if available (Principle VIII: traced)
        let bus = self.event_bus.lock().unwrap().clone();
        if let Some(ref bus) = bus {
            if let Some(payload) = session_event_to_payload(&event) {
                match bus.try_read() {
                    Ok(bus) => {
                        let delivered = bus.publish(&payload);
                        if let Some(ref t) = self.tracer {
                            t.event_emit("session_manager", &payload.event_name, delivered);
                        }
                    }
                    Err(_) => {
                        // Principle V: bounded, drop-on-contention with trace
                        if let Some(ref t) = self.tracer {
                            t.event_delivery_failed(
                                &payload.event_name,
                                "event_bus",
                                "lock_contention",
                            );
                        }
                    }
                }
            }
        }
    }

    fn trace_spawned(&self, session_id: &str) {
        if let Some(ref tracer) = self.tracer {
            tracer.session_spawned(session_id);
        }
    }

    fn trace_state_changed(&self, session_id: &str, status: &SessionStatus) {
        if let Some(ref tracer) = self.tracer {
            tracer.session_state_changed(session_id, status.as_str());
        }
    }

    fn trace_progress(&self, session_id: &str, total_cost_usd: f32) {
        if let Some(ref tracer) = self.tracer {
            tracer.session_progress(session_id, total_cost_usd);
        }
    }

    fn trace_budget_updated(&self, session_id: &str, total_cost_usd: f32) {
        if let Some(ref tracer) = self.tracer {
            tracer.session_budget_updated(session_id, total_cost_usd);
        }
    }

    fn trace_completed(&self, session_id: &str, total_cost_usd: f32) {
        if let Some(ref tracer) = self.tracer {
            tracer.session_completed(session_id, total_cost_usd);
        }
    }

    fn trace_failed(&self, session_id: &str, error: &str) {
        if let Some(ref tracer) = self.tracer {
            tracer.session_failed(session_id, error);
        }
    }

    fn trace_cancelled(&self, session_id: &str) {
        if let Some(ref tracer) = self.tracer {
            tracer.session_cancelled(session_id);
        }
    }

    fn trace_resume_attempted(&self, session_id: &str) {
        if let Some(ref tracer) = self.tracer {
            tracer.session_resume_attempted(session_id);
        }
    }

    fn trace_resume_failed(&self, session_id: &str, error: &str) {
        if let Some(ref tracer) = self.tracer {
            tracer.session_resume_failed(session_id, error);
        }
    }
}

fn session_id_of(event: &SessionEvent) -> Option<&str> {
    match event {
        SessionEvent::Spawned { session_id }
        | SessionEvent::StateChanged { session_id, .. }
        | SessionEvent::Progress { session_id, .. }
        | SessionEvent::BudgetUpdated { session_id, .. }
        | SessionEvent::Completed { session_id, .. }
        | SessionEvent::Failed { session_id, .. }
        | SessionEvent::Cancelled { session_id }
        | SessionEvent::ResumeAttempted { session_id }
        | SessionEvent::ResumeFailed { session_id, .. } => Some(session_id.as_str()),
    }
}

fn session_event_to_payload(event: &SessionEvent) -> Option<EventPayload> {
    let text = |s: &str| ConfidentValue::deterministic(Value::Text(s.to_string()));
    let num = |n: f64| ConfidentValue::deterministic(Value::Number(n));

    match event {
        SessionEvent::Progress {
            session_id,
            payload,
            timestamp,
            message,
        } => {
            let mut fields = HashMap::new();
            fields.insert("session_id".to_string(), text(session_id));
            fields.insert("timestamp".to_string(), text(&timestamp.to_rfc3339()));
            fields.insert("output".to_string(), payload.clone());
            if let Some(msg) = message {
                fields.insert("message".to_string(), text(msg));
            }
            Some(EventPayload {
                event_name: "session.progress".to_string(),
                args: vec![payload.clone()],
                source_agent: "session_manager".to_string(),
                fields,
            })
        }
        SessionEvent::Completed {
            session_id,
            result,
            duration_secs,
            final_status,
        } => {
            let mut fields = HashMap::new();
            fields.insert("session_id".to_string(), text(session_id));
            fields.insert("result".to_string(), result.clone());
            fields.insert("duration_secs".to_string(), num(*duration_secs));
            fields.insert("final_status".to_string(), text(final_status.as_str()));
            Some(EventPayload {
                event_name: "session.complete".to_string(),
                args: vec![result.clone()],
                source_agent: "session_manager".to_string(),
                fields,
            })
        }
        SessionEvent::Failed { session_id, error } => {
            let mut fields = HashMap::new();
            fields.insert("session_id".to_string(), text(session_id));
            fields.insert("error".to_string(), text(error));
            Some(EventPayload {
                event_name: "session.failed".to_string(),
                args: vec![text(error)],
                source_agent: "session_manager".to_string(),
                fields,
            })
        }
        SessionEvent::Cancelled { session_id } => {
            let mut fields = HashMap::new();
            fields.insert("session_id".to_string(), text(session_id));
            Some(EventPayload {
                event_name: "session.cancelled".to_string(),
                args: vec![text(session_id)],
                source_agent: "session_manager".to_string(),
                fields,
            })
        }
        // Other events (Spawned, StateChanged, BudgetUpdated, Resume*) are internal
        _ => None,
    }
}

pub fn default_session_dir(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref().join(".forge-data").join("sessions")
}

pub fn new_shared_default_session_manager(tracer: Option<Tracer>) -> SharedSessionManager {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let manager = SessionManager::new(default_session_dir(&root))
        .with_tracer(tracer)
        .with_verification_engine(
            crate::runtime::verification_engine::VerificationEngine::coding_session(),
        );

    // Auto-register adapters from the resolution chain:
    //   1. Project local: ./adapters/{name}/ADAPTER.toml
    //   2. Built-in: {exe_dir}/adapters/{name}/ADAPTER.toml
    let mut adapter_dirs = vec![root.join("adapters")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            adapter_dirs.push(exe_dir.join("adapters"));
        }
    }

    for dir in &adapter_dirs {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let toml_path = entry.path().join("ADAPTER.toml");
                if toml_path.exists() {
                    match crate::runtime::adapter_loader::parse_adapter_toml(&toml_path) {
                        Ok(config) => {
                            let name = config.name.clone();
                            let driver = Arc::new(
                                crate::runtime::session_adapter::ConfigDrivenDriver::new(config),
                            );
                            manager.register_driver(name, driver);
                        }
                        Err(e) => {
                            eprintln!(
                                "warning: failed to load adapter {}: {}",
                                toml_path.display(),
                                e
                            );
                        }
                    }
                }
            }
        }
    }

    Arc::new(manager)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    fn text(s: &str) -> ConfidentValue {
        ConfidentValue::from_skill(Value::Text(s.to_string()), 0.8)
    }

    fn temp_manager() -> (TempDir, SessionManager) {
        let dir = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(default_session_dir(dir.path()));
        (dir, manager)
    }

    #[derive(Default)]
    struct TestController {
        cancel_requests: AtomicUsize,
        kill_requests: AtomicUsize,
    }

    #[async_trait]
    impl SessionController for TestController {
        async fn request_cancel(&self) -> Result<(), String> {
            self.cancel_requests.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn force_kill(&self) -> Result<(), String> {
            self.kill_requests.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct ScriptedDriver {
        factory: Arc<dyn Fn() -> SessionRuntimeHandle + Send + Sync>,
        resume_factory: Option<Arc<dyn Fn() -> Option<SessionRuntimeHandle> + Send + Sync>>,
    }

    #[async_trait]
    impl SessionDriver for ScriptedDriver {
        async fn start(
            &self,
            _session_id: &str,
            _config: &SessionConfig,
        ) -> Result<SessionRuntimeHandle, String> {
            Ok((self.factory)())
        }

        async fn resume(
            &self,
            _state: &SessionState,
        ) -> Result<Option<SessionRuntimeHandle>, String> {
            match &self.resume_factory {
                Some(factory) => Ok(factory()),
                None => Ok(None),
            }
        }
    }

    fn runtime_with_events(
        controller: Arc<TestController>,
        events: Vec<SessionDriverEvent>,
    ) -> SessionRuntimeHandle {
        let (tx, rx) = mpsc::unbounded_channel();
        for event in events {
            tx.send(event).unwrap();
        }
        drop(tx);
        SessionRuntimeHandle {
            external_session_id: Some("driver-123".to_string()),
            process_id: Some(4321),
            events: rx,
            controller,
        }
    }

    fn runtime_keepalive(controller: Arc<TestController>) -> SessionRuntimeHandle {
        let (tx, rx) = mpsc::unbounded_channel::<SessionDriverEvent>();
        tokio::spawn(async move {
            let _tx = tx;
            std::future::pending::<()>().await;
        });
        SessionRuntimeHandle {
            external_session_id: None,
            process_id: None,
            events: rx,
            controller,
        }
    }

    fn runtime_keepalive_with_events(
        controller: Arc<TestController>,
        events: Vec<SessionDriverEvent>,
    ) -> SessionRuntimeHandle {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            for event in events {
                let _ = tx.send(event);
            }
            std::future::pending::<()>().await;
        });
        SessionRuntimeHandle {
            external_session_id: None,
            process_id: None,
            events: rx,
            controller,
        }
    }

    #[tokio::test]
    async fn lifecycle_runs_start_to_complete() {
        let (_dir, manager) = temp_manager();
        manager.register_driver(
            "fake",
            Arc::new(ScriptedDriver {
                factory: Arc::new(|| {
                    runtime_with_events(
                        Arc::new(TestController::default()),
                        vec![
                            SessionDriverEvent::Progress {
                                payload: text("working"),
                                cost_delta_usd: 0.2,
                            },
                            SessionDriverEvent::Completing,
                            SessionDriverEvent::Completed {
                                result: text("done"),
                            },
                        ],
                    )
                }),
                resume_factory: None,
            }),
        );

        let state = manager
            .run_to_completion(SessionConfig::new("review", "fake"), Vec::new())
            .await
            .unwrap();

        assert_eq!(state.status, SessionStatus::Done);
        assert!((state.cost_usd - 0.2).abs() < f32::EPSILON);
        assert_eq!(
            manager.output(&state.id).unwrap().value.to_string(),
            "done".to_string()
        );
    }

    #[tokio::test]
    async fn cancel_escalates_to_force_kill_after_timeout() {
        let (_dir, manager) = temp_manager();
        let controller = Arc::new(TestController::default());
        let controller_for_driver = controller.clone();
        manager.register_driver(
            "fake",
            Arc::new(ScriptedDriver {
                factory: Arc::new(move || runtime_keepalive(controller_for_driver.clone())),
                resume_factory: None,
            }),
        );

        let mut config = SessionConfig::new("review", "fake");
        config.cancel_timeout_secs = 0;
        let session_id = manager.spawn(config).await.unwrap();
        manager.cancel(&session_id).unwrap();
        let state = manager.wait(&session_id).await.unwrap();

        assert_eq!(state.status, SessionStatus::Cancelled);
        assert_eq!(controller.cancel_requests.load(Ordering::SeqCst), 1);
        assert_eq!(controller.kill_requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn resume_all_reattaches_running_sessions() {
        let (dir, manager) = temp_manager();
        let mut persisted = SessionState::new(
            "resume-me".to_string(),
            SessionConfig::new("review", "fake"),
        );
        persisted.status = SessionStatus::Running;
        persisted.updated_at = Utc::now();
        manager.persist_state(&persisted).unwrap();

        manager.register_driver(
            "fake",
            Arc::new(ScriptedDriver {
                factory: Arc::new(|| {
                    runtime_with_events(
                        Arc::new(TestController::default()),
                        vec![SessionDriverEvent::Completed {
                            result: text("resumed"),
                        }],
                    )
                }),
                resume_factory: Some(Arc::new(|| {
                    Some(runtime_with_events(
                        Arc::new(TestController::default()),
                        vec![SessionDriverEvent::Completed {
                            result: text("resumed"),
                        }],
                    ))
                })),
            }),
        );

        let reloaded = SessionManager::new(default_session_dir(dir.path()));
        reloaded.register_driver(
            "fake",
            Arc::new(ScriptedDriver {
                factory: Arc::new(|| {
                    runtime_with_events(
                        Arc::new(TestController::default()),
                        vec![SessionDriverEvent::Completed {
                            result: text("resumed"),
                        }],
                    )
                }),
                resume_factory: Some(Arc::new(|| {
                    Some(runtime_with_events(
                        Arc::new(TestController::default()),
                        vec![SessionDriverEvent::Completed {
                            result: text("resumed"),
                        }],
                    ))
                })),
            }),
        );

        let resumed = reloaded.resume_all().await;
        assert_eq!(resumed, vec!["resume-me".to_string()]);
        let state = reloaded.wait("resume-me").await.unwrap();
        assert_eq!(state.status, SessionStatus::Done);
    }

    #[tokio::test]
    async fn budget_exceeded_triggers_cancel() {
        let (_dir, manager) = temp_manager();
        let controller = Arc::new(TestController::default());
        let controller_for_driver = controller.clone();
        manager.register_driver(
            "fake",
            Arc::new(ScriptedDriver {
                factory: Arc::new(move || {
                    runtime_keepalive_with_events(
                        controller_for_driver.clone(),
                        vec![SessionDriverEvent::Progress {
                            payload: text("expensive"),
                            cost_delta_usd: 2.5,
                        }],
                    )
                }),
                resume_factory: None,
            }),
        );

        let mut config = SessionConfig::new("review", "fake");
        config.budget_usd = Some(1.0);
        config.cancel_timeout_secs = 0;
        let session_id = manager.spawn(config).await.unwrap();
        let state = manager.wait(&session_id).await.unwrap();

        assert_eq!(state.status, SessionStatus::Cancelled);
        assert!(state.budget_exceeded);
        assert_eq!(controller.cancel_requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn timeout_triggers_cancel() {
        let (_dir, manager) = temp_manager();
        let controller = Arc::new(TestController::default());
        let controller_for_driver = controller.clone();
        manager.register_driver(
            "fake",
            Arc::new(ScriptedDriver {
                factory: Arc::new(move || runtime_keepalive(controller_for_driver.clone())),
                resume_factory: None,
            }),
        );

        let mut config = SessionConfig::new("review", "fake");
        config.timeout_secs = Some(0);
        config.cancel_timeout_secs = 0;
        let session_id = manager.spawn(config).await.unwrap();
        let state = manager.wait(&session_id).await.unwrap();

        assert_eq!(state.status, SessionStatus::Cancelled);
        assert_eq!(state.error.as_deref(), Some("session timed out after 0s"));
        assert_eq!(controller.cancel_requests.load(Ordering::SeqCst), 1);
    }
}
