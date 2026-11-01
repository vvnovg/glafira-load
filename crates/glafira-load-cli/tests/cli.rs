//! `glafira-load check|smoke|run`: коды возврата и файлы прогона (спека §5.4, §9.1).

mod common;

use assert_cmd::Command;
use axum::http::StatusCode;
use common::{stand_stub, temp_project, write_prod_load, write_profile};
use predicates::prelude::*;

fn cli() -> Command {
    Command::from_std(std::process::Command::new(env!(
        "CARGO_BIN_EXE_glafira-load"
    )))
}

#[test]
fn version_and_bad_usage() {
    cli()
        .arg("--version")
        .assert()
        .code(0)
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
    // без --env: ошибка разбора → 3, не clap-овский 2
    let project = temp_project("http://127.0.0.1:1");
    cli()
        .args(["run", project.path().to_str().unwrap(), "--profile", "peak"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("--env"));
    cli()
        .args(["check", "/no/such/project"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("error"));
}

#[test]
fn check_orders_suite_passes() {
    let project = temp_project("http://127.0.0.1:1");
    cli()
        .args([
            "check",
            project.path().to_str().unwrap(),
            "--suite",
            "orders",
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("0 error(s)"));
}

#[test]
fn check_reports_missing_operation_with_code_4() {
    let project = temp_project("http://127.0.0.1:1");
    let bad = project.path().join("suites/bad");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::copy(
        common::fixtures().join("invalid/missing-operation.yaml"),
        bad.join("suite.yaml"),
    )
    .unwrap();
    cli()
        .args(["check", project.path().to_str().unwrap()])
        .assert()
        .code(4)
        .stdout(
            predicate::str::contains("error")
                .and(predicate::str::contains("suites/bad/suite.yaml")),
        );
}

#[tokio::test(flavor = "multi_thread")]
async fn smoke_against_stub_writes_smoke_yaml() {
    let stand = stand_stub(StatusCode::CREATED).await;
    let project = temp_project(&format!("http://{stand}"));
    let out = project.path().join("out");
    cli()
        .args([
            "smoke",
            project.path().to_str().unwrap(),
            "--suite",
            "orders-http",
            "--env",
            "test",
            "--out",
            out.to_str().unwrap(),
        ])
        .env("GLAFIRA_LOAD_SECRET_API_TOKEN", "t")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("0 step(s) failed"));
    let report = std::fs::read_to_string(out.join("smoke.yaml")).unwrap();
    assert!(
        report.contains("ok: true") && !report.contains("ok: false"),
        "{report}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn smoke_with_failing_stand_returns_4() {
    let stand = stand_stub(StatusCode::INTERNAL_SERVER_ERROR).await;
    let project = temp_project(&format!("http://{stand}"));
    let out = project.path().join("out");
    cli()
        .args([
            "smoke",
            project.path().to_str().unwrap(),
            "--suite",
            "orders-http",
            "--env",
            "test",
            "--out",
            out.to_str().unwrap(),
        ])
        .env("GLAFIRA_LOAD_SECRET_API_TOKEN", "t")
        .assert()
        .code(4)
        .stdout(
            predicate::str::contains("step(s) failed")
                .and(predicate::str::contains("0 step(s) failed").not()),
        );
    let report = std::fs::read_to_string(out.join("smoke.yaml")).unwrap();
    assert!(
        report.contains("ok: false") && report.contains("code: SKIPPED"),
        "first step failed, second skipped: {report}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn run_three_seconds_writes_summary() {
    let stand = stand_stub(StatusCode::CREATED).await;
    let project = temp_project(&format!("http://{stand}"));
    write_profile(project.path(), "short", 0.05);
    write_prod_load(project.path());
    let out = project.path().join("out");
    cli()
        .args([
            "run",
            project.path().to_str().unwrap(),
            "--profile",
            "short",
            "--env",
            "test",
            "--suite",
            "orders-http",
            "--out",
            out.to_str().unwrap(),
        ])
        .env("GLAFIRA_LOAD_SECRET_API_TOKEN", "t")
        .timeout(std::time::Duration::from_secs(60))
        .assert()
        .code(0)
        .stdout(
            predicate::str::contains("t=")
                .and(predicate::str::contains("operation"))
                .and(predicate::str::contains("status: completed")),
        );
    assert!(out.join("summary.yaml").is_file());
    assert!(out.join("run.yaml").is_file());
}
