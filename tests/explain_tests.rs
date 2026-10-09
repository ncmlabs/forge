// `forge explain` binary tests (#474): every registered diagnostic code must be
// explainable from the CLI, and unknown codes must fail loudly.

use forge::diagnostic_codes::CODES;

fn forge(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(args)
        .output()
        .expect("failed to execute forge binary")
}

#[test]
fn explain_prints_title_and_snippets() {
    let output = forge(&["explain", "E030"]);
    assert!(
        output.status.success(),
        "forge explain E030 failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("E030"), "stdout: {stdout}");
    assert!(stdout.contains(" — "), "stdout: {stdout}");
    assert!(stdout.contains("Wrong:"), "stdout: {stdout}");
    assert!(stdout.contains("Right:"), "stdout: {stdout}");
}

#[test]
fn explain_works_for_every_registered_code() {
    for info in CODES {
        let output = forge(&["explain", info.code]);
        assert!(
            output.status.success(),
            "forge explain {} failed: {}",
            info.code,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(info.code) && stdout.contains(info.title),
            "forge explain {} printed: {stdout}",
            info.code
        );
    }
}

#[test]
fn explain_unknown_code_exits_one() {
    let output = forge(&["explain", "E999"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown code E999") && stderr.contains("forge explain --list"),
        "stderr: {stderr}"
    );
}

#[test]
fn explain_list_prints_every_code() {
    let output = forge(&["explain", "--list"]);
    assert!(
        output.status.success(),
        "forge explain --list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for info in CODES {
        assert!(
            stdout.contains(&format!("{} — {}", info.code, info.title)),
            "missing {} from --list output",
            info.code
        );
    }
}
