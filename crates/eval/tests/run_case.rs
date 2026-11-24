//! `harness::run_case` contracts beyond the committed gate: engine and config
//! failures abort the case, census-dependent target defects surface as case
//! errors, run modes select the evaluated faces, and failed verdicts read as
//! findings in the aggregated failure message.

use std::error::Error;
use std::path::{Path, PathBuf};

use unlace_eval::{CaseReport, EvalError, TargetSpec, eval_fixture_root, harness};

type TestResult = Result<(), Box<dyn Error>>;

/// The inversion fixture's header: seven files, both faces, built-in defaults.
const INVERSION_HEADER: &str = r#"
schema = 1
fixture = "inversion"

[meta]
failure_mode = "anchored-inversion"
language = "python"
question = "Does the harness measure the probe?"

[run]
mode = "both"
candidates = 3
seed = 42
config = "defaults"

[[precondition]]
kind = "file_count"
min = 7
max = 7
because = "two files per feature directory plus the facade"
"#;

/// A minimal block every face-agnostic probe can extend.
const ANCHORED_BLOCK: &str = r#"
[[assert]]
modes = ["anchored"]
"#;

fn spec(document: &str) -> Result<TargetSpec, toml::de::Error> {
    toml::from_str(document)
}

/// Runs `document` against the inversion fixture under the `inversion` name.
fn run_inversion(document: &str) -> Result<CaseReport, Box<dyn Error>> {
    Ok(harness::run_case(
        &eval_fixture_root("inversion"),
        &spec(document)?,
        "inversion",
    )?)
}

/// A scratch directory removed when dropped, so no test leaves state behind
/// and nothing under the committed corpus is ever written.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!("unlace-eval-{tag}-{}", std::process::id()));
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
        }
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Best effort: a leftover temp directory is harmless.
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// Copies a fixture tree so a test can add an `unlace.toml` without touching
/// the committed corpus.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[test]
fn an_unreadable_fixture_root_aborts_as_an_engine_failure() -> TestResult {
    let spec = spec(&format!("{INVERSION_HEADER}{ANCHORED_BLOCK}"))?;
    let error = harness::run_case(&eval_fixture_root("no-such-fixture"), &spec, "inversion")
        .err()
        .ok_or("a missing root was analyzed")?;
    assert!(
        matches!(&error, EvalError::EngineRun { fixture, .. } if fixture == "inversion"),
        "an unreadable root is an engine failure, never distance: {error}"
    );

    // A readable root with no sources snapshots, but analysis has no
    // container tree to render.
    let empty = Scratch::new("empty-root")?;
    let analyze_error = harness::run_case(&empty.0, &spec, "inversion")
        .err()
        .ok_or("an empty root was analyzed")?;
    assert!(
        matches!(&analyze_error, EvalError::EngineRun { fixture, message }
            if fixture == "inversion" && message.contains("snapshot failed validation")),
        "an unanalyzable snapshot is an engine failure: {analyze_error}"
    );
    Ok(())
}

#[test]
fn fixture_config_source_reads_the_fixture_unlace_toml() -> TestResult {
    let document = format!(
        "{}{ANCHORED_BLOCK}",
        INVERSION_HEADER.replace(r#"config = "defaults""#, r#"config = "fixture""#)
    );
    let spec = spec(&document)?;

    let valid = Scratch::new("fixture-config")?;
    copy_tree(&eval_fixture_root("inversion"), &valid.0)?;
    std::fs::write(valid.0.join("unlace.toml"), "[analysis]\njobs = 1\n")?;
    let report = harness::run_case(&valid.0, &spec, "inversion")?;
    assert_eq!(
        report.failure_message(),
        None,
        "a valid fixture config analyzes like the defaults"
    );
    assert_eq!(report.preconditions.len(), 1);

    let broken = Scratch::new("fixture-config-broken")?;
    copy_tree(&eval_fixture_root("inversion"), &broken.0)?;
    std::fs::write(broken.0.join("unlace.toml"), "[analysis\n")?;
    let error = harness::run_case(&broken.0, &spec, "inversion")
        .err()
        .ok_or("a malformed unlace.toml was accepted")?;
    assert!(
        matches!(&error, EvalError::EngineRun { fixture, message }
            if fixture == "inversion" && message.starts_with("loading ") && message.contains("unlace.toml")),
        "a malformed fixture config aborts the case: {error}"
    );

    // A config source of `fixture` with no unlace.toml is unreadable input.
    let missing = Scratch::new("fixture-config-missing")?;
    copy_tree(&eval_fixture_root("inversion"), &missing.0)?;
    let missing_error = harness::run_case(&missing.0, &spec, "inversion")
        .err()
        .ok_or("a missing unlace.toml was accepted")?;
    assert!(
        missing_error.to_string().contains("unlace.toml"),
        "the missing config path is named: {missing_error}"
    );
    Ok(())
}

#[test]
fn single_face_runs_produce_only_their_configured_profile() -> TestResult {
    // `preserve_dir` may override its face without validation checking the
    // run, so an override naming the other face reaches the engine result and
    // proves which profile the run actually produced.
    for (run, block_face, missing) in [
        ("anchored", "anchored", "Greenfield"),
        ("greenfield", "greenfield", "Anchored"),
    ] {
        let other = missing.to_lowercase();
        let error = run_inversion(&format!(
            "{}\n[[assert]]\nmodes = [\"{block_face}\"]\n\n[[assert.preserve_dir]]\npath = \"geometry\"\nmode = \"{other}\"\nbecause = \"geometry survives\"\n",
            INVERSION_HEADER.replace(r#"mode = "both""#, &format!("mode = \"{run}\""))
        ))
        .err()
        .ok_or("an unproduced face was evaluated")?;
        assert!(
            error.to_string().contains(&format!(
                "assertions need the {missing} mode but [run].mode did not produce it"
            )),
            "a {run} run must not produce the {missing} profile: {error}"
        );
    }
    Ok(())
}

#[test]
fn a_failed_precondition_is_a_case_error_not_distance() -> TestResult {
    let report = run_inversion(&format!(
        "{}{ANCHORED_BLOCK}",
        INVERSION_HEADER.replace("min = 7\nmax = 7", "min = 8\nmax = 9")
    ))?;
    assert!(
        report.preconditions.iter().all(|verdict| !verdict.passed),
        "seven files lie outside [8, 9]"
    );
    assert!(
        report.errors.iter().any(|error| error
            .to_string()
            .contains("precondition `file_count` failed against current")),
        "the failed precondition lands in errors: {:?}",
        report.errors
    );
    let message = report.failure_message().ok_or("the case reads clean")?;
    assert!(message.contains("ERROR target `inversion` is invalid: precondition `file_count`"));
    assert!(message.contains("PRECONDITION file_count :: summary.files = 7 against [8, 9]"));
    Ok(())
}

#[test]
fn census_references_that_do_not_resolve_are_case_errors() -> TestResult {
    let report = run_inversion(&format!(
        r#"{INVERSION_HEADER}{ANCHORED_BLOCK}
[[assert.separate]]
paths = ["geometry/shape.py", "geometry/missing.py"]
because = "a misspelled path"

[[assert.preserve_dir]]
path = "nowhere"
because = "a directory the fixture does not have"

[[assert.preserve_symbol_home]]
symbol = "Shape"
path = "metrics/counter.py"
because = "a home the current layout does not show"

[[assert.preserve_symbol_home]]
symbol = "Shape"
path = "geometry/shape.py"
because = "Shape lives in geometry/shape.py today"

[[reference.container]]
path = "ghost"
files = ["ghost.py"]
"#
    ))?;
    let errors: Vec<String> = report.errors.iter().map(ToString::to_string).collect();
    for expected in [
        r#"keep_together/separate references "geometry/missing.py", which the fixture census does not contain"#,
        r#"reference.container references "ghost.py", which the fixture census does not contain"#,
        r#"preserve_dir "nowhere" resolves under no analyzed package"#,
        r#"preserve_symbol_home pins "Shape" in "metrics/counter.py", which the current layout does not show"#,
    ] {
        assert!(
            errors.iter().any(|error| error.contains(expected)),
            "missing {expected:?} among {errors:?}"
        );
    }
    assert_eq!(errors.len(), 4, "the real Shape home is not an error");
    let real_home = report
        .verdicts
        .iter()
        .filter(|verdict| verdict.label == "preserve_symbol_home(Shape)#Anchored")
        .map(|verdict| verdict.passed)
        .collect::<Vec<_>>();
    assert_eq!(real_home, vec![false, true], "only the real home holds");
    Ok(())
}

#[test]
fn an_assertion_mode_override_adds_its_face_to_the_evaluation() -> TestResult {
    let report = run_inversion(&format!(
        r#"{INVERSION_HEADER}{ANCHORED_BLOCK}
[[assert.keep_together]]
paths = ["geometry/shape.py", "geometry/area.py"]
mode = "greenfield"
because = "geometry stays whole"
"#
    ))?;
    let labels: Vec<&str> = report
        .verdicts
        .iter()
        .map(|verdict| verdict.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec!["keep_together(geometry/shape.py+geometry/area.py)#Greenfield"]
    );
    assert_eq!(
        report.observations.len(),
        2,
        "both the block face and the override face are observed"
    );
    Ok(())
}

#[test]
fn failed_assertions_read_as_findings_in_the_failure_message() -> TestResult {
    let report = run_inversion(&format!(
        r#"{INVERSION_HEADER}{ANCHORED_BLOCK}
[[assert.keep_together]]
paths = ["geometry/shape.py", "metrics/counter.py"]
because = "a deliberately false co-location"
"#
    ))?;
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let message = report
        .failure_message()
        .ok_or("the false co-location held")?;
    assert_eq!(
        message,
        "ASSERT keep_together(geometry/shape.py+metrics/counter.py)#Anchored :: \
         no folder/domain container holds all of [geometry/shape.py, metrics/counter.py]"
    );
    Ok(())
}

#[test]
fn an_invalid_target_aborts_before_the_engine_runs() -> TestResult {
    // The root does not exist, so only validation can produce this error.
    let error = harness::run_case(
        &eval_fixture_root("no-such-fixture"),
        &spec(&format!("{INVERSION_HEADER}{ANCHORED_BLOCK}"))?,
        "tearing",
    )
    .err()
    .ok_or("a mismatched target ran")?;
    assert!(
        matches!(&error, EvalError::TargetInvalid { target, message }
            if target == "tearing" && message.contains("does not match the invoked case name")),
        "a renamed fixture fails loud at validation: {error}"
    );
    Ok(())
}

#[test]
fn a_broken_case_reports_only_its_abort_error() {
    let report = CaseReport::broken(
        "tearing",
        EvalError::EngineRun {
            fixture: "tearing".to_owned(),
            message: "discovery failed".to_owned(),
        },
    );
    assert_eq!(report.fixture, "tearing");
    assert!(report.verdicts.is_empty() && report.preconditions.is_empty());
    assert!(report.pair_f1.is_empty() && report.observations.is_empty());
    assert_eq!(
        report.failure_message().as_deref(),
        Some("ERROR fixture `tearing` could not be analyzed: discovery failed")
    );
}
