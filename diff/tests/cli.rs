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
    assert!(stdout.contains("edge total (#3) depends on price (#0) state Settled -> Pending"));
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
    assert!(stdout.contains("s3_n3 -->|\"Pending\"| s3_n0"));
    assert!(stdout.contains("s3_n4 -->|\"Pending\"| s3_n3"));
}

#[test]
fn cli_can_run_the_same_output_scenario() {
    let output = Command::new(env!("CARGO_BIN_EXE_attribute_graph_diff"))
        .args(["--scenario", "same-output", "--format", "text"])
        .output()
        .expect("cli should run");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("capped price (#2)"));
    assert!(stdout.contains("price label (#3)"));
    assert!(stdout.contains("capped total (#4)"));
    assert!(stdout.contains("Diff: price changed above cap -> read dependents again"));
    assert!(stdout.contains("node capped price (#2) state Dirty -> Clean"));
    assert!(stdout.contains("node capped total (#4) state MaybeDirty -> Clean"));
}

#[test]
fn cli_escapes_string_values_in_mermaid_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_attribute_graph_diff"))
        .args(["--scenario", "same-output", "--format", "mermaid"])
        .output()
        .expect("cli should run");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("value: #quot;capped#quot; (String)"));
    assert!(!stdout.contains("\\\"capped\\\""));
}

#[test]
fn cli_can_run_the_conditional_scenario() {
    let output = Command::new(env!("CARGO_BIN_EXE_attribute_graph_diff"))
        .args(["--scenario=conditional", "--format", "text"])
        .output()
        .expect("cli should run");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("selected price (#3)"));
    assert!(stdout.contains("Diff: switched to regular price -> read selected after switch"));
    assert!(
        stdout.contains("edge selected price (#3) depends on sale price (#1) removed (Settled)")
    );
    assert!(
        stdout.contains("edge selected price (#3) depends on regular price (#2) added (Settled)")
    );
    assert!(stdout.contains("Diff: inactive sale price changed -> active regular price changed"));
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

#[test]
fn cli_rejects_unknown_scenarios() {
    let output = Command::new(env!("CARGO_BIN_EXE_attribute_graph_diff"))
        .args(["--scenario", "wat"])
        .output()
        .expect("cli should run");

    assert!(!output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf8");
    assert!(stderr.contains("unknown scenario"));
}
