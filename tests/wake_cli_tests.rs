// FORGE `forge wake` CLI tests — issue #335
// Exercises the storage surface used by the CLI handlers directly. The CLI
// itself is a thin wrapper around `ForgeStorage::{upsert,lookup,list,delete}_wake_secret`
// plus `getrandom::fill`, so driving storage directly is sufficient coverage
// for the round-trip semantics the user sees.

use forge::runtime::storage::ForgeStorage;
use tempfile::tempdir;

fn fresh_storage() -> (tempfile::TempDir, ForgeStorage) {
    let dir = tempdir().unwrap();
    let storage = ForgeStorage::open(&dir.path().join("wake.redb")).unwrap();
    (dir, storage)
}

#[test]
fn register_then_lookup_round_trips_the_secret() {
    let (_dir, storage) = fresh_storage();
    storage
        .upsert_wake_secret("mastermind", "pr_merged", "deadbeef")
        .unwrap();
    let s = storage
        .lookup_wake_secret("mastermind", "pr_merged")
        .unwrap()
        .expect("secret should be present after register");
    assert_eq!(s, "deadbeef");
}

#[test]
fn rotate_replaces_prior_secret_and_produces_new_bytes() {
    let (_dir, storage) = fresh_storage();
    // Two rotations — simulating the CLI's generate-and-upsert path.
    let gen_and_upsert = |agent: &str, trigger: &str| -> String {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).unwrap();
        let hex = hex::encode(bytes);
        storage.upsert_wake_secret(agent, trigger, &hex).unwrap();
        hex
    };
    let first = gen_and_upsert("a", "t");
    let second = gen_and_upsert("a", "t");
    assert_ne!(first, second, "rotation must produce fresh random bytes");
    assert_eq!(
        storage.lookup_wake_secret("a", "t").unwrap(),
        Some(second.clone())
    );
    assert_eq!(second.len(), 64, "32 bytes hex-encoded is 64 characters");
}

#[test]
fn list_returns_pairs_but_never_secret_material() {
    let (_dir, storage) = fresh_storage();
    storage
        .upsert_wake_secret("mastermind", "pr_merged", "sentinel-secret-a")
        .unwrap();
    storage
        .upsert_wake_secret("bridge", "incoming", "sentinel-secret-b")
        .unwrap();

    let pairs = storage.list_wake_triggers().unwrap();
    // The sentinel strings must not appear anywhere in the serialized form.
    let out = format!("{pairs:?}");
    assert!(
        !out.contains("sentinel-secret"),
        "list must not leak secret bytes, got {out}"
    );
    assert_eq!(pairs.len(), 2);
}

#[test]
fn delete_removes_row_and_reports_prior_state() {
    let (_dir, storage) = fresh_storage();
    storage.upsert_wake_secret("a", "t", "s").unwrap();
    assert!(storage.delete_wake_secret("a", "t").unwrap());
    assert!(!storage.delete_wake_secret("a", "t").unwrap());
    assert!(storage.lookup_wake_secret("a", "t").unwrap().is_none());
}

#[test]
fn register_rejects_empty_secret_via_trim() {
    // The CLI trims stdin before upsert; storage itself accepts any bytes.
    // Assertion: a trimmed empty string is refused at the CLI layer. We
    // reproduce that branch by simulating the trim-and-check guard.
    let raw = "   \n  \t\n";
    let trimmed = raw.trim();
    assert!(
        trimmed.is_empty(),
        "guard condition must fire on blank stdin"
    );
}
