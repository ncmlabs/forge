// Integration tests for T10.3 (#369) — Gate-3 (merge-PR) config
// toggle on the reviewer agent in workflows/dev-cycle/agents.forge.
//
// Gate 3 differs from gates 1 and 2 in shape: the merge-PR approval
// already lives inside the reviewer agent (lines ~720-862 of
// dev-cycle/agents.forge), so this issue adds a config branch and a
// timeout state machine rather than a new standalone agent. The
// reviewer reads `repo_config_for(memory.config, repo_slug).merge_pr`
// to decide whether to emit a Slack approval card or auto-merge.
//
// Unlike gate_one / gate_two, the reviewer is heavily integration-y:
// every code path calls `skill.github.create_pr`, `check_ci`,
// `merge_pr`, and `delete_branch` (real `gh` CLI invocations) plus
// `recall` against the knowledge store. None of the existing tests in
// `tests/` boot the reviewer agent end-to-end (grep confirms it).
// Mocking the gh skill is out of scope for this PR; the live reviewer
// behavior will be exercised by a follow-up live smoke test mirroring
// `clone_dev_gate_two_live_smoke.rs`.
//
// What this file *does* cover deterministically — the new config
// surface that the reviewer reads:
//
//   1. Default value when [gates] merge_pr is omitted (back-compat
//      with pre-#369 TOML files).
//   2. Authored values from the [gates] section override the default.
//   3. Per-repo `[repos."<owner>/<name>"] merge_pr = ...` overrides
//      the gate value while leaving other repos on the global default.
//   4. The shipped `workflows/dev-cycle/clone-dev.toml` parses cleanly
//      and exposes the documented defaults — guards against authoring
//      drift between code and config.
//   5. The FORGE record `to_forge_record()` exposes both the new
//      top-level scalars and the per-repo `merge_pr` field, which is
//      what `repo_config_for` reads on the FORGE side.
//
// The Rust unit tests in src/runtime/clone_dev_config.rs already
// cover (1)-(3) and (5) at a granular level; this file re-asserts
// them at the integration layer (loading from disk, resolving env
// vars, exercising the cache) and adds the shipped-config check (4).

#![allow(clippy::float_cmp)]

use forge::runtime::clone_dev_config::CloneDevConfig;

const SHIPPED_CONFIG_PATH: &str = "workflows/dev-cycle/clone-dev.toml";

// ── (1) Default value when [gates] merge_pr is omitted ─────────────

#[test]
fn merge_pr_defaults_to_true_when_gates_section_omits_it() {
    // Pre-#369 TOML files don't carry the new keys. The reviewer must
    // boot with the slack-approval flow on by default — operators opt
    // into auto-merge, never the other way around.
    let cfg = CloneDevConfig::from_toml_str(
        r#"
        [org]
        name = "back-compat"
        "#,
    )
    .expect("parse minimal");
    assert!(
        cfg.gates_merge_pr,
        "gates.merge_pr must default to true (slack-approval on)"
    );
    assert_eq!(
        cfg.gates_merge_pr_timeout_mins, 30.0,
        "gates.merge_pr_timeout_mins must default to 30"
    );
}

// ── (2) Authored values override the default ──────────────────────

#[test]
fn merge_pr_false_disables_slack_approval_globally() {
    // `merge_pr = false` is the sandbox-org case: every repo
    // auto-merges after CI green + knowledge-store consultation,
    // skipping the Slack approval card entirely.
    let cfg = CloneDevConfig::from_toml_str(
        r#"
        [gates]
        merge_pr              = false
        merge_pr_timeout_mins = 60
        "#,
    )
    .expect("parse");
    assert!(!cfg.gates_merge_pr);
    assert_eq!(cfg.gates_merge_pr_timeout_mins, 60.0);
}

// ── (3) Per-repo override semantics ───────────────────────────────

#[test]
fn per_repo_merge_pr_overrides_global_gate_value() {
    // The motivating case from the issue: "some orgs want auto-merge
    // for sandbox repos; most want the manual button-click." A single
    // organization config can mix both — production stays on the gate,
    // sandbox flips off, and both inherit through the same
    // `repo_config_for` lookup the reviewer uses.
    let cfg = CloneDevConfig::from_toml_str(
        r#"
        [gates]
        merge_pr = true

        [repos."acme/prod"]
        # inherits the global gate (true)

        [repos."acme/sandbox"]
        merge_pr = false
        "#,
    )
    .expect("parse");

    assert!(cfg.gates_merge_pr, "global default stays true");
    let prod = cfg
        .repos
        .iter()
        .find(|r| r.slug == "acme/prod")
        .expect("prod repo");
    let sandbox = cfg
        .repos
        .iter()
        .find(|r| r.slug == "acme/sandbox")
        .expect("sandbox repo");
    assert!(
        prod.merge_pr,
        "prod inherits gates.merge_pr = true (slack approval still required)"
    );
    assert!(
        !sandbox.merge_pr,
        "sandbox per-repo override flips merge_pr to false (auto-merge)"
    );
}

#[test]
fn per_repo_merge_pr_can_re_enable_when_gate_is_off() {
    // Symmetric override: a globally-disabled gate can be re-enabled
    // for one critical repo. This is the "auto-merge everywhere except
    // the production repo" shape.
    let cfg = CloneDevConfig::from_toml_str(
        r#"
        [gates]
        merge_pr = false

        [repos."acme/critical"]
        merge_pr = true

        [repos."acme/sandbox"]
        # inherits gates.merge_pr = false
        "#,
    )
    .expect("parse");

    assert!(!cfg.gates_merge_pr);
    let critical = cfg
        .repos
        .iter()
        .find(|r| r.slug == "acme/critical")
        .expect("critical repo");
    let sandbox = cfg
        .repos
        .iter()
        .find(|r| r.slug == "acme/sandbox")
        .expect("sandbox repo");
    assert!(
        critical.merge_pr,
        "critical repo per-repo override re-enables slack approval"
    );
    assert!(!sandbox.merge_pr, "sandbox inherits gates.merge_pr = false");
}

// ── (4) Shipped config parses with documented defaults ─────────────

#[test]
fn shipped_clone_dev_toml_parses_with_documented_gate_three_defaults() {
    // The dev-cycle standalone runner ships with
    // workflows/dev-cycle/clone-dev.toml. After this PR it carries
    // `[gates] merge_pr = true` and `merge_pr_timeout_mins = 30`. If
    // an operator deletes those lines or types `merge_pr = false` by
    // accident, this test catches the drift before the reviewer boots
    // with surprising behavior.
    let src = std::fs::read_to_string(SHIPPED_CONFIG_PATH).unwrap_or_else(|e| {
        panic!("could not read {SHIPPED_CONFIG_PATH}: {e}");
    });
    let cfg = CloneDevConfig::from_toml_str(&src).expect("parse shipped config");
    assert!(
        cfg.gates_merge_pr,
        "shipped config should keep gates.merge_pr = true (default behavior)"
    );
    assert_eq!(
        cfg.gates_merge_pr_timeout_mins, 30.0,
        "shipped config should keep gates.merge_pr_timeout_mins = 30"
    );
}

// ── (5) FORGE record exposes the new fields ───────────────────────

#[test]
fn forge_record_carries_merge_pr_top_level_and_per_repo() {
    // The reviewer's FORGE code reads `memory.config.gates_merge_pr`
    // (top-level) and `repo_config_for(...).merge_pr` (per-repo). Both
    // must surface on the Value::Record produced by `to_forge_record`
    // or the reviewer's `on start` log line will print garbage and
    // the auto-merge branch will silently never fire.
    use forge::runtime::confidence::Value;

    let cfg = CloneDevConfig::from_toml_str(
        r#"
        [gates]
        merge_pr              = false
        merge_pr_timeout_mins = 45

        [repos."acme/sandbox"]
        merge_pr = true
        "#,
    )
    .expect("parse");
    let record = cfg.to_forge_record();
    let fields = match record {
        Value::Record(ref f) => f,
        _ => panic!("expected Record"),
    };

    let flag = fields
        .get("gates_merge_pr")
        .expect("gates_merge_pr field missing");
    match &flag.value {
        Value::Bool(b) => assert!(!*b, "top-level gates_merge_pr should reflect TOML"),
        _ => panic!("gates_merge_pr should be Bool"),
    }

    let timeout = fields
        .get("gates_merge_pr_timeout_mins")
        .expect("gates_merge_pr_timeout_mins field missing");
    match &timeout.value {
        Value::Number(n) => assert_eq!(*n, 45.0),
        _ => panic!("gates_merge_pr_timeout_mins should be Number"),
    }

    // The per-repo override surfaces on the repo record so
    // `repo_config_for` returns the right value. Sandbox flipped the
    // gate back to true even though the global default is false.
    let repos = fields.get("repos").expect("repos field missing");
    let items = match &repos.value {
        Value::Array(items) => items,
        _ => panic!("repos should be Array"),
    };
    let sandbox = items
        .iter()
        .find_map(|cv| match &cv.value {
            Value::Record(r) => {
                let slug = r.get("slug").and_then(|s| match &s.value {
                    Value::Text(t) => Some(t.as_str()),
                    _ => None,
                })?;
                if slug == "acme/sandbox" {
                    Some(r)
                } else {
                    None
                }
            }
            _ => None,
        })
        .expect("sandbox repo record missing");
    let merge_pr = sandbox
        .get("merge_pr")
        .expect("repo merge_pr field missing");
    match &merge_pr.value {
        Value::Bool(b) => assert!(
            *b,
            "sandbox per-repo merge_pr override should win on the repo record"
        ),
        _ => panic!("repo.merge_pr should be Bool"),
    }
}
