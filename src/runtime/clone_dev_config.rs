// clone-dev TOML config loader — issue #357 (T8.2)
//
// FORGE has no file-I/O or TOML-parse primitives, so the loader lives in
// Rust and is exposed to FORGE as a runtime intrinsic `config.load_clone_dev`
// (see executor.rs, next to `env.get`). The FORGE-visible contract is a pure
// call returning a CloneDevConfig record — the DoD of #357 described it as a
// "pure FORGE task"; this module is the honest equivalent given today's
// language surface. If FORGE ever grows `file.read` + `toml.parse` we can
// move the merge logic into .forge code without changing the surface.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Deserialize;

use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::executor::RuntimeError;

// ── Raw TOML schema ──────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct CloneDevConfigRaw {
    #[serde(default)]
    pub org: OrgSection,
    #[serde(default)]
    pub slack: SlackSection,
    #[serde(default)]
    pub github: GithubSection,
    #[serde(default)]
    pub labels: LabelsSection,
    #[serde(default)]
    pub llm: LlmSection,
    #[serde(default)]
    pub warden: WardenSection,
    #[serde(default)]
    pub budget: BudgetSection,
    #[serde(default)]
    pub gates: GatesSection,
    #[serde(default)]
    pub defaults: DefaultsSection,
    #[serde(default)]
    pub repos: HashMap<String, RepoOverride>,
}

#[derive(Debug, Deserialize, Default)]
pub struct OrgSection {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct SlackSection {
    #[serde(default)]
    pub bot_token_env: String,
    #[serde(default)]
    pub signing_secret_env: String,
    #[serde(default)]
    pub default_channel: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct GithubSection {
    #[serde(default)]
    pub token_env: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct LabelsSection {
    #[serde(default)]
    pub triage: Vec<String>,
    #[serde(default)]
    pub blocked: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LlmSection {
    #[serde(default)]
    pub routing: LlmRouting,
}

#[derive(Debug, Deserialize, Default)]
pub struct LlmRouting {
    #[serde(default)]
    pub fast: String,
    #[serde(default)]
    pub balanced: String,
    #[serde(default)]
    pub high: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct WardenSection {
    #[serde(default)]
    pub max_retries: Option<f64>,
    #[serde(default)]
    pub escalate_after_seconds: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct BudgetSection {
    #[serde(default)]
    pub per_task_usd: Option<f64>,
    #[serde(default)]
    pub per_hour_usd: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GatesSection {
    #[serde(default)]
    pub require_approval_for: Vec<String>,
    #[serde(default)]
    pub auto_approve_labels: Vec<String>,
}

// [defaults] carries the per-repo-style knobs whose values apply to every
// repo unless overridden in [repos."<owner>/<name>"].
#[derive(Debug, Deserialize, Default, Clone)]
pub struct DefaultsSection {
    #[serde(default)]
    pub slack_channel: String,
    #[serde(default)]
    pub test_cmd: String,
    #[serde(default)]
    pub model_override: String,
    #[serde(default)]
    pub budget_per_task_usd: Option<f64>,
    #[serde(default)]
    pub warden_max_retries: Option<f64>,
    #[serde(default)]
    pub labels_extra: Vec<String>,
}

// [repos."<owner>/<name>"]. Any scalar set on a per-repo block wins over
// the corresponding field in [defaults]; labels_extra concatenates
// (defaults first, then per-repo).
#[derive(Debug, Deserialize, Default)]
pub struct RepoOverride {
    #[serde(default)]
    pub slack_channel: Option<String>,
    #[serde(default)]
    pub test_cmd: Option<String>,
    #[serde(default)]
    pub model_override: Option<String>,
    #[serde(default)]
    pub budget_per_task_usd: Option<f64>,
    #[serde(default)]
    pub warden_max_retries: Option<f64>,
    #[serde(default)]
    pub labels_extra: Vec<String>,
}

// ── Resolved config (post-merge, env-vars resolved) ──────────────

#[derive(Debug, Clone)]
pub struct CloneDevConfig {
    pub org_name: String,
    pub slack_bot_token: String,
    pub slack_default_channel: String,
    pub slack_signing_secret: String,
    pub github_token: String,
    pub github_labels_triage: Vec<String>,
    pub github_labels_blocked: Vec<String>,
    pub llm_routing_fast: String,
    pub llm_routing_balanced: String,
    pub llm_routing_high: String,
    pub warden_max_retries: f64,
    pub warden_escalate_after_seconds: f64,
    pub budget_per_task_usd: f64,
    pub budget_per_hour_usd: f64,
    pub gates_require_approval_for: Vec<String>,
    pub gates_auto_approve_labels: Vec<String>,
    pub repos: Vec<ResolvedRepo>,
}

#[derive(Debug, Clone)]
pub struct ResolvedRepo {
    pub slug: String,
    pub slack_channel: String,
    pub test_cmd: String,
    pub model_override: String,
    pub budget_per_task_usd: f64,
    pub warden_max_retries: f64,
    pub labels_extra: Vec<String>,
}

// Sentinels exposed to FORGE as "inherit default" markers for numeric
// per-repo fields. The FORGE-side RepoConfig record uses the same
// convention (-1.0 = inherit). String fields use the empty string.
const INHERIT_F64: f64 = -1.0;
const DEFAULT_WARDEN_MAX_RETRIES: f64 = 3.0;
const DEFAULT_WARDEN_ESCALATE_AFTER_SEC: f64 = 3600.0;

impl CloneDevConfig {
    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        let raw: CloneDevConfigRaw =
            toml::from_str(s).map_err(|e| format!("invalid clone-dev TOML: {e}"))?;
        Ok(Self::from_raw(raw))
    }

    fn from_raw(raw: CloneDevConfigRaw) -> Self {
        let defaults = raw.defaults;

        let mut repos: Vec<ResolvedRepo> = raw
            .repos
            .into_iter()
            .map(|(slug, over)| ResolvedRepo {
                slug,
                slack_channel: over
                    .slack_channel
                    .unwrap_or_else(|| defaults.slack_channel.clone()),
                test_cmd: over.test_cmd.unwrap_or_else(|| defaults.test_cmd.clone()),
                model_override: over
                    .model_override
                    .unwrap_or_else(|| defaults.model_override.clone()),
                budget_per_task_usd: over
                    .budget_per_task_usd
                    .or(defaults.budget_per_task_usd)
                    .unwrap_or(INHERIT_F64),
                warden_max_retries: over
                    .warden_max_retries
                    .or(defaults.warden_max_retries)
                    .unwrap_or(INHERIT_F64),
                labels_extra: {
                    let mut v = defaults.labels_extra.clone();
                    v.extend(over.labels_extra);
                    v
                },
            })
            .collect();

        // Stable ordering makes test assertions and log output predictable.
        repos.sort_by(|a, b| a.slug.cmp(&b.slug));

        CloneDevConfig {
            org_name: raw.org.name,
            slack_bot_token: resolve_env(&raw.slack.bot_token_env),
            slack_default_channel: raw.slack.default_channel,
            slack_signing_secret: resolve_env(&raw.slack.signing_secret_env),
            github_token: resolve_env(&raw.github.token_env),
            github_labels_triage: raw.labels.triage,
            github_labels_blocked: raw.labels.blocked,
            llm_routing_fast: raw.llm.routing.fast,
            llm_routing_balanced: raw.llm.routing.balanced,
            llm_routing_high: raw.llm.routing.high,
            warden_max_retries: raw
                .warden
                .max_retries
                .unwrap_or(DEFAULT_WARDEN_MAX_RETRIES),
            warden_escalate_after_seconds: raw
                .warden
                .escalate_after_seconds
                .unwrap_or(DEFAULT_WARDEN_ESCALATE_AFTER_SEC),
            budget_per_task_usd: raw.budget.per_task_usd.unwrap_or(INHERIT_F64),
            budget_per_hour_usd: raw.budget.per_hour_usd.unwrap_or(INHERIT_F64),
            gates_require_approval_for: raw.gates.require_approval_for,
            gates_auto_approve_labels: raw.gates.auto_approve_labels,
            repos,
        }
    }

    // Build the FORGE-facing Value matching the CloneDevConfig record
    // shape in workflows/clone-dev/shared/types.forge. Field names and
    // order below must stay in sync with that declaration.
    pub fn to_forge_record(&self) -> Value {
        let mut fields = HashMap::new();
        insert_text(&mut fields, "org_name", &self.org_name);
        insert_text(&mut fields, "slack_bot_token", &self.slack_bot_token);
        insert_text(
            &mut fields,
            "slack_default_channel",
            &self.slack_default_channel,
        );
        insert_text(
            &mut fields,
            "slack_signing_secret",
            &self.slack_signing_secret,
        );
        insert_text(&mut fields, "github_token", &self.github_token);
        insert_text_array(
            &mut fields,
            "github_labels_triage",
            &self.github_labels_triage,
        );
        insert_text_array(
            &mut fields,
            "github_labels_blocked",
            &self.github_labels_blocked,
        );
        insert_text(&mut fields, "llm_routing_fast", &self.llm_routing_fast);
        insert_text(
            &mut fields,
            "llm_routing_balanced",
            &self.llm_routing_balanced,
        );
        insert_text(&mut fields, "llm_routing_high", &self.llm_routing_high);
        insert_number(&mut fields, "warden_max_retries", self.warden_max_retries);
        insert_number(
            &mut fields,
            "warden_escalate_after_seconds",
            self.warden_escalate_after_seconds,
        );
        insert_number(
            &mut fields,
            "budget_per_task_usd",
            self.budget_per_task_usd,
        );
        insert_number(
            &mut fields,
            "budget_per_hour_usd",
            self.budget_per_hour_usd,
        );
        insert_text_array(
            &mut fields,
            "gates_require_approval_for",
            &self.gates_require_approval_for,
        );
        insert_text_array(
            &mut fields,
            "gates_auto_approve_labels",
            &self.gates_auto_approve_labels,
        );
        let repo_records: Vec<ConfidentValue> = self.repos.iter().map(repo_to_record).collect();
        fields.insert(
            "repos".into(),
            ConfidentValue::deterministic(Value::Array(repo_records)),
        );
        Value::Record(fields)
    }
}

fn resolve_env(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    std::env::var(name).unwrap_or_default()
}

fn insert_text(fields: &mut HashMap<String, ConfidentValue>, key: &str, s: &str) {
    fields.insert(
        key.into(),
        ConfidentValue::deterministic(Value::Text(s.into())),
    );
}

fn insert_number(fields: &mut HashMap<String, ConfidentValue>, key: &str, n: f64) {
    fields.insert(key.into(), ConfidentValue::deterministic(Value::Number(n)));
}

fn insert_text_array(fields: &mut HashMap<String, ConfidentValue>, key: &str, items: &[String]) {
    let arr: Vec<ConfidentValue> = items
        .iter()
        .map(|s| ConfidentValue::deterministic(Value::Text(s.clone())))
        .collect();
    fields.insert(
        key.into(),
        ConfidentValue::deterministic(Value::Array(arr)),
    );
}

fn repo_to_record(r: &ResolvedRepo) -> ConfidentValue {
    let mut fields = HashMap::new();
    insert_text(&mut fields, "slug", &r.slug);
    insert_text(&mut fields, "slack_channel", &r.slack_channel);
    insert_text(&mut fields, "test_cmd", &r.test_cmd);
    insert_text(&mut fields, "model_override", &r.model_override);
    insert_number(&mut fields, "budget_per_task_usd", r.budget_per_task_usd);
    insert_number(&mut fields, "warden_max_retries", r.warden_max_retries);
    insert_text_array(&mut fields, "labels_extra", &r.labels_extra);
    ConfidentValue::deterministic(Value::Record(fields))
}

// ── Process-wide cache ──────────────────────────────────────────
//
// Agents call the intrinsic in their own `on start` handlers; the
// cache means N agents × M restarts parse the file once per unique
// absolute path.

fn cache() -> &'static Mutex<HashMap<PathBuf, Arc<CloneDevConfig>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<CloneDevConfig>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Load config from `path`, using a process-wide cache keyed on the
/// canonicalized absolute path. Repeated callers for the same file
/// share a single parse and env-var resolution snapshot.
pub fn load(path: &Path) -> Result<Arc<CloneDevConfig>, String> {
    let canon = std::fs::canonicalize(path)
        .map_err(|e| format!("cannot resolve config path '{}': {e}", path.display()))?;
    let mut guard = cache()
        .lock()
        .map_err(|e| format!("config cache poisoned: {e}"))?;
    if let Some(cfg) = guard.get(&canon) {
        return Ok(cfg.clone());
    }
    let content = std::fs::read_to_string(&canon)
        .map_err(|e| format!("cannot read config '{}': {e}", canon.display()))?;
    let cfg = Arc::new(CloneDevConfig::from_toml_str(&content)?);
    guard.insert(canon, cfg.clone());
    Ok(cfg)
}

/// Executor-facing entry: returns a ConfidentValue wrapping the
/// FORGE record, ready to assign into `memory.config`.
pub fn load_as_forge_value(path: &Path) -> Result<ConfidentValue, RuntimeError> {
    let cfg = load(path).map_err(RuntimeError::FlowError)?;
    Ok(ConfidentValue::deterministic(cfg.to_forge_record()))
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config_with_defaults() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [org]
            name = "ncmlabs"
            "#,
        )
        .expect("parse");
        assert_eq!(cfg.org_name, "ncmlabs");
        assert_eq!(cfg.warden_max_retries, DEFAULT_WARDEN_MAX_RETRIES);
        assert_eq!(
            cfg.warden_escalate_after_seconds,
            DEFAULT_WARDEN_ESCALATE_AFTER_SEC
        );
        assert!(cfg.repos.is_empty());
    }

    #[test]
    fn parses_all_sections() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [org]
            name = "ncmlabs"

            [slack]
            default_channel = "C_default"

            [github]
            # no token_env set — should resolve to empty

            [labels]
            triage  = ["needs-triage"]
            blocked = ["blocked"]

            [llm.routing]
            fast     = "claude-haiku"
            balanced = "claude-haiku"
            high     = "claude-opus"

            [warden]
            max_retries = 5
            escalate_after_seconds = 7200

            [budget]
            per_task_usd = 2.5
            per_hour_usd = 20.0

            [gates]
            require_approval_for = ["release", "destructive"]
            auto_approve_labels  = ["docs-only"]
            "#,
        )
        .expect("parse");
        assert_eq!(cfg.org_name, "ncmlabs");
        assert_eq!(cfg.slack_default_channel, "C_default");
        assert_eq!(cfg.github_token, "");
        assert_eq!(cfg.github_labels_triage, vec!["needs-triage".to_string()]);
        assert_eq!(cfg.llm_routing_high, "claude-opus");
        assert_eq!(cfg.warden_max_retries, 5.0);
        assert_eq!(cfg.warden_escalate_after_seconds, 7200.0);
        assert_eq!(cfg.budget_per_task_usd, 2.5);
        assert_eq!(cfg.budget_per_hour_usd, 20.0);
        assert_eq!(cfg.gates_require_approval_for.len(), 2);
        assert_eq!(cfg.gates_auto_approve_labels, vec!["docs-only".to_string()]);
    }

    #[test]
    fn per_repo_scalars_win_over_defaults() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [defaults]
            slack_channel       = "C_default"
            test_cmd            = "cargo test"
            model_override      = "claude-haiku"
            budget_per_task_usd = 1.0
            warden_max_retries  = 3

            [repos."acme/alpha"]
            slack_channel       = "C_alpha"
            budget_per_task_usd = 5.0

            [repos."acme/beta"]
            # no scalars — inherits everything from defaults
            "#,
        )
        .expect("parse");
        let alpha = cfg
            .repos
            .iter()
            .find(|r| r.slug == "acme/alpha")
            .expect("alpha present");
        let beta = cfg
            .repos
            .iter()
            .find(|r| r.slug == "acme/beta")
            .expect("beta present");

        // scalar-wins
        assert_eq!(alpha.slack_channel, "C_alpha");
        assert_eq!(alpha.budget_per_task_usd, 5.0);
        // inherits defaults when override absent
        assert_eq!(alpha.test_cmd, "cargo test");
        assert_eq!(alpha.model_override, "claude-haiku");
        assert_eq!(alpha.warden_max_retries, 3.0);

        // beta inherits every scalar
        assert_eq!(beta.slack_channel, "C_default");
        assert_eq!(beta.test_cmd, "cargo test");
        assert_eq!(beta.budget_per_task_usd, 1.0);
        assert_eq!(beta.warden_max_retries, 3.0);
    }

    #[test]
    fn per_repo_arrays_concat_with_defaults() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [defaults]
            labels_extra = ["auto-forge", "priority:normal"]

            [repos."acme/alpha"]
            labels_extra = ["alpha-only"]

            [repos."acme/beta"]
            labels_extra = []
            "#,
        )
        .expect("parse");
        let alpha = cfg
            .repos
            .iter()
            .find(|r| r.slug == "acme/alpha")
            .unwrap();
        let beta = cfg.repos.iter().find(|r| r.slug == "acme/beta").unwrap();
        // defaults first, per-repo concatenated after
        assert_eq!(
            alpha.labels_extra,
            vec![
                "auto-forge".to_string(),
                "priority:normal".to_string(),
                "alpha-only".to_string()
            ]
        );
        // empty per-repo array still concatenates (no-op)
        assert_eq!(
            beta.labels_extra,
            vec!["auto-forge".to_string(), "priority:normal".to_string()]
        );
    }

    #[test]
    fn missing_per_repo_fields_sentinel_to_inherit() {
        // No [defaults] block and no per-repo overrides for numeric fields:
        // we expect the INHERIT_F64 sentinel so the FORGE side can fall
        // back to org-wide defaults without ambiguity.
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [repos."acme/orphan"]
            "#,
        )
        .expect("parse");
        let orphan = cfg.repos.first().expect("one repo");
        assert_eq!(orphan.budget_per_task_usd, INHERIT_F64);
        assert_eq!(orphan.warden_max_retries, INHERIT_F64);
        assert_eq!(orphan.slack_channel, "");
        assert_eq!(orphan.test_cmd, "");
    }

    #[test]
    fn env_var_indirection_resolves_at_parse_time() {
        // Use a scoped var name that's unlikely to collide.
        std::env::set_var("FORGE_T357_TEST_SLACK_TOKEN", "xoxb-fixture");
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [slack]
            bot_token_env = "FORGE_T357_TEST_SLACK_TOKEN"
            "#,
        )
        .expect("parse");
        std::env::remove_var("FORGE_T357_TEST_SLACK_TOKEN");
        assert_eq!(cfg.slack_bot_token, "xoxb-fixture");
    }

    #[test]
    fn env_var_indirection_missing_var_yields_empty_string() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [slack]
            bot_token_env = "FORGE_T357_DEFINITELY_UNSET"
            "#,
        )
        .expect("parse");
        assert_eq!(cfg.slack_bot_token, "");
    }

    #[test]
    fn invalid_toml_returns_clean_error() {
        let err = CloneDevConfig::from_toml_str("this is not = valid = toml")
            .expect_err("should reject");
        assert!(
            err.contains("invalid clone-dev TOML"),
            "error should mention clone-dev TOML, got: {err}"
        );
    }

    #[test]
    fn to_forge_record_emits_expected_fields() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [org]
            name = "ncmlabs"

            [slack]
            default_channel = "C_default"

            [defaults]
            test_cmd = "cargo test"

            [repos."acme/alpha"]
            slack_channel = "C_alpha"
            "#,
        )
        .expect("parse");
        let record = cfg.to_forge_record();
        let fields = match record {
            Value::Record(ref f) => f,
            _ => panic!("expected Record"),
        };
        // Top-level scalars present
        assert!(fields.contains_key("org_name"));
        assert!(fields.contains_key("slack_default_channel"));
        assert!(fields.contains_key("warden_max_retries"));
        // repos array present and shaped
        let repos = fields.get("repos").expect("repos field");
        match &repos.value {
            Value::Array(items) => {
                assert_eq!(items.len(), 1);
                match &items[0].value {
                    Value::Record(r) => {
                        assert!(r.contains_key("slug"));
                        assert!(r.contains_key("slack_channel"));
                        assert!(r.contains_key("labels_extra"));
                    }
                    _ => panic!("repo item should be a Record"),
                }
            }
            _ => panic!("repos should be an Array"),
        }
    }

    #[test]
    fn load_caches_by_canonical_path() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("forge-t357-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("clone-dev.toml");
        {
            let mut f = std::fs::File::create(&path).expect("create");
            writeln!(f, "[org]\nname = \"cache-test\"").expect("write");
        }
        let a = load(&path).expect("load 1");
        // Mutate the file; a cached load should still return the original.
        {
            let mut f = std::fs::File::create(&path).expect("rewrite");
            writeln!(f, "[org]\nname = \"should-not-appear\"").expect("write");
        }
        let b = load(&path).expect("load 2");
        assert_eq!(a.org_name, "cache-test");
        assert_eq!(b.org_name, "cache-test");
        assert!(Arc::ptr_eq(&a, &b), "cache should return same Arc");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_returns_clean_error_for_missing_file() {
        let missing = std::path::PathBuf::from("/nonexistent/forge-t357/not-a-file.toml");
        let err = load(&missing).expect_err("missing file should error");
        assert!(err.contains("cannot resolve config path"));
    }
}
