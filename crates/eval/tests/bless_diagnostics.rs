//! `UNLACE_BLESS_EVAL` regenerates a diagnostic dump under the workspace
//! `target/` and nothing else; a dump that cannot be written is a case error.
//!
//! The variable is never set in this process: the blessed run re-executes this
//! test binary as a child with the variable in the child's environment only,
//! so no test here (or anywhere) races on process-global environment state.

use std::error::Error;
use std::path::PathBuf;
use std::process::Command;

use unlace_eval::{EvalError, eval_fixture_root, harness, load_target};

type TestResult = Result<(), Box<dyn Error>>;

const BLESS_ENV: &str = "UNLACE_BLESS_EVAL";

/// The ignored test the blessed parent runs in a child process.
const BLESSED_CHILD: &str = "blessed_child_writes_and_reports_dumps";

/// Where the harness writes dumps: the workspace `target/`, never the corpus.
fn diagnostics_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("unlace-eval-diagnostics")
}

/// Runs the committed inversion target under a probe fixture name so the dump
/// lands at a probe-owned path.
fn run_probe(fixture: &str) -> Result<unlace_eval::CaseReport, Box<dyn Error>> {
    let mut spec = load_target("inversion")?;
    fixture.clone_into(&mut spec.fixture);
    Ok(harness::run_case(
        &eval_fixture_root("inversion"),
        &spec,
        fixture,
    )?)
}

#[test]
fn without_the_bless_environment_no_dump_is_written() -> TestResult {
    let fixture = "bless-probe-plain";
    let dump = diagnostics_dir().join(format!("{fixture}.json"));
    std::fs::remove_file(&dump).ok();

    let report = run_probe(fixture)?;
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(!dump.exists(), "an unblessed run never writes {dump:?}");
    Ok(())
}

#[test]
fn the_bless_environment_regenerates_the_diagnostic_dump() -> TestResult {
    let output = Command::new(std::env::current_exe()?)
        .args([BLESSED_CHILD, "--exact", "--ignored"])
        .env(BLESS_ENV, "1")
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "the blessed child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

/// Runs only inside the child [`the_bless_environment_regenerates_the_diagnostic_dump`]
/// spawns with the bless variable set.
#[test]
#[ignore = "spawned with UNLACE_BLESS_EVAL by the_bless_environment_regenerates_the_diagnostic_dump"]
fn blessed_child_writes_and_reports_dumps() -> TestResult {
    std::env::var_os(BLESS_ENV).ok_or("run through the blessed parent test")?;

    let fixture = "bless-probe-blessed";
    let dump = diagnostics_dir().join(format!("{fixture}.json"));
    std::fs::remove_file(&dump).ok();
    let report = run_probe(fixture)?;
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let document: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&dump)?)?;
    std::fs::remove_file(&dump)?;
    assert_eq!(
        document
            .pointer("/fixture")
            .and_then(serde_json::Value::as_str),
        Some(fixture)
    );
    let dumped_labels: Vec<&str> = document
        .pointer("/verdicts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|verdict| {
            verdict
                .pointer("/label")
                .and_then(serde_json::Value::as_str)
        })
        .collect();
    let reported_labels: Vec<&str> = report
        .verdicts
        .iter()
        .map(|verdict| verdict.label.as_str())
        .collect();
    assert_eq!(
        dumped_labels, reported_labels,
        "the dump mirrors the verdicts"
    );
    assert_eq!(
        document
            .pointer("/preconditions/0/passed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        document
            .pointer("/observations")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(report.observations.len())
    );
    assert_eq!(
        document
            .pointer("/pair_f1")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(report.pair_f1.len())
    );
    assert!(
        document.pointer("/result/current").is_some(),
        "the dump carries the full engine result"
    );

    // A fixture name whose dump path has no parent directory cannot be
    // written; the failure lands in the report instead of vanishing.
    let unwritable = "bless-missing-dir/probe";
    let failed = run_probe(unwritable)?;
    assert!(
        failed.errors.iter().any(
            |error| matches!(error, EvalError::DumpFailed { fixture: dumped, .. } if dumped == unwritable)
        ),
        "the dump failure is a case error: {:?}",
        failed.errors
    );
    assert!(
        failed.failure_message().is_some_and(
            |message| message.contains("diagnostics dump for `bless-missing-dir/probe` failed")
        ),
        "the dump failure reads as a finding"
    );
    Ok(())
}
