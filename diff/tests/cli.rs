use std::process::Command;

#[test]
fn cli_prints_the_demo_timeline() {
    let output = Command::new(env!("CARGO_BIN_EXE_attribute_graph_diff"))
        .args(["--format", "text"])
        .output()
        .expect("cli should run");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("Snapshot: empty graph"));
    assert!(stdout.contains("Diff: read grand total -> price changed"));
    assert!(stdout.contains("node price (#0) value 10 (i64) -> 11 (i64)"));
    assert!(stdout.contains("edge price (#0) -> total (#3) state Settled -> Pending"));
    assert!(stdout.contains("MaybeDirty"));
    assert!(stdout.contains("Pending"));
}

#[test]
fn cli_prints_mermaid_when_requested() {
    let output = Command::new(env!("CARGO_BIN_EXE_attribute_graph_diff"))
        .args(["--format", "mermaid"])
        .output()
        .expect("cli should run");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.starts_with("flowchart LR"));
    assert!(stdout.contains("price (#0)"));
    assert!(stdout.contains("grand total (#4)"));
    assert!(stdout.contains("price + quantity"));
    assert!(stdout.contains("total + shipping"));
    assert!(stdout.contains("MaybeDirty"));
    assert!(stdout.contains("Pending"));
    assert!(stdout.contains("Settled"));
}

#[test]
fn cli_rejects_unknown_formats() {
    let output = Command::new(env!("CARGO_BIN_EXE_attribute_graph_diff"))
        .args(["--format", "wat"])
        .output()
        .expect("cli should run");

    assert!(!output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf8");
    assert!(stderr.contains("unknown format"));
}
