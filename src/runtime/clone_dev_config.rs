// clone-dev TOML config loader — issue #357 (T8.2)
//
// FORGE has no file-I/O or TOML-parse primitives, so the loader lives in
// Rust and is exposed to FORGE as a runtime intrinsic `config.load_clone_dev`
// (see executor.rs, next to `env.get`). The FORGE-visible contract is a pure
// call returning a CloneDevConfig record — the DoD of #357 described it as a
// "pure FORGE task"; this module is the honest equivalent given today's
// language surface. If FORGE ever grows `file.read` + `toml.parse` we can
// move the merge logic into .forge code without changing the surface.

use std::collections::{BTreeMap, HashMap};
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
    // T9.5 (#366) — channel ID the slack_devops_monitor watches for
    // @-mentions. Channel IDs (`C…`) are non-secret, so this is a raw
    // value rather than an env-indirection.
    #[serde(default)]
    pub devops_channel: String,
    // T10.2 (#368) — channel ID gate_two posts plan-approval cards into.
    // Empty string ⇒ fall back to the issue's per-thread channel from
    // PlanReady.channel (multi-channel orgs); set to a single ID to
    // funnel every plan approval into one reviewer channel.
    #[serde(default)]
    pub approval_channel: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct GithubSection {
    #[serde(default)]
    pub token_env: String,
    // T10.1 (#367) — repo slug used by gate_one when forking a
    // ProposalReady(kind=propose_issue) into ProposalApproved. Single-repo
    // assumption for v1; future work can promote to a per-thread repo
    // resolver.
    #[serde(default)]
    pub default_repo: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct LabelsSection {
    #[serde(default)]
    pub triage: Vec<String>,
    #[serde(default)]
    pub blocked: Vec<String>,
    // T8.3 (#358) — Deterministic routing.
    // BTreeMap so the parallel suffix/target arrays we expose to FORGE
    // come out in stable, predictable order regardless of TOML key order.
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub triage_target: String,
    #[serde(default)]
    pub routing: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LlmSection {
    // T8.6 (#361) — routing is now a free-form phase→provider table with an
    // optional `fallback` sub-table for chains. Parsing as a raw `toml::Table`
    // lets us accept arbitrary phase keys (`classify`, `plan`, `implement`,
    // `review`, `ops_investigate`, plus the legacy `fast/balanced/high`)
    // without locking the schema. Validation happens in `from_raw`.
    #[serde(default)]
    pub routing: toml::value::Table,
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
    // T10.1 (#367) — Gate-1 (create-issue) toggles. `create_issue =
    // false` makes gate_one auto-approve every propose_issue proposal
    // without going through Slack. `create_issue_timeout_mins` is the
    // window after which a pending approval escalates via warden.
    #[serde(default)]
    pub create_issue: Option<bool>,
    #[serde(default)]
    pub create_issue_timeout_mins: Option<f64>,
    // T10.2 (#368) — Gate-2 (start-implementation) toggles. Same shape
    // as create_issue: `start_implementation = false` makes gate_two
    // auto-approve every PlanReady without going through Slack;
    // `start_implementation_timeout_mins` drives the escalation
    // tick counter.
    #[serde(default)]
    pub start_implementation: Option<bool>,
    #[serde(default)]
    pub start_implementation_timeout_mins: Option<f64>,
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
    // T8.5 (#360) — dev-cycle template knobs. Empty string ⇒ apply
    // built-in default (constants below); per-repo overrides win when
    // non-empty / Some.
    #[serde(default)]
    pub workdir_root: String,
    #[serde(default)]
    pub branch_prefix: String,
    #[serde(default)]
    pub commit_template: String,
    #[serde(default)]
    pub fix_commit_template: String,
    #[serde(default)]
    pub max_iterations: Option<f64>,
    #[serde(default)]
    pub auto_approve: Option<bool>,
    // T10.2 (#368) — bound on planner re-plan attempts triggered by
    // gate_two's ImplementationRejected feedback loop. Per-repo
    // override available below; `None` ⇒ use built-in default.
    #[serde(default)]
    pub max_plan_revisions: Option<f64>,
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
    // T8.5 (#360) — per-repo overrides. None ⇒ inherit defaults.
    #[serde(default)]
    pub workdir_root: Option<String>,
    #[serde(default)]
    pub branch_prefix: Option<String>,
    #[serde(default)]
    pub commit_template: Option<String>,
    #[serde(default)]
    pub fix_commit_template: Option<String>,
    #[serde(default)]
    pub max_iterations: Option<f64>,
    #[serde(default)]
    pub auto_approve: Option<bool>,
    // T10.2 (#368) — per-repo override of [defaults].max_plan_revisions.
    #[serde(default)]
    pub max_plan_revisions: Option<f64>,
}

// ── Resolved config (post-merge, env-vars resolved) ──────────────

#[derive(Debug, Clone)]
pub struct CloneDevConfig {
    pub org_name: String,
    pub slack_bot_token: String,
    pub slack_default_channel: String,
    // T9.5 (#366) — channel ID polled by slack_devops_monitor for
    // inbound DevOps mentions.
    pub slack_devops_channel: String,
    pub slack_signing_secret: String,
    pub github_token: String,
    pub github_labels_triage: Vec<String>,
    pub github_labels_blocked: Vec<String>,
    // Back-compat with T8.2 (#357) — these three keys remain part of the
    // FORGE record schema declared in workflows/clone-dev/shared/types.forge.
    // Post-#361 they are pulled from the same `[llm.routing]` map as any
    // other phase key, so authoring `fast = "x"` and authoring `plan = "y"`
    // sit side-by-side in TOML.
    pub llm_routing_fast: String,
    pub llm_routing_balanced: String,
    pub llm_routing_high: String,
    // T8.6 (#361) — the full phase→provider primary table and the optional
    // phase→fallback-chain table. Used by the runtime to overlay routing
    // onto the ProviderRegistry; not currently surfaced into the FORGE
    // record (the executor consults the registry directly per `reason
    // "..." for <phase>`).
    pub llm_routing: HashMap<String, String>,
    pub llm_routing_fallback: HashMap<String, Vec<String>>,
    pub warden_max_retries: f64,
    pub warden_escalate_after_seconds: f64,
    pub budget_per_task_usd: f64,
    pub budget_per_hour_usd: f64,
    pub gates_require_approval_for: Vec<String>,
    pub gates_auto_approve_labels: Vec<String>,
    // T10.1 (#367) — gate_one toggles. `gates_create_issue` defaults to
    // true (gate is on); flip to false to auto-approve every
    // propose_issue without going through Slack. The timeout is in
    // minutes; gate_one ticks a 5-minute timer and escalates when the
    // counter reaches the configured value.
    pub gates_create_issue: bool,
    pub gates_create_issue_timeout_mins: f64,
    // T10.2 (#368) — gate_two toggles, mirroring create_issue semantics.
    // `gates_start_implementation = false` ⇒ gate_two auto-emits
    // ImplementationApproved with `decision_by = "auto (policy)"`.
    pub gates_start_implementation: bool,
    pub gates_start_implementation_timeout_mins: f64,
    // T10.2 (#368) — channel ID gate_two posts plan-approval cards into.
    // Empty ⇒ fall back to PlanReady.channel.
    pub slack_approval_channel: String,
    // T10.1 (#367) — single-repo destination for gate_one's
    // propose_issue → ProposalApproved fork. Empty string means no
    // default; gate_one falls back to skipping the issue creation step.
    pub github_default_repo: String,
    // T8.3 (#358) — flat sibling fields. FORGE record-of-record is
    // untested today (see workflows/clone-dev/shared/types.forge:154–158);
    // suffixes/targets are parallel arrays so a pure FORGE task can
    // rebuild a typed LabelRouting record from these scalars.
    pub label_routing_namespace: String,
    pub label_routing_suffixes: Vec<String>,
    pub label_routing_targets: Vec<String>,
    pub label_routing_triage_target: String,
    // T8.5 (#360) — dev-cycle template defaults. Per-repo overrides
    // resolve into ResolvedRepo (below). agents.forge consults the
    // resolved RepoConfig, which falls back to these scalars via
    // repo_config_for in shared/types.forge.
    pub defaults_workdir_root: String,
    pub defaults_branch_prefix: String,
    pub defaults_commit_template: String,
    pub defaults_fix_commit_template: String,
    pub defaults_max_iterations: f64,
    pub defaults_auto_approve: bool,
    // T10.2 (#368) — bound on planner re-plan attempts. Per-repo
    // override resolves into ResolvedRepo.max_plan_revisions.
    pub defaults_max_plan_revisions: f64,
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
    // T8.5 (#360) — per-repo dev-cycle templates, resolved by merging
    // RepoOverride against DefaultsSection (and built-in constants when
    // both are empty). Empty Text ⇒ "inherit"; FORGE side uses the
    // default scalars from CloneDevConfig in that case.
    pub workdir_root: String,
    pub branch_prefix: String,
    pub commit_template: String,
    pub fix_commit_template: String,
    pub max_iterations: f64,
    pub auto_approve: bool,
    // T10.2 (#368) — per-repo override of defaults_max_plan_revisions.
    pub max_plan_revisions: f64,
}

// Sentinels exposed to FORGE as "inherit default" markers for numeric
// per-repo fields. The FORGE-side RepoConfig record uses the same
// convention (-1.0 = inherit). String fields use the empty string.
const INHERIT_F64: f64 = -1.0;
const DEFAULT_WARDEN_MAX_RETRIES: f64 = 3.0;
const DEFAULT_WARDEN_ESCALATE_AFTER_SEC: f64 = 3600.0;
// T8.3 (#358) — back-compat default for older TOML files that pre-date
// the [labels.routing] block. The label_router pure task uses this
// name when emitting fall-through routes.
const DEFAULT_TRIAGE_TARGET: &str = "triage_specialist";
// T8.5 (#360) — built-in defaults for dev-cycle templating knobs. Used
// when neither [defaults] nor [repos."*"] supplies a value. The literal
// strings preserve the historical hardcoded behavior in agents.forge so
// pre-T8.5 TOML files keep working.
const DEFAULT_WORKDIR_ROOT: &str = "/tmp/forge-workdir";
const DEFAULT_BRANCH_PREFIX: &str = "clone-dev";
const DEFAULT_COMMIT_TEMPLATE: &str = "feat({issue_id}): implement per plan";
const DEFAULT_FIX_COMMIT_TEMPLATE: &str = "fix({issue_id}): iteration {iteration}";
const DEFAULT_MAX_ITERATIONS: f64 = 3.0;
// T10.1 (#367) — Gate-1 (create-issue) defaults. Gate is on by default
// (operators must opt out via `[gates] create_issue = false`); the 30m
// window matches the dev-cycle reviewer's typical approval cadence.
const DEFAULT_GATES_CREATE_ISSUE: bool = true;
const DEFAULT_GATES_CREATE_ISSUE_TIMEOUT_MINS: f64 = 30.0;
// T10.2 (#368) — Gate-2 (start-implementation) defaults. Same shape
// and rationale as Gate-1: opt-out via `[gates] start_implementation
// = false`, 30-minute escalation window.
const DEFAULT_GATES_START_IMPLEMENTATION: bool = true;
const DEFAULT_GATES_START_IMPLEMENTATION_TIMEOUT_MINS: f64 = 30.0;
// T10.2 (#368) — bound on planner re-plan attempts before escalating.
// 3 mirrors the implementer's max_iterations default — a planner that
// cannot satisfy a reviewer in 3 tries needs human attention.
const DEFAULT_MAX_PLAN_REVISIONS: f64 = 3.0;

impl CloneDevConfig {
    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        let raw: CloneDevConfigRaw =
            toml::from_str(s).map_err(|e| format!("invalid clone-dev TOML: {e}"))?;
        Ok(Self::from_raw(raw))
    }

    fn from_raw(raw: CloneDevConfigRaw) -> Self {
        let defaults = raw.defaults;

        // ── T8.6 (#361) — split [llm.routing] into a primary phase→provider
        // map and an optional phase→fallback-chain table.
        //
        // Authored shape:
        //   [llm.routing]
        //   plan      = "sonnet"
        //   implement = "gpt-4o"
        //   [llm.routing.fallback]
        //   plan      = ["sonnet", "gpt-4o"]
        //
        // Anything that isn't a string-valued top-level key or the named
        // `fallback` sub-table is silently skipped. We don't error on
        // malformed entries so older/newer authored configs degrade
        // gracefully instead of crashing the runtime — the warden surfaces
        // unresolvable provider names later.
        let (llm_routing, llm_routing_fallback) = parse_llm_routing(&raw.llm.routing);

        // BTreeMap iteration is deterministic — suffixes[i] always pairs
        // with targets[i] regardless of TOML authoring order.
        let (label_routing_suffixes, label_routing_targets): (Vec<_>, Vec<_>) =
            raw.labels.routing.into_iter().unzip();
        let label_routing_triage_target = if raw.labels.triage_target.is_empty() {
            DEFAULT_TRIAGE_TARGET.to_string()
        } else {
            raw.labels.triage_target
        };

        // T8.5 — resolve [defaults] templating fields once so per-repo
        // merge has a single fallback layer. Empty TOML strings collapse
        // to the built-in constants here; per-repo None then falls back
        // to these resolved defaults.
        let defaults_workdir_root = if defaults.workdir_root.is_empty() {
            DEFAULT_WORKDIR_ROOT.to_string()
        } else {
            defaults.workdir_root.clone()
        };
        let defaults_branch_prefix = if defaults.branch_prefix.is_empty() {
            DEFAULT_BRANCH_PREFIX.to_string()
        } else {
            defaults.branch_prefix.clone()
        };
        let defaults_commit_template = if defaults.commit_template.is_empty() {
            DEFAULT_COMMIT_TEMPLATE.to_string()
        } else {
            defaults.commit_template.clone()
        };
        let defaults_fix_commit_template = if defaults.fix_commit_template.is_empty() {
            DEFAULT_FIX_COMMIT_TEMPLATE.to_string()
        } else {
            defaults.fix_commit_template.clone()
        };
        let defaults_max_iterations = defaults.max_iterations.unwrap_or(DEFAULT_MAX_ITERATIONS);
        let defaults_auto_approve = defaults.auto_approve.unwrap_or(false);
        let defaults_max_plan_revisions = defaults
            .max_plan_revisions
            .unwrap_or(DEFAULT_MAX_PLAN_REVISIONS);

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
                workdir_root: over
                    .workdir_root
                    .unwrap_or_else(|| defaults_workdir_root.clone()),
                branch_prefix: over
                    .branch_prefix
                    .unwrap_or_else(|| defaults_branch_prefix.clone()),
                commit_template: over
                    .commit_template
                    .unwrap_or_else(|| defaults_commit_template.clone()),
                fix_commit_template: over
                    .fix_commit_template
                    .unwrap_or_else(|| defaults_fix_commit_template.clone()),
                max_iterations: over.max_iterations.unwrap_or(defaults_max_iterations),
                auto_approve: over.auto_approve.unwrap_or(defaults_auto_approve),
                max_plan_revisions: over
                    .max_plan_revisions
                    .unwrap_or(defaults_max_plan_revisions),
            })
            .collect();

        // Stable ordering makes test assertions and log output predictable.
        repos.sort_by(|a, b| a.slug.cmp(&b.slug));

        CloneDevConfig {
            org_name: raw.org.name,
            slack_bot_token: resolve_env(&raw.slack.bot_token_env),
            slack_default_channel: raw.slack.default_channel,
            slack_devops_channel: raw.slack.devops_channel,
            slack_signing_secret: resolve_env(&raw.slack.signing_secret_env),
            github_token: resolve_env(&raw.github.token_env),
            github_labels_triage: raw.labels.triage,
            github_labels_blocked: raw.labels.blocked,
            llm_routing_fast: llm_routing.get("fast").cloned().unwrap_or_default(),
            llm_routing_balanced: llm_routing.get("balanced").cloned().unwrap_or_default(),
            llm_routing_high: llm_routing.get("high").cloned().unwrap_or_default(),
            llm_routing,
            llm_routing_fallback,
            warden_max_retries: raw.warden.max_retries.unwrap_or(DEFAULT_WARDEN_MAX_RETRIES),
            warden_escalate_after_seconds: raw
                .warden
                .escalate_after_seconds
                .unwrap_or(DEFAULT_WARDEN_ESCALATE_AFTER_SEC),
            budget_per_task_usd: raw.budget.per_task_usd.unwrap_or(INHERIT_F64),
            budget_per_hour_usd: raw.budget.per_hour_usd.unwrap_or(INHERIT_F64),
            gates_require_approval_for: raw.gates.require_approval_for,
            gates_auto_approve_labels: raw.gates.auto_approve_labels,
            gates_create_issue: raw.gates.create_issue.unwrap_or(DEFAULT_GATES_CREATE_ISSUE),
            gates_create_issue_timeout_mins: raw
                .gates
                .create_issue_timeout_mins
                .unwrap_or(DEFAULT_GATES_CREATE_ISSUE_TIMEOUT_MINS),
            gates_start_implementation: raw
                .gates
                .start_implementation
                .unwrap_or(DEFAULT_GATES_START_IMPLEMENTATION),
            gates_start_implementation_timeout_mins: raw
                .gates
                .start_implementation_timeout_mins
                .unwrap_or(DEFAULT_GATES_START_IMPLEMENTATION_TIMEOUT_MINS),
            slack_approval_channel: raw.slack.approval_channel,
            github_default_repo: raw.github.default_repo,
            label_routing_namespace: raw.labels.namespace,
            label_routing_suffixes,
            label_routing_targets,
            label_routing_triage_target,
            defaults_workdir_root,
            defaults_branch_prefix,
            defaults_commit_template,
            defaults_fix_commit_template,
            defaults_max_iterations,
            defaults_auto_approve,
            defaults_max_plan_revisions,
            repos,
        }
    }

    /// Resolve the ordered provider chain for a routing phase (#361).
    /// Order: `[primary, ...fallback_chain]`. Returns an empty chain when
    /// the phase has no entry — callers should fall back to runtime defaults.
    pub fn routing(&self, phase: &str) -> Vec<String> {
        let mut chain = Vec::new();
        if let Some(primary) = self.llm_routing.get(phase) {
            chain.push(primary.clone());
        }
        if let Some(fallbacks) = self.llm_routing_fallback.get(phase) {
            for name in fallbacks {
                if !chain.contains(name) {
                    chain.push(name.clone());
                }
            }
        }
        chain
    }

    /// Materialize every configured phase into a `phase → chain` table for
    /// the ProviderRegistry overlay (#361). Phases that only appear in the
    /// fallback table (no primary) are still emitted so explicit fallback-
    /// only configs still route somewhere.
    pub fn routing_table(&self) -> HashMap<String, Vec<String>> {
        let mut keys: std::collections::BTreeSet<String> =
            self.llm_routing.keys().cloned().collect();
        keys.extend(self.llm_routing_fallback.keys().cloned());
        keys.into_iter()
            .map(|phase| {
                let chain = self.routing(&phase);
                (phase, chain)
            })
            .filter(|(_, chain)| !chain.is_empty())
            .collect()
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
            "slack_devops_channel",
            &self.slack_devops_channel,
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
        insert_number(&mut fields, "budget_per_task_usd", self.budget_per_task_usd);
        insert_number(&mut fields, "budget_per_hour_usd", self.budget_per_hour_usd);
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
        insert_bool(&mut fields, "gates_create_issue", self.gates_create_issue);
        insert_number(
            &mut fields,
            "gates_create_issue_timeout_mins",
            self.gates_create_issue_timeout_mins,
        );
        insert_bool(
            &mut fields,
            "gates_start_implementation",
            self.gates_start_implementation,
        );
        insert_number(
            &mut fields,
            "gates_start_implementation_timeout_mins",
            self.gates_start_implementation_timeout_mins,
        );
        insert_text(
            &mut fields,
            "slack_approval_channel",
            &self.slack_approval_channel,
        );
        insert_text(
            &mut fields,
            "github_default_repo",
            &self.github_default_repo,
        );
        insert_text(
            &mut fields,
            "label_routing_namespace",
            &self.label_routing_namespace,
        );
        insert_text_array(
            &mut fields,
            "label_routing_suffixes",
            &self.label_routing_suffixes,
        );
        insert_text_array(
            &mut fields,
            "label_routing_targets",
            &self.label_routing_targets,
        );
        insert_text(
            &mut fields,
            "label_routing_triage_target",
            &self.label_routing_triage_target,
        );
        insert_text(
            &mut fields,
            "defaults_workdir_root",
            &self.defaults_workdir_root,
        );
        insert_text(
            &mut fields,
            "defaults_branch_prefix",
            &self.defaults_branch_prefix,
        );
        insert_text(
            &mut fields,
            "defaults_commit_template",
            &self.defaults_commit_template,
        );
        insert_text(
            &mut fields,
            "defaults_fix_commit_template",
            &self.defaults_fix_commit_template,
        );
        insert_number(
            &mut fields,
            "defaults_max_iterations",
            self.defaults_max_iterations,
        );
        insert_bool(
            &mut fields,
            "defaults_auto_approve",
            self.defaults_auto_approve,
        );
        insert_number(
            &mut fields,
            "defaults_max_plan_revisions",
            self.defaults_max_plan_revisions,
        );
        let repo_records: Vec<ConfidentValue> = self.repos.iter().map(repo_to_record).collect();
        fields.insert(
            "repos".into(),
            ConfidentValue::deterministic(Value::Array(repo_records)),
        );
        Value::Record(fields)
    }
}

/// Split `[llm.routing]` raw TOML into (primary, fallback) maps (#361).
/// Top-level string entries become `primary[phase] = provider_name`. The
/// nested `fallback` sub-table, when present, is read as `phase → [provider,
/// ...]`. Non-conforming values are skipped without erroring.
fn parse_llm_routing(
    table: &toml::value::Table,
) -> (HashMap<String, String>, HashMap<String, Vec<String>>) {
    let mut primary = HashMap::new();
    let mut fallback = HashMap::new();
    for (k, v) in table {
        if k == "fallback" {
            if let Some(sub) = v.as_table() {
                for (phase, chain) in sub {
                    if let Some(arr) = chain.as_array() {
                        let names: Vec<String> = arr
                            .iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect();
                        if !names.is_empty() {
                            fallback.insert(phase.clone(), names);
                        }
                    }
                }
            }
            continue;
        }
        if let Some(name) = v.as_str() {
            primary.insert(k.clone(), name.to_string());
        }
    }
    (primary, fallback)
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

fn insert_bool(fields: &mut HashMap<String, ConfidentValue>, key: &str, b: bool) {
    fields.insert(key.into(), ConfidentValue::deterministic(Value::Bool(b)));
}

fn insert_text_array(fields: &mut HashMap<String, ConfidentValue>, key: &str, items: &[String]) {
    let arr: Vec<ConfidentValue> = items
        .iter()
        .map(|s| ConfidentValue::deterministic(Value::Text(s.clone())))
        .collect();
    fields.insert(key.into(), ConfidentValue::deterministic(Value::Array(arr)));
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
    insert_text(&mut fields, "workdir_root", &r.workdir_root);
    insert_text(&mut fields, "branch_prefix", &r.branch_prefix);
    insert_text(&mut fields, "commit_template", &r.commit_template);
    insert_text(&mut fields, "fix_commit_template", &r.fix_commit_template);
    insert_number(&mut fields, "max_iterations", r.max_iterations);
    insert_bool(&mut fields, "auto_approve", r.auto_approve);
    insert_number(&mut fields, "max_plan_revisions", r.max_plan_revisions);
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
            devops_channel  = "C_devops"

            [github]
            # no token_env set — should resolve to empty
            default_repo = "ncmlabs/forge"

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
            require_approval_for     = ["release", "destructive"]
            auto_approve_labels      = ["docs-only"]
            create_issue             = false
            create_issue_timeout_mins = 60
            "#,
        )
        .expect("parse");
        assert_eq!(cfg.org_name, "ncmlabs");
        assert_eq!(cfg.slack_default_channel, "C_default");
        assert_eq!(cfg.slack_devops_channel, "C_devops");
        assert_eq!(cfg.github_token, "");
        assert_eq!(cfg.github_default_repo, "ncmlabs/forge");
        assert_eq!(cfg.github_labels_triage, vec!["needs-triage".to_string()]);
        assert_eq!(cfg.llm_routing_high, "claude-opus");
        assert_eq!(cfg.warden_max_retries, 5.0);
        assert_eq!(cfg.warden_escalate_after_seconds, 7200.0);
        assert_eq!(cfg.budget_per_task_usd, 2.5);
        assert_eq!(cfg.budget_per_hour_usd, 20.0);
        assert_eq!(cfg.gates_require_approval_for.len(), 2);
        assert_eq!(cfg.gates_auto_approve_labels, vec!["docs-only".to_string()]);
        assert!(!cfg.gates_create_issue);
        assert_eq!(cfg.gates_create_issue_timeout_mins, 60.0);
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
        let alpha = cfg.repos.iter().find(|r| r.slug == "acme/alpha").unwrap();
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

    // ── T9.5 (#366) — devops_channel surface ────────────────────────

    #[test]
    fn parses_slack_devops_channel() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [slack]
            devops_channel = "C_devops"
            "#,
        )
        .expect("parse");
        assert_eq!(cfg.slack_devops_channel, "C_devops");
    }

    #[test]
    fn slack_devops_channel_defaults_to_empty() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [org]
            name = "ncmlabs"
            "#,
        )
        .expect("parse");
        assert_eq!(cfg.slack_devops_channel, "");
    }

    #[test]
    fn to_forge_record_emits_slack_devops_channel() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [slack]
            devops_channel = "C_devops"
            "#,
        )
        .expect("parse");
        let record = cfg.to_forge_record();
        let fields = match record {
            Value::Record(ref f) => f,
            _ => panic!("expected Record"),
        };
        let dev = fields
            .get("slack_devops_channel")
            .expect("slack_devops_channel field");
        match &dev.value {
            Value::Text(s) => assert_eq!(s, "C_devops"),
            _ => panic!("slack_devops_channel should be Text"),
        }
    }

    // ── T10.1 (#367) — Gate-1 (create-issue) toggles ──────────────

    #[test]
    fn gates_create_issue_defaults_when_omitted() {
        // Older TOML files (pre-#367) don't carry the new keys. Loader
        // must fall back to the documented defaults so gate_one boots
        // without operator action.
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [gates]
            require_approval_for = ["release"]
            "#,
        )
        .expect("parse");
        assert!(cfg.gates_create_issue);
        assert_eq!(
            cfg.gates_create_issue_timeout_mins,
            DEFAULT_GATES_CREATE_ISSUE_TIMEOUT_MINS
        );
        assert_eq!(cfg.github_default_repo, "");
    }

    #[test]
    fn to_forge_record_emits_gate_one_fields() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [github]
            default_repo = "ncmlabs/forge"

            [gates]
            create_issue              = false
            create_issue_timeout_mins = 45
            "#,
        )
        .expect("parse");
        let record = cfg.to_forge_record();
        let fields = match record {
            Value::Record(ref f) => f,
            _ => panic!("expected Record"),
        };
        let create_issue = fields.get("gates_create_issue").expect("flag field");
        match &create_issue.value {
            Value::Bool(b) => assert!(!*b),
            _ => panic!("gates_create_issue should be Bool"),
        }
        let timeout = fields
            .get("gates_create_issue_timeout_mins")
            .expect("timeout field");
        match &timeout.value {
            Value::Number(n) => assert_eq!(*n, 45.0),
            _ => panic!("gates_create_issue_timeout_mins should be Number"),
        }
        let repo = fields.get("github_default_repo").expect("repo field");
        match &repo.value {
            Value::Text(s) => assert_eq!(s, "ncmlabs/forge"),
            _ => panic!("github_default_repo should be Text"),
        }
    }

    // ── T10.2 (#368) — Gate-2 (start-implementation) toggles ──────

    #[test]
    fn gates_start_implementation_defaults_when_omitted() {
        // Older TOML files (pre-#368) don't carry the new keys. Loader
        // must fall back to the documented defaults so gate_two boots
        // without operator action.
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [gates]
            require_approval_for = ["release"]
            "#,
        )
        .expect("parse");
        assert!(cfg.gates_start_implementation);
        assert_eq!(
            cfg.gates_start_implementation_timeout_mins,
            DEFAULT_GATES_START_IMPLEMENTATION_TIMEOUT_MINS
        );
        assert_eq!(cfg.slack_approval_channel, "");
        assert_eq!(cfg.defaults_max_plan_revisions, DEFAULT_MAX_PLAN_REVISIONS);
    }

    #[test]
    fn gates_start_implementation_honors_authored_values() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [slack]
            approval_channel = "C_reviewers"

            [gates]
            start_implementation              = false
            start_implementation_timeout_mins = 45

            [defaults]
            max_plan_revisions = 5
            "#,
        )
        .expect("parse");
        assert!(!cfg.gates_start_implementation);
        assert_eq!(cfg.gates_start_implementation_timeout_mins, 45.0);
        assert_eq!(cfg.slack_approval_channel, "C_reviewers");
        assert_eq!(cfg.defaults_max_plan_revisions, 5.0);
    }

    #[test]
    fn per_repo_max_plan_revisions_overrides_default() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [defaults]
            max_plan_revisions = 3

            [repos."acme/alpha"]
            max_plan_revisions = 7

            [repos."acme/beta"]
            # inherits
            "#,
        )
        .expect("parse");
        let alpha = cfg.repos.iter().find(|r| r.slug == "acme/alpha").unwrap();
        let beta = cfg.repos.iter().find(|r| r.slug == "acme/beta").unwrap();
        assert_eq!(alpha.max_plan_revisions, 7.0);
        assert_eq!(beta.max_plan_revisions, 3.0);
    }

    #[test]
    fn to_forge_record_emits_gate_two_fields() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [slack]
            approval_channel = "C_reviewers"

            [gates]
            start_implementation              = false
            start_implementation_timeout_mins = 60

            [defaults]
            max_plan_revisions = 4

            [repos."acme/alpha"]
            max_plan_revisions = 8
            "#,
        )
        .expect("parse");
        let record = cfg.to_forge_record();
        let fields = match record {
            Value::Record(ref f) => f,
            _ => panic!("expected Record"),
        };

        let flag = fields
            .get("gates_start_implementation")
            .expect("flag field");
        match &flag.value {
            Value::Bool(b) => assert!(!*b),
            _ => panic!("gates_start_implementation should be Bool"),
        }
        let timeout = fields
            .get("gates_start_implementation_timeout_mins")
            .expect("timeout field");
        match &timeout.value {
            Value::Number(n) => assert_eq!(*n, 60.0),
            _ => panic!("gates_start_implementation_timeout_mins should be Number"),
        }
        let channel = fields
            .get("slack_approval_channel")
            .expect("approval channel field");
        match &channel.value {
            Value::Text(s) => assert_eq!(s, "C_reviewers"),
            _ => panic!("slack_approval_channel should be Text"),
        }
        let max_rev = fields
            .get("defaults_max_plan_revisions")
            .expect("max_plan_revisions field");
        match &max_rev.value {
            Value::Number(n) => assert_eq!(*n, 4.0),
            _ => panic!("defaults_max_plan_revisions should be Number"),
        }
        // Per-repo override surfaces on the repo record.
        let repos = fields.get("repos").expect("repos field");
        match &repos.value {
            Value::Array(items) => {
                let r = match &items[0].value {
                    Value::Record(r) => r,
                    _ => panic!("repo item should be a Record"),
                };
                let v = r.get("max_plan_revisions").expect("repo field");
                match &v.value {
                    Value::Number(n) => assert_eq!(*n, 8.0),
                    _ => panic!("repo.max_plan_revisions should be Number"),
                }
            }
            _ => panic!("repos should be an Array"),
        }
    }

    #[test]
    fn invalid_toml_returns_clean_error() {
        let err =
            CloneDevConfig::from_toml_str("this is not = valid = toml").expect_err("should reject");
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
    fn label_routing_parses_namespace_and_suffix_target_pairs() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [labels]
            namespace      = "clone-dev"
            triage_target  = "custom_triage"

            [labels.routing]
            plan   = "planner"
            impl   = "implementer"
            test   = "tester"
            review = "reviewer"
            merge  = "release_manager"
            ops    = "release_manager"
            "#,
        )
        .expect("parse");
        assert_eq!(cfg.label_routing_namespace, "clone-dev");
        assert_eq!(cfg.label_routing_triage_target, "custom_triage");
        // BTreeMap iteration is alphabetical by key; suffixes/targets are paired.
        assert_eq!(cfg.label_routing_suffixes.len(), 6);
        let pairs: Vec<(String, String)> = cfg
            .label_routing_suffixes
            .iter()
            .cloned()
            .zip(cfg.label_routing_targets.iter().cloned())
            .collect();
        assert!(pairs.contains(&("plan".into(), "planner".into())));
        assert!(pairs.contains(&("impl".into(), "implementer".into())));
        assert!(pairs.contains(&("test".into(), "tester".into())));
        assert!(pairs.contains(&("review".into(), "reviewer".into())));
        assert!(pairs.contains(&("merge".into(), "release_manager".into())));
        assert!(pairs.contains(&("ops".into(), "release_manager".into())));
    }

    #[test]
    fn label_routing_defaults_triage_target_when_omitted() {
        // Older TOML files (pre-#358) won't carry triage_target. Loader must
        // fall back to "triage_specialist" so the stub agent receives routes.
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [labels]
            triage = ["needs-triage"]
            "#,
        )
        .expect("parse");
        assert_eq!(cfg.label_routing_triage_target, "triage_specialist");
        assert_eq!(cfg.label_routing_namespace, "");
        assert!(cfg.label_routing_suffixes.is_empty());
        assert!(cfg.label_routing_targets.is_empty());
    }

    #[test]
    fn to_forge_record_emits_label_routing_fields() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [labels]
            namespace = "clone-dev"

            [labels.routing]
            plan = "planner"
            impl = "implementer"
            "#,
        )
        .expect("parse");
        let record = cfg.to_forge_record();
        let fields = match record {
            Value::Record(ref f) => f,
            _ => panic!("expected Record"),
        };
        assert!(fields.contains_key("label_routing_namespace"));
        assert!(fields.contains_key("label_routing_suffixes"));
        assert!(fields.contains_key("label_routing_targets"));
        assert!(fields.contains_key("label_routing_triage_target"));
        let suffixes = fields.get("label_routing_suffixes").expect("suffixes");
        match &suffixes.value {
            Value::Array(items) => assert_eq!(items.len(), 2),
            _ => panic!("suffixes should be a Text[]"),
        }
    }

    #[test]
    fn load_returns_clean_error_for_missing_file() {
        let missing = std::path::PathBuf::from("/nonexistent/forge-t357/not-a-file.toml");
        let err = load(&missing).expect_err("missing file should error");
        assert!(err.contains("cannot resolve config path"));
    }

    // ── T8.5 (#360) — dev-cycle templating defaults ────────────────

    #[test]
    fn defaults_templating_uses_built_in_constants_when_absent() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [org]
            name = "ncmlabs"
            "#,
        )
        .expect("parse");
        assert_eq!(cfg.defaults_workdir_root, DEFAULT_WORKDIR_ROOT);
        assert_eq!(cfg.defaults_branch_prefix, DEFAULT_BRANCH_PREFIX);
        assert_eq!(cfg.defaults_commit_template, DEFAULT_COMMIT_TEMPLATE);
        assert_eq!(
            cfg.defaults_fix_commit_template,
            DEFAULT_FIX_COMMIT_TEMPLATE
        );
        assert_eq!(cfg.defaults_max_iterations, DEFAULT_MAX_ITERATIONS);
        assert!(!cfg.defaults_auto_approve);
    }

    #[test]
    fn defaults_templating_honors_authored_values() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [defaults]
            workdir_root        = "/var/forge/work"
            branch_prefix       = "ncmlabs/clone"
            commit_template     = "feat({issue_id}) — {title}"
            fix_commit_template = "chore({issue_id}): retry {iteration}"
            max_iterations      = 5
            auto_approve        = true
            "#,
        )
        .expect("parse");
        assert_eq!(cfg.defaults_workdir_root, "/var/forge/work");
        assert_eq!(cfg.defaults_branch_prefix, "ncmlabs/clone");
        assert_eq!(cfg.defaults_commit_template, "feat({issue_id}) — {title}");
        assert_eq!(
            cfg.defaults_fix_commit_template,
            "chore({issue_id}): retry {iteration}"
        );
        assert_eq!(cfg.defaults_max_iterations, 5.0);
        assert!(cfg.defaults_auto_approve);
    }

    #[test]
    fn per_repo_templating_overrides_defaults() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [defaults]
            workdir_root    = "/var/forge/work"
            branch_prefix   = "clone-dev"
            commit_template = "feat({issue_id}): default"
            max_iterations  = 3
            auto_approve    = false

            [repos."acme/alpha"]
            workdir_root    = "/var/forge/alpha"
            branch_prefix   = "alpha"
            commit_template = "alpha({issue_id}): {title}"
            max_iterations  = 7
            auto_approve    = true

            [repos."acme/beta"]
            # inherits everything
            "#,
        )
        .expect("parse");
        let alpha = cfg.repos.iter().find(|r| r.slug == "acme/alpha").unwrap();
        let beta = cfg.repos.iter().find(|r| r.slug == "acme/beta").unwrap();

        // alpha overrides win
        assert_eq!(alpha.workdir_root, "/var/forge/alpha");
        assert_eq!(alpha.branch_prefix, "alpha");
        assert_eq!(alpha.commit_template, "alpha({issue_id}): {title}");
        assert_eq!(alpha.max_iterations, 7.0);
        assert!(alpha.auto_approve);

        // beta inherits resolved defaults
        assert_eq!(beta.workdir_root, "/var/forge/work");
        assert_eq!(beta.branch_prefix, "clone-dev");
        assert_eq!(beta.commit_template, "feat({issue_id}): default");
        assert_eq!(beta.max_iterations, 3.0);
        assert!(!beta.auto_approve);
    }

    #[test]
    fn per_repo_inherits_built_in_when_defaults_section_absent() {
        // Neither [defaults] nor per-repo override sets templating fields:
        // each repo must still resolve to the built-in constants so
        // agents.forge can run unconfigured.
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [repos."acme/orphan"]
            "#,
        )
        .expect("parse");
        let orphan = cfg.repos.first().expect("one repo");
        assert_eq!(orphan.workdir_root, DEFAULT_WORKDIR_ROOT);
        assert_eq!(orphan.branch_prefix, DEFAULT_BRANCH_PREFIX);
        assert_eq!(orphan.commit_template, DEFAULT_COMMIT_TEMPLATE);
        assert_eq!(orphan.fix_commit_template, DEFAULT_FIX_COMMIT_TEMPLATE);
        assert_eq!(orphan.max_iterations, DEFAULT_MAX_ITERATIONS);
        assert!(!orphan.auto_approve);
    }

    #[test]
    fn to_forge_record_emits_templating_fields() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [defaults]
            workdir_root    = "/var/forge/work"
            commit_template = "feat({issue_id}): {title}"
            auto_approve    = true

            [repos."acme/alpha"]
            branch_prefix = "alpha"
            "#,
        )
        .expect("parse");
        let record = cfg.to_forge_record();
        let fields = match record {
            Value::Record(ref f) => f,
            _ => panic!("expected Record"),
        };
        // Top-level defaults_* present.
        for key in [
            "defaults_workdir_root",
            "defaults_branch_prefix",
            "defaults_commit_template",
            "defaults_fix_commit_template",
            "defaults_max_iterations",
            "defaults_auto_approve",
        ] {
            assert!(fields.contains_key(key), "missing field {key}");
        }
        // Per-repo record carries the merged templating fields.
        let repos = fields.get("repos").expect("repos field");
        match &repos.value {
            Value::Array(items) => {
                let r = match &items[0].value {
                    Value::Record(r) => r,
                    _ => panic!("repo item should be a Record"),
                };
                for key in [
                    "workdir_root",
                    "branch_prefix",
                    "commit_template",
                    "fix_commit_template",
                    "max_iterations",
                    "auto_approve",
                ] {
                    assert!(r.contains_key(key), "missing repo field {key}");
                }
            }
            _ => panic!("repos should be an Array"),
        }
    }

    // ── T8.6 (#361) — phase-keyed [llm.routing] ──────────────────────

    #[test]
    fn routing_phase_keyed_round_trips() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [llm.routing]
            classify        = "claude-haiku"
            plan            = "sonnet"
            implement       = "gpt-4o"
            review          = "sonnet"
            ops_investigate = "ollama-local"
            "#,
        )
        .expect("parse");
        assert_eq!(
            cfg.llm_routing.get("classify"),
            Some(&"claude-haiku".into())
        );
        assert_eq!(cfg.llm_routing.get("plan"), Some(&"sonnet".into()));
        assert_eq!(cfg.llm_routing.get("implement"), Some(&"gpt-4o".into()));
        assert_eq!(
            cfg.llm_routing.get("ops_investigate"),
            Some(&"ollama-local".into())
        );
        assert!(cfg.llm_routing_fallback.is_empty());
    }

    #[test]
    fn routing_back_compat_with_fast_balanced_high() {
        // Pre-#361 configs only used fast/balanced/high. Post-#361 those
        // keys live in the same primary table as any other phase, and the
        // back-compat scalars on CloneDevConfig still resolve to them.
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [llm.routing]
            fast     = "claude-haiku"
            balanced = "claude-haiku"
            high     = "claude-opus"
            "#,
        )
        .expect("parse");
        assert_eq!(cfg.llm_routing_fast, "claude-haiku");
        assert_eq!(cfg.llm_routing_balanced, "claude-haiku");
        assert_eq!(cfg.llm_routing_high, "claude-opus");
        // And those entries are also visible in the unified routing map so
        // a future caller can iterate every configured phase.
        assert_eq!(cfg.llm_routing.get("fast"), Some(&"claude-haiku".into()));
        assert_eq!(cfg.llm_routing.get("high"), Some(&"claude-opus".into()));
    }

    #[test]
    fn routing_resolves_chain_with_fallback_table() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [llm.routing]
            plan      = "sonnet"
            implement = "gpt-4o"

            [llm.routing.fallback]
            plan      = ["sonnet", "gpt-4o", "ollama-local"]
            implement = ["gpt-4o", "sonnet"]
            "#,
        )
        .expect("parse");

        // Primary first, then fallback entries appended without dups.
        assert_eq!(
            cfg.routing("plan"),
            vec!["sonnet".to_string(), "gpt-4o".into(), "ollama-local".into()]
        );
        // For implement, primary 'gpt-4o' equals the first fallback entry —
        // dedup keeps the chain at 2 entries.
        assert_eq!(
            cfg.routing("implement"),
            vec!["gpt-4o".to_string(), "sonnet".into()]
        );
    }

    #[test]
    fn routing_unknown_phase_returns_empty_chain() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [llm.routing]
            plan = "sonnet"
            "#,
        )
        .expect("parse");
        assert!(cfg.routing("does_not_exist").is_empty());
    }

    #[test]
    fn routing_table_emits_every_configured_phase() {
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [llm.routing]
            plan      = "sonnet"
            implement = "gpt-4o"

            [llm.routing.fallback]
            review = ["sonnet"]
            "#,
        )
        .expect("parse");
        let table = cfg.routing_table();
        // plan + implement carry primary; review is fallback-only and still
        // surfaces because operators may want a chain even without a primary.
        assert_eq!(table.len(), 3);
        assert_eq!(table.get("plan").unwrap(), &vec!["sonnet".to_string()]);
        assert_eq!(table.get("implement").unwrap(), &vec!["gpt-4o".to_string()]);
        assert_eq!(table.get("review").unwrap(), &vec!["sonnet".to_string()]);
    }

    #[test]
    fn routing_silently_skips_malformed_entries() {
        // Non-string values for primary keys and non-array fallback values
        // are dropped rather than rejecting the whole config — keeps a
        // newer config readable by older runtimes (forward-compat).
        let cfg = CloneDevConfig::from_toml_str(
            r#"
            [llm.routing]
            plan = "sonnet"
            broken_number = 42
            broken_array = ["a", "b"]

            [llm.routing.fallback]
            plan = ["sonnet", "gpt-4o"]
            broken_str = "not-a-list"
            "#,
        )
        .expect("parse");
        assert_eq!(cfg.llm_routing.get("plan"), Some(&"sonnet".into()));
        assert!(!cfg.llm_routing.contains_key("broken_number"));
        assert!(!cfg.llm_routing.contains_key("broken_array"));
        assert_eq!(
            cfg.llm_routing_fallback.get("plan"),
            Some(&vec!["sonnet".into(), "gpt-4o".into()])
        );
        assert!(!cfg.llm_routing_fallback.contains_key("broken_str"));
    }
}
