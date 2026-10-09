//! Fixture recording and replay for `forge run --record` / `forge test` (#478).
//!
//! One JSON file per program (default `<program>.fixtures.json`, beside it)
//! records every provider response keyed by a normalized request hash, so a
//! recorded run replays offline: no network, zero tokens, zero cost.

use crate::llm::{
    BoxedProvider, CompletionRequest, CompletionResponse, LLMProvider, ProviderCapabilities,
    ProviderError, ToolCallRequest,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Fixture format version written by `save` and required by `load`.
const VERSION: u32 = 1;
/// Prompt characters kept for human eyes in `prompt_preview` (not part of the key).
const PREVIEW_CHARS: usize = 120;
/// Prompt characters quoted in a replay-miss error.
const MISS_PREVIEW_CHARS: usize = 60;

// ── Mode (process-global, set once by the CLI) ───────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtureMode {
    Record(PathBuf),
    Replay(PathBuf),
}

static FIXTURE_MODE: OnceLock<FixtureMode> = OnceLock::new();

/// Set by the CLI before the program runs. First call wins; later calls are ignored.
pub fn set_mode(m: FixtureMode) {
    let _ = FIXTURE_MODE.set(m);
}

pub fn mode() -> Option<&'static FixtureMode> {
    FIXTURE_MODE.get()
}

/// Default fixture path for a program: `<program>.fixtures.json` beside it.
pub fn default_path(program: &Path) -> PathBuf {
    let mut name = program.as_os_str().to_os_string();
    name.push(".fixtures.json");
    PathBuf::from(name)
}

// ── File format ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FixtureFile {
    version: u32,
    calls: Vec<RecordedCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecordedCall {
    key: String,
    index: u32,
    provider: String,
    prompt_preview: String,
    response: RecordedResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecordedResponse {
    content: String,
    #[serde(default)]
    tool_calls: Vec<ToolCallRequest>,
    model_used: String,
    provider_name: String,
}

/// Cache key: provider name, system, prompt, json mode, and tool names.
/// Two calls that differ only in temperature or max_tokens share a key.
fn call_key(provider: &str, request: &CompletionRequest) -> String {
    let tools = request
        .tools
        .iter()
        .map(|t| t.name.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let mut buf = Vec::new();
    for part in [
        provider,
        request.system.as_deref().unwrap_or(""),
        request.prompt.as_str(),
    ] {
        buf.extend_from_slice(part.as_bytes());
        buf.push(0);
    }
    buf.push(request.json_mode as u8);
    buf.push(0);
    buf.extend_from_slice(tools.as_bytes());
    crate::portability::sha256_hex(&buf)
}

fn preview(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

fn fixture_err(message: String) -> ProviderError {
    ProviderError::Fixture(message)
}

/// Persist the whole fixture file. Failures come back as plain messages: a
/// failed write must never discard a response the run already paid for.
fn save(path: &Path, calls: &[RecordedCall]) -> Result<(), String> {
    let file = FixtureFile {
        version: VERSION,
        calls: calls.to_vec(),
    };
    let json = serde_json::to_string_pretty(&file).map_err(|e| format!("cannot encode: {e}"))?;
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
    }
    std::fs::write(path, json).map_err(|e| e.to_string())
}

fn load(path: &Path) -> Result<FixtureFile, ProviderError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| fixture_err(format!("failed to read fixtures {}: {e}", path.display())))?;
    let file: FixtureFile = serde_json::from_str(&text)
        .map_err(|e| fixture_err(format!("invalid fixtures at {}: {e}", path.display())))?;
    if file.version != VERSION {
        return Err(fixture_err(format!(
            "unsupported fixture version {} at {} (expected {VERSION})",
            file.version,
            path.display()
        )));
    }
    Ok(file)
}

// ── Providers ────────────────────────────────────────────────────────────────

/// Wraps a real provider: forwards the call, then persists the response.
/// The whole file is rewritten after each call, so a crash keeps what was recorded.
struct RecordingProvider {
    name: String,
    inner: BoxedProvider,
    path: PathBuf,
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

#[async_trait]
impl LLMProvider for RecordingProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        self.inner.capabilities()
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let key = call_key(&self.name, &request);
        let prompt_preview = preview(&request.prompt, PREVIEW_CHARS);
        let resp = self.inner.complete(request).await?;

        // No `.await` inside this lock: the recorded state is sync-only.
        let mut calls = self.calls.lock().expect("fixture lock poisoned");
        let index = calls.iter().filter(|c| c.key == key).count() as u32;
        calls.push(RecordedCall {
            key,
            index,
            provider: self.name.clone(),
            prompt_preview,
            response: RecordedResponse {
                content: resp.content.clone(),
                tool_calls: resp.tool_calls.clone(),
                model_used: resp.model_used.clone(),
                provider_name: resp.provider_name.clone(),
            },
        });
        save(&self.path, &calls).unwrap_or_else(|e| {
            // The response is already paid for: warn, keep it, keep going.
            eprintln!(
                "[forge] warning: could not write fixtures to {}: {e}",
                self.path.display()
            );
        });
        Ok(resp)
    }
}

struct ReplayState {
    path: PathBuf,
    by_key: HashMap<String, Vec<RecordedResponse>>,
    cursors: Mutex<HashMap<String, u32>>,
}

impl ReplayState {
    fn load(path: &Path) -> Result<Self, ProviderError> {
        let file = load(path)?;
        let mut grouped: HashMap<String, Vec<(u32, RecordedResponse)>> = HashMap::new();
        for call in file.calls {
            grouped
                .entry(call.key)
                .or_default()
                .push((call.index, call.response));
        }
        let by_key = grouped
            .into_iter()
            .map(|(key, mut entries)| {
                entries.sort_by_key(|(index, _)| *index);
                (key, entries.into_iter().map(|(_, r)| r).collect())
            })
            .collect();
        Ok(Self {
            path: path.to_path_buf(),
            by_key,
            cursors: Mutex::new(HashMap::new()),
        })
    }

    /// n-th response for `key` in this run: repeated identical prompts replay
    /// in the order they were recorded.
    fn next(&self, key: &str) -> Option<RecordedResponse> {
        let mut cursors = self.cursors.lock().expect("fixture lock poisoned");
        let cursor = cursors.entry(key.to_string()).or_insert(0);
        let hit = self
            .by_key
            .get(key)
            .and_then(|responses| responses.get(*cursor as usize))
            .cloned();
        *cursor += 1;
        hit
    }
}

/// Replaces a real provider during replay: never calls the inner provider,
/// and reports zero tokens/cost/latency (Principle III — replay spends nothing).
struct ReplayProvider {
    name: String,
    caps: ProviderCapabilities,
    fixtures: Arc<ReplayState>,
}

#[async_trait]
impl LLMProvider for ReplayProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        &self.caps
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let key = call_key(&self.name, &request);
        let recorded = self.fixtures.next(&key);
        let Some(recorded) = recorded else {
            return Err(fixture_err(format!(
                "no recorded response for provider '{}' (prompt starts: {:?}); \
                 re-record with: forge run <program.forge> --record {}",
                self.name,
                preview(&request.prompt, MISS_PREVIEW_CHARS),
                self.fixtures.path.display()
            )));
        };
        Ok(CompletionResponse {
            content: recorded.content,
            tool_calls: recorded.tool_calls,
            tokens_in: 0,
            tokens_out: 0,
            latency_ms: 0,
            model_used: recorded.model_used,
            provider_name: recorded.provider_name,
            cost_usd: 0.0,
        })
    }
}

/// Replace every provider with its recorder/replayer for `mode`.
/// Called by `ProviderRegistry::from_config` and directly by tests.
pub fn wrap_providers(
    providers: &mut HashMap<String, BoxedProvider>,
    mode: &FixtureMode,
) -> Result<(), ProviderError> {
    match mode {
        FixtureMode::Record(path) => {
            // Start clean: a record run that makes no calls (or crashes early)
            // must not leave a stale file behind for `forge test` to replay.
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(fixture_err(format!(
                        "failed to remove stale fixtures {}: {e}",
                        path.display()
                    )))
                }
            }
            // The run's calls live in memory; each one rewrites the whole file.
            let calls = Arc::new(Mutex::new(Vec::new()));
            for (name, inner) in providers.iter_mut() {
                *inner = Arc::new(RecordingProvider {
                    name: name.clone(),
                    inner: Arc::clone(inner),
                    path: path.clone(),
                    calls: Arc::clone(&calls),
                });
            }
        }
        FixtureMode::Replay(path) => {
            let state = Arc::new(ReplayState::load(path)?);
            for (name, inner) in providers.iter_mut() {
                *inner = Arc::new(ReplayProvider {
                    name: name.clone(),
                    caps: inner.capabilities().clone(),
                    fixtures: Arc::clone(&state),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::providers::mock::MockProvider;

    fn one(inner: BoxedProvider) -> HashMap<String, BoxedProvider> {
        HashMap::from([("mock".to_string(), inner)])
    }

    fn mock(content: &str) -> BoxedProvider {
        Arc::new(MockProvider::new("mock").with_default(content))
    }

    fn fixture_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("prog.forge.fixtures.json")
    }

    fn tool_call() -> ToolCallRequest {
        ToolCallRequest {
            id: "call-1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "a.txt"}),
        }
    }

    async fn complete(
        providers: &HashMap<String, BoxedProvider>,
        prompt: &str,
    ) -> Result<CompletionResponse, ProviderError> {
        providers["mock"]
            .complete(CompletionRequest::simple(prompt))
            .await
    }

    #[tokio::test]
    async fn record_then_replay_round_trip_is_identical() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture_path(&dir);

        let inner: BoxedProvider = Arc::new(
            MockProvider::new("mock")
                .with_default("hello from the real provider")
                .with_tool_call_response(vec![tool_call()]),
        );
        let mut recording = one(inner);
        wrap_providers(&mut recording, &FixtureMode::Record(path.clone())).unwrap();
        let live = complete(&recording, "greet me").await.unwrap();
        assert!(path.exists(), "each call rewrites the fixture file");

        let mut replaying = one(mock("SHOULD NOT BE USED"));
        wrap_providers(&mut replaying, &FixtureMode::Replay(path)).unwrap();
        let replayed = complete(&replaying, "greet me").await.unwrap();

        assert_eq!(replayed.content, live.content);
        assert_eq!(replayed.tool_calls.len(), 1);
        assert_eq!(replayed.tool_calls[0].id, live.tool_calls[0].id);
        assert_eq!(replayed.tool_calls[0].name, live.tool_calls[0].name);
        assert_eq!(
            replayed.tool_calls[0].arguments,
            live.tool_calls[0].arguments
        );
        assert_eq!(replayed.model_used, live.model_used);
        assert_eq!(replayed.provider_name, live.provider_name);
    }

    #[tokio::test]
    async fn replay_reports_zero_tokens_and_cost() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture_path(&dir);

        let mut recording = one(mock("paid response"));
        wrap_providers(&mut recording, &FixtureMode::Record(path.clone())).unwrap();
        let live = complete(&recording, "a prompt long enough to cost tokens")
            .await
            .unwrap();
        assert!(live.tokens_in > 0, "control: the live call billed tokens");

        let mut replaying = one(mock("unused"));
        wrap_providers(&mut replaying, &FixtureMode::Replay(path)).unwrap();
        let replayed = complete(&replaying, "a prompt long enough to cost tokens")
            .await
            .unwrap();

        assert_eq!(replayed.tokens_in, 0);
        assert_eq!(replayed.tokens_out, 0);
        assert_eq!(replayed.cost_usd, 0.0);
        assert_eq!(replayed.latency_ms, 0);
        assert_eq!(replayed.content, "paid response");
    }

    #[tokio::test]
    async fn repeated_identical_prompts_replay_in_index_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture_path(&dir);

        let inner: BoxedProvider = Arc::new(
            MockProvider::new("mock")
                .with_responses_sequence(vec!["first".to_string(), "second".to_string()]),
        );
        let mut recording = one(inner);
        wrap_providers(&mut recording, &FixtureMode::Record(path.clone())).unwrap();
        assert_eq!(
            complete(&recording, "same prompt").await.unwrap().content,
            "first"
        );
        assert_eq!(
            complete(&recording, "same prompt").await.unwrap().content,
            "second"
        );

        let file: FixtureFile =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(file.version, 1);
        assert_eq!(file.calls.len(), 2);
        assert_eq!(file.calls[0].key, file.calls[1].key);
        assert_eq!(file.calls[0].index, 0);
        assert_eq!(file.calls[1].index, 1);
        assert_eq!(file.calls[0].prompt_preview, "same prompt");

        let mut replaying = one(mock("unused"));
        wrap_providers(&mut replaying, &FixtureMode::Replay(path)).unwrap();
        assert_eq!(
            complete(&replaying, "same prompt").await.unwrap().content,
            "first"
        );
        assert_eq!(
            complete(&replaying, "same prompt").await.unwrap().content,
            "second"
        );
    }

    #[tokio::test]
    async fn replay_miss_names_provider_prompt_and_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture_path(&dir);

        let mut recording = one(mock("recorded"));
        wrap_providers(&mut recording, &FixtureMode::Record(path.clone())).unwrap();
        complete(&recording, "the recorded prompt").await.unwrap();

        let mut replaying = one(mock("unused"));
        wrap_providers(&mut replaying, &FixtureMode::Replay(path.clone())).unwrap();
        let err = complete(&replaying, "a different prompt")
            .await
            .expect_err("a miss is a hard error");
        let msg = err.to_string();

        assert!(
            msg.starts_with(
                "no recorded response for provider 'mock' (prompt starts: \"a different prompt\")"
            ),
            "unexpected miss message: {msg}"
        );
        assert!(msg.contains("re-record with: forge run"), "{msg}");
        assert!(msg.contains(path.to_str().unwrap()), "{msg}");
    }

    #[tokio::test]
    async fn recording_starts_from_a_clean_fixture_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture_path(&dir);
        std::fs::write(&path, r#"{"version":1,"calls":[]}"#).unwrap();

        let mut recording = one(mock("fresh"));
        wrap_providers(&mut recording, &FixtureMode::Record(path.clone())).unwrap();
        assert!(
            !path.exists(),
            "recording removes the stale file before the run"
        );
        assert!(!path.exists(), "a zero-call run writes nothing back");

        complete(&recording, "hi").await.unwrap();
        let file: FixtureFile = serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
            .expect("the fresh recording is valid JSON");
        assert_eq!(file.calls.len(), 1, "only this run's calls are recorded");
        assert_eq!(file.calls[0].response.content, "fresh");
    }

    #[tokio::test]
    async fn a_failed_fixture_write_does_not_lose_the_response() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture_path(&dir);
        let mut recording = one(mock("paid response"));
        wrap_providers(&mut recording, &FixtureMode::Record(path.clone())).unwrap();
        // Block the write: a directory at the fixture path makes fs::write fail.
        std::fs::create_dir(&path).unwrap();

        let resp = complete(&recording, "hi")
            .await
            .expect("a failed fixture write must not discard the response");
        assert_eq!(resp.content, "paid response");
        assert!(path.is_dir(), "the blocked path is left alone");
    }

    #[tokio::test]
    async fn replay_never_calls_the_inner_provider() {
        struct PanicProvider {
            caps: ProviderCapabilities,
        }

        #[async_trait]
        impl LLMProvider for PanicProvider {
            fn name(&self) -> &str {
                "mock"
            }
            fn capabilities(&self) -> &ProviderCapabilities {
                &self.caps
            }
            async fn complete(
                &self,
                _request: CompletionRequest,
            ) -> Result<CompletionResponse, ProviderError> {
                panic!("inner provider must not be called during replay")
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let path = fixture_path(&dir);

        let mut recording = one(mock("recorded offline"));
        wrap_providers(&mut recording, &FixtureMode::Record(path.clone())).unwrap();
        complete(&recording, "hi").await.unwrap();

        let mut replaying = one(Arc::new(PanicProvider {
            caps: ProviderCapabilities::default(),
        }) as BoxedProvider);
        wrap_providers(&mut replaying, &FixtureMode::Replay(path)).unwrap();
        assert_eq!(
            complete(&replaying, "hi").await.unwrap().content,
            "recorded offline"
        );
    }

    #[tokio::test]
    #[should_panic(expected = "inner provider must not be called during replay")]
    async fn control_panicking_provider_is_reached_when_recording() {
        // Control for the test above: the same panicking provider does blow up
        // when the wrapper forwards calls, so the replay assertion is meaningful.
        struct PanicProvider;

        #[async_trait]
        impl LLMProvider for PanicProvider {
            fn name(&self) -> &str {
                "mock"
            }
            fn capabilities(&self) -> &ProviderCapabilities {
                static CAPS: ProviderCapabilities = ProviderCapabilities {
                    max_context_tokens: 0,
                    quality_tier: crate::llm::QualityTier::Fast,
                    local: true,
                    cost_per_1k_input_tokens: 0.0,
                    cost_per_1k_output_tokens: 0.0,
                    supports_streaming: false,
                    supports_function_calling: false,
                    supports_json_mode: false,
                    max_output_tokens: 0,
                };
                &CAPS
            }
            async fn complete(
                &self,
                _request: CompletionRequest,
            ) -> Result<CompletionResponse, ProviderError> {
                panic!("inner provider must not be called during replay")
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut recording = one(Arc::new(PanicProvider) as BoxedProvider);
        wrap_providers(&mut recording, &FixtureMode::Record(fixture_path(&dir))).unwrap();
        let _ = complete(&recording, "hi").await;
    }

    #[tokio::test]
    async fn replay_preserves_recorded_confidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture_path(&dir);

        let mut recording = one(mock("I think it might be 42, but it depends"));
        wrap_providers(&mut recording, &FixtureMode::Record(path.clone())).unwrap();
        let live = complete(&recording, "answer?").await.unwrap();
        assert!(live.estimate_confidence() < 0.85, "control: hedged content");

        let mut replaying = one(mock("unused"));
        wrap_providers(&mut replaying, &FixtureMode::Replay(path)).unwrap();
        let replayed = complete(&replaying, "answer?").await.unwrap();

        assert_eq!(replayed.estimate_confidence(), live.estimate_confidence());
    }

    #[tokio::test]
    async fn replay_of_a_missing_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture_path(&dir);
        let mut replaying = one(mock("unused"));
        let err = wrap_providers(&mut replaying, &FixtureMode::Replay(path.clone()))
            .expect_err("a missing fixture file is a hard error");
        assert!(err.to_string().contains(path.to_str().unwrap()), "{err}");
    }

    #[test]
    fn default_path_sits_beside_the_program() {
        assert_eq!(
            default_path(Path::new("examples/llm/classify.forge")),
            PathBuf::from("examples/llm/classify.forge.fixtures.json")
        );
    }

    #[test]
    fn key_ignores_sampling_knobs_but_not_tools_or_system() {
        let base = CompletionRequest::simple("p");
        let key = call_key("mock", &base);

        let hotter = CompletionRequest::simple("p").with_temperature(0.1);
        assert_eq!(
            call_key("mock", &hotter),
            key,
            "temperature is not part of the key"
        );

        let other_provider = call_key("other", &base);
        assert_ne!(other_provider, key);

        let with_system = base.clone().with_system("be terse");
        assert_ne!(call_key("mock", &with_system), key);

        let tool = crate::llm::ToolDefinition {
            name: "read_file".to_string(),
            description: String::new(),
            input_schema: serde_json::json!({}),
        };
        let with_tool = base.clone().with_tools(vec![tool]);
        assert_ne!(call_key("mock", &with_tool), key);
    }
}
