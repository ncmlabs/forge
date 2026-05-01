// T8.5 (#360) — concurrent-invocation workdir isolation.
//
// agents.forge (issue 360 work) computes the dev-cycle workdir as
// `{repo_cfg.workdir_root}/{repo_slug}/{issue_id}-{short_id}`. The
// DoD requires that two concurrent invocations with the same
// {issue_id} but different {repo_slug} land in distinct, non-
// colliding paths. This test exercises the resolution + formatting
// at the same layer agents.forge does — going through the Rust
// loader and the same `text.short_id`-style suffix — and confirms:
//   1. distinct repo_slug ⇒ distinct path even with shared issue_id
//   2. same repo_slug + same issue_id but distinct invocations also
//      diverge thanks to the short_id suffix
//   3. creating real files in each workdir does not interfere
//
// Pure-Rust mirror of the FORGE-side string interpolation keeps the
// test independent of the executor pipeline; if the formula changes
// in agents.forge the assertion comment below should be updated to
// match.

use forge::runtime::clone_dev_config::CloneDevConfig;

fn workdir_for(workdir_root: &str, repo_slug: &str, issue_id: &str, short_id: &str) -> String {
    // Mirrors agents.forge:
    //   memory.workdir = "{repo_cfg.workdir_root}/{memory.repo_slug}/{issue_id}-{short_id}"
    format!("{workdir_root}/{repo_slug}/{issue_id}-{short_id}")
}

fn short_id() -> String {
    // Mirrors the executor's text.short_id intrinsic: 8-char prefix
    // of a v4 UUID's simple form. Inlined so the test doesn't depend
    // on running FORGE source through the executor.
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

#[test]
fn distinct_repos_with_same_issue_id_resolve_to_distinct_workdirs() {
    let cfg = CloneDevConfig::from_toml_str(
        r#"
        [defaults]
        workdir_root = "/tmp/forge-t360-isolation"

        [repos."acme/alpha"]
        # inherits workdir_root

        [repos."acme/beta"]
        # inherits workdir_root
        "#,
    )
    .expect("parse");

    let alpha = cfg
        .repos
        .iter()
        .find(|r| r.slug == "acme/alpha")
        .expect("alpha");
    let beta = cfg
        .repos
        .iter()
        .find(|r| r.slug == "acme/beta")
        .expect("beta");

    let issue_id = "360";
    let alpha_dir = workdir_for(&alpha.workdir_root, &alpha.slug, issue_id, &short_id());
    let beta_dir = workdir_for(&beta.workdir_root, &beta.slug, issue_id, &short_id());

    assert_ne!(
        alpha_dir, beta_dir,
        "two repos with shared issue_id must resolve to distinct workdirs"
    );
    assert!(
        alpha_dir.contains("acme/alpha/"),
        "alpha workdir should contain repo slug, got: {alpha_dir}"
    );
    assert!(
        beta_dir.contains("acme/beta/"),
        "beta workdir should contain repo slug, got: {beta_dir}"
    );
}

#[test]
fn per_repo_workdir_root_overrides_isolate_at_root_level() {
    // Beyond same-issue/different-repo: per-repo workdir_root override
    // means even `acme/alpha` instances on different hosts (or test
    // runners) can be steered to disjoint roots.
    let cfg = CloneDevConfig::from_toml_str(
        r#"
        [defaults]
        workdir_root = "/tmp/forge-t360-default"

        [repos."acme/alpha"]
        workdir_root = "/tmp/forge-t360-alpha"

        [repos."acme/beta"]
        # inherits default workdir_root
        "#,
    )
    .expect("parse");

    let alpha = cfg.repos.iter().find(|r| r.slug == "acme/alpha").unwrap();
    let beta = cfg.repos.iter().find(|r| r.slug == "acme/beta").unwrap();

    assert_eq!(alpha.workdir_root, "/tmp/forge-t360-alpha");
    assert_eq!(beta.workdir_root, "/tmp/forge-t360-default");
    assert_ne!(
        alpha.workdir_root, beta.workdir_root,
        "alpha override should take precedence over defaults"
    );
}

#[test]
fn same_repo_same_issue_distinct_invocations_diverge_via_short_id() {
    // The short_id suffix guards against retry / parallel-run
    // collisions when the (repo_slug, issue_id) pair is identical.
    let cfg = CloneDevConfig::from_toml_str(
        r#"
        [defaults]
        workdir_root = "/tmp/forge-t360-retry"

        [repos."acme/gamma"]
        "#,
    )
    .expect("parse");

    let gamma = cfg.repos.first().expect("one repo");
    let issue_id = "360";

    let invoc1 = workdir_for(&gamma.workdir_root, &gamma.slug, issue_id, &short_id());
    let invoc2 = workdir_for(&gamma.workdir_root, &gamma.slug, issue_id, &short_id());

    assert_ne!(
        invoc1, invoc2,
        "two invocations on the same repo+issue must diverge via short_id"
    );
}

#[test]
fn concurrent_workdirs_can_be_created_without_collision_on_disk() {
    // End-to-end-ish: actually mkdir both candidate workdirs and
    // confirm they coexist. Cleans up after itself; uses a unique
    // root so parallel test runs don't tread on each other.
    let root = std::env::temp_dir().join(format!(
        "forge-t360-fs-{pid}-{nonce}",
        pid = std::process::id(),
        nonce = short_id()
    ));
    let cfg = CloneDevConfig::from_toml_str(&format!(
        r#"
        [defaults]
        workdir_root = "{root}"

        [repos."acme/alpha"]
        [repos."acme/beta"]
        "#,
        root = root.display()
    ))
    .expect("parse");

    let issue_id = "concurrent-collision-check";
    let alpha = cfg.repos.iter().find(|r| r.slug == "acme/alpha").unwrap();
    let beta = cfg.repos.iter().find(|r| r.slug == "acme/beta").unwrap();
    let alpha_dir = workdir_for(&alpha.workdir_root, &alpha.slug, issue_id, &short_id());
    let beta_dir = workdir_for(&beta.workdir_root, &beta.slug, issue_id, &short_id());

    std::fs::create_dir_all(&alpha_dir).expect("mkdir alpha");
    std::fs::create_dir_all(&beta_dir).expect("mkdir beta");
    let alpha_marker = std::path::Path::new(&alpha_dir).join("OWNER");
    let beta_marker = std::path::Path::new(&beta_dir).join("OWNER");
    std::fs::write(&alpha_marker, b"alpha").expect("write alpha marker");
    std::fs::write(&beta_marker, b"beta").expect("write beta marker");

    assert_eq!(
        std::fs::read_to_string(&alpha_marker).expect("read alpha"),
        "alpha"
    );
    assert_eq!(
        std::fs::read_to_string(&beta_marker).expect("read beta"),
        "beta"
    );

    let _ = std::fs::remove_dir_all(&root);
}
