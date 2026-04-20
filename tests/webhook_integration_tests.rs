// Integration tests for issue #335 — POST /wake/{agent}/{trigger}.
//
// Spins up a minimal axum Router with a hand-constructed `AppState` that
// exposes only the surface the wake handler touches (storage, driver, rate
// limiter, events_tx). The handler itself is re-used via the public router
// from `ForgeServer::build_router_for_test` would be ideal but we don't ship
// one — so these tests attach the public state + handler via the crate's
// exported types. Where the handler requires `lifecycle` / `event_bus`, we
// leave them None: the `mode: wake` branch short-circuits cleanly when no
// lifecycle is attached, which is enough to cover HMAC / driver / rate-limit
// behaviour end-to-end.

use std::sync::Arc;
use std::time::Duration;

use forge::ast::{Program, ScheduleMode};
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::http_server::ForgeServer;
use forge::runtime::storage::ForgeStorage;
use forge::runtime::webhook_driver::{WebhookDriver, WebhookRegistration};
use forge::runtime::webhook_rate_limiter::WebhookRateLimiter;

use tokio::net::TcpListener;
use tokio::sync::broadcast;

fn reg(agent: &str, trigger: &str, emit: &str, mode: ScheduleMode) -> WebhookRegistration {
    WebhookRegistration {
        agent: agent.to_string(),
        trigger: trigger.to_string(),
        mode,
        emit_event: emit.to_string(),
    }
}

fn hmac_hex(secret: &str, body: &[u8]) -> String {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    let mut mac: Hmac<Sha256> = Hmac::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Spin up a minimal ForgeServer whose state carries just enough to
/// exercise the wake-webhook handler. Returns (base URL, events_rx).
async fn start_test_server(
    driver: WebhookDriver,
    storage: Arc<ForgeStorage>,
    rate_limiter: Arc<WebhookRateLimiter>,
) -> (String, broadcast::Receiver<String>) {
    // Executor is required to construct ForgeServer; a minimal mock executor
    // works because the wake handler never hits the executor path.
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(MockProvider::new("mock")));
    let program = Program {
        boundary: None,
        items: vec![],
    };
    let executor = TaskExecutor::new(program, Arc::new(reg), None);
    let (events_tx, events_rx) = broadcast::channel::<String>(64);

    let server = ForgeServer::new(executor, None)
        .with_events_tx(events_tx)
        .with_webhook_driver(Arc::new(driver))
        .with_wake_storage(storage)
        .with_webhook_rate_limiter(rate_limiter);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");

    let router = server.build_router();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    // Give the server a moment to accept.
    tokio::time::sleep(Duration::from_millis(50)).await;
    (base, events_rx)
}

async fn next_event(
    rx: &mut broadcast::Receiver<String>,
    needle: &str,
    total_wait: Duration,
) -> Option<String> {
    let deadline = tokio::time::Instant::now() + total_wait;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(payload)) => {
                if payload.contains(needle) {
                    return Some(payload);
                }
            }
            _ => return None,
        }
    }
    None
}

#[tokio::test]
async fn valid_hmac_returns_202_and_traces_webhook_received() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(ForgeStorage::open(&dir.path().join("db.redb")).unwrap());
    storage
        .upsert_wake_secret("mastermind", "pr_merged", "test-secret-123")
        .unwrap();
    let driver = WebhookDriver::new(vec![reg(
        "mastermind",
        "pr_merged",
        "PrMerged",
        ScheduleMode::Spawn, // Spawn avoids needing an AgentLifecycle in this test.
    )]);
    let rl = Arc::new(WebhookRateLimiter::default_for_webhooks());
    let (base, mut events) = start_test_server(driver, storage, rl).await;

    let body = br#"{"repo":"repo-b","pr_number":42}"#;
    let sig = format!("sha256={}", hmac_hex("test-secret-123", body));

    let client = reqwest::Client::new();
    let res = client
        .post(format!("{base}/wake/mastermind/pr_merged"))
        .header("Content-Type", "application/json")
        .header("X-Hub-Signature-256", sig)
        .body(body.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 202);

    let evt = next_event(&mut events, "webhook_received", Duration::from_secs(1)).await;
    assert!(evt.is_some(), "expected webhook_received trace event");
    let payload = evt.unwrap();
    assert!(payload.contains("\"agent\":\"mastermind\""));
    assert!(payload.contains("\"trigger\":\"pr_merged\""));
    assert!(
        !payload.contains("test-secret-123"),
        "tracer event must not leak secret"
    );
}

#[tokio::test]
async fn tampered_body_returns_401_and_traces_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(ForgeStorage::open(&dir.path().join("db.redb")).unwrap());
    storage
        .upsert_wake_secret("mastermind", "pr_merged", "s1")
        .unwrap();
    let driver = WebhookDriver::new(vec![reg(
        "mastermind",
        "pr_merged",
        "PrMerged",
        ScheduleMode::Spawn,
    )]);
    let rl = Arc::new(WebhookRateLimiter::default_for_webhooks());
    let (base, mut events) = start_test_server(driver, storage, rl).await;

    let sig = format!("sha256={}", hmac_hex("s1", b"original"));
    let res = reqwest::Client::new()
        .post(format!("{base}/wake/mastermind/pr_merged"))
        .header("X-Hub-Signature-256", sig)
        .body(b"tampered".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 401);

    assert!(next_event(
        &mut events,
        "webhook_rejected_signature",
        Duration::from_secs(1)
    )
    .await
    .is_some());
}

#[tokio::test]
async fn unknown_secret_returns_404() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(ForgeStorage::open(&dir.path().join("db.redb")).unwrap());
    // No secret registered.
    let driver = WebhookDriver::new(vec![reg(
        "mastermind",
        "pr_merged",
        "PrMerged",
        ScheduleMode::Spawn,
    )]);
    let rl = Arc::new(WebhookRateLimiter::default_for_webhooks());
    let (base, _) = start_test_server(driver, storage, rl).await;

    let res = reqwest::Client::new()
        .post(format!("{base}/wake/mastermind/pr_merged"))
        .body(b"{}".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 404);
}

#[tokio::test]
async fn unknown_agent_or_trigger_returns_404_even_with_valid_signature() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(ForgeStorage::open(&dir.path().join("db.redb")).unwrap());
    storage.upsert_wake_secret("other", "trig", "s").unwrap();
    // Driver has no registration for (other, trig).
    let driver = WebhookDriver::new(vec![]);
    let rl = Arc::new(WebhookRateLimiter::default_for_webhooks());
    let (base, _) = start_test_server(driver, storage, rl).await;

    let body = b"{}";
    let sig = format!("sha256={}", hmac_hex("s", body));
    let res = reqwest::Client::new()
        .post(format!("{base}/wake/other/trig"))
        .header("X-Hub-Signature-256", sig)
        .body(body.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 404);
}

#[tokio::test]
async fn rate_limit_exceeded_returns_429() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(ForgeStorage::open(&dir.path().join("db.redb")).unwrap());
    storage.upsert_wake_secret("a", "t", "s").unwrap();
    let driver = WebhookDriver::new(vec![reg("a", "t", "E", ScheduleMode::Spawn)]);
    // Tiny bucket so we can blow past it in one test.
    let rl = Arc::new(WebhookRateLimiter::new(1, 2));
    let (base, _) = start_test_server(driver, storage, rl).await;

    let body = b"{}";
    let sig = format!("sha256={}", hmac_hex("s", body));
    let client = reqwest::Client::new();
    let mut statuses: Vec<u16> = Vec::new();
    for _ in 0..6 {
        let res = client
            .post(format!("{base}/wake/a/t"))
            .header("X-Hub-Signature-256", &sig)
            .body(body.to_vec())
            .send()
            .await
            .unwrap();
        statuses.push(res.status().as_u16());
    }
    assert!(
        statuses.contains(&429),
        "expected at least one 429 in {statuses:?}"
    );
    // Accepted ≤ burst (2).
    let accepted = statuses.iter().filter(|&&s| s == 202).count();
    assert!(accepted <= 2, "more than burst accepted: {statuses:?}");
}

#[tokio::test]
async fn missing_signature_header_returns_401() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(ForgeStorage::open(&dir.path().join("db.redb")).unwrap());
    storage.upsert_wake_secret("a", "t", "s").unwrap();
    let driver = WebhookDriver::new(vec![reg("a", "t", "E", ScheduleMode::Spawn)]);
    let rl = Arc::new(WebhookRateLimiter::default_for_webhooks());
    let (base, _) = start_test_server(driver, storage, rl).await;

    let res = reqwest::Client::new()
        .post(format!("{base}/wake/a/t"))
        .body(b"{}".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 401);
}
