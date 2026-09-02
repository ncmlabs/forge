// #448 — wedged/invalid redb store must fail fast with actionable errors.
use forge::runtime::storage::{ForgeStorage, StorageError};

/// A fresh store opens normally and round-trips a key.
#[test]
fn fresh_store_opens_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fresh.redb");
    let storage = ForgeStorage::open(&path).expect("fresh open must succeed");
    storage.store("k", "v").unwrap();
    assert_eq!(storage.get("k").unwrap(), Some("v".to_string()));
}

/// A text file at the store path (not a redb db) must be classified as
/// NotADatabase with rotation guidance — not an opaque error.
#[test]
fn non_redb_file_is_classified_as_not_a_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("garbage.redb");
    std::fs::write(&path, "this is definitely not a database file").unwrap();
    let err = match ForgeStorage::open(&path) {
        Err(e) => e,
        Ok(_) => panic!("garbage file must fail to open"),
    };
    match &err {
        StorageError::NotADatabase { path } => {
            assert!(path.ends_with("garbage.redb"), "error names the store path");
        }
        other => panic!("expected NotADatabase, got: {other:?}"),
    }
    let msg = err.to_string();
    assert!(
        msg.contains("rotate"),
        "error must include recovery guidance: {msg}"
    );
}

/// An empty (zero-length) file is a valid store-create target for redb —
/// it must open and round-trip (pre-existing redb contract, kept green).
#[test]
fn empty_file_initializes_normally() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zero.redb");
    std::fs::write(&path, b"").unwrap();
    let storage = ForgeStorage::open(&path).expect("empty file is creatable");
    storage.store("k", "v").unwrap();
    assert_eq!(
        storage.get("k").expect("read after create"),
        Some("v".into())
    );
}

/// A store file with a plausible-size but corrupted header must fail with a
/// classified error, not wedge or panic (mid-write kill approximation).
#[test]
fn corridorupted_store_fails_fast_with_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupted.redb");
    // Real store first so the size is plausible, then clobber the header.
    {
        let _s = ForgeStorage::open(&path).unwrap();
    }
    let mut bytes = std::fs::read(&path).unwrap();
    for b in bytes.iter_mut().take(64) {
        *b = 0xAB;
    }
    std::fs::write(&path, &bytes).unwrap();
    let started = std::time::Instant::now();
    let result = ForgeStorage::open(&path);
    let elapsed = started.elapsed();
    assert!(result.is_err(), "clobbered header must fail");
    let msg = result.err().unwrap().to_string();
    assert!(
        !msg.is_empty() && elapsed < std::time::Duration::from_secs(9),
        "must fail fast (< open timeout), took {elapsed:?}: {msg}"
    );
}

/// Two processes contending for the same store: the second must fail fast
/// with DatabaseAlreadyOpen, never hang.
#[test]
fn lock_contention_fails_fast() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("locked.redb");
    // Hold a database open; opening again from another "process" (instance)
    // must return quickly with an error.
    let _holder = ForgeStorage::open(&path).expect("first open succeeds");
    let started = std::time::Instant::now();
    let second = ForgeStorage::open(&path);
    let elapsed = started.elapsed();
    assert!(second.is_err(), "second concurrent open must fail");
    assert!(
        elapsed < std::time::Duration::from_secs(9),
        "contention must fail fast, took {elapsed:?}"
    );
}

/// Display strings for the new #448 variants carry the path + guidance.
#[test]
fn storage_error_display_is_actionable() {
    let timed_out = StorageError::OpenTimedOut {
        path: "/x/.forge-data/store.redb".into(),
        timeout_secs: 10,
    };
    let msg = timed_out.to_string();
    assert!(
        msg.contains("wedged") && msg.contains("/x/.forge-data/store.redb"),
        "{msg}"
    );

    let version = StorageError::VersionMismatch {
        path: "/x/store.redb".into(),
        found: 2,
        hint: "rotate it".into(),
    };
    let msg = version.to_string();
    assert!(msg.contains("v2") && msg.contains("rotate it"), "{msg}");

    let not_db = StorageError::NotADatabase {
        path: "/x/s".into(),
    };
    let msg = not_db.to_string();
    assert!(msg.contains("not a redb database"), "{msg}");
}
