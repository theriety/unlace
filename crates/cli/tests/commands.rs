//! Process-level behavior of the `unlace` binary: argument parsing, subcommand
//! dispatch, the fixed exit codes, and the error report printed on failure.
//!
//! Each test builds a small Python repository in a fresh temp dir, runs the real
//! binary against it, and checks the observable contract: exit code, stdout, and
//! stderr.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use unlace_engine::RESULT_SCHEMA_VERSION;

/// Two Python modules that import each other: a two-symbol dependency cycle.
const CYCLIC: &[(&str, &str)] = &[
    ("pkg/__init__.py", ""),
    (
        "pkg/a.py",
        "from pkg.b import beta\n\ndef alpha():\n    return beta()\n",
    ),
    (
        "pkg/b.py",
        "from pkg.a import alpha\n\ndef beta():\n    return alpha()\n",
    ),
];

/// A temp directory removed when dropped.
struct Repo {
    root: PathBuf,
}

impl Repo {
    /// Creates a fresh temp dir holding `files` as (relative path, contents).
    fn new(files: &[(&str, &str)]) -> Result<Self, String> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "unlace-cli-commands-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = Self { root };
        for (path, contents) in files {
            repo.write(path, contents)?;
        }
        Ok(repo)
    }

    /// Writes `contents` to `path` under the root, creating parent folders.
    fn write(&self, path: &str, contents: &str) -> Result<(), String> {
        let full = self.root.join(path);
        let parent = full.parent().unwrap_or(&self.root);
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        std::fs::write(&full, contents).map_err(|error| error.to_string())
    }

    /// Returns the absolute path of `path` under the root.
    fn path(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    /// Returns the root as a CLI argument.
    fn arg(&self) -> &str {
        self.root.to_str().unwrap_or_default()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The observable outcome of one finished `unlace` process.
struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Runs the binary with `args` and captures its exit code and output.
fn unlace(args: &[&str]) -> Result<Run, String> {
    let output = Command::new(env!("CARGO_BIN_EXE_unlace"))
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    Ok(Run {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Runs `analyze --format json` over `repo` with `extra` flags and returns the
/// saved result path.
fn saved_result(repo: &Repo, extra: &[&str]) -> Result<PathBuf, String> {
    let saved = repo.path("result.json");
    let saved_arg = saved.to_str().unwrap_or_default();
    let mut args = vec![
        "analyze",
        "--root",
        repo.arg(),
        "--format",
        "json",
        "--output",
        saved_arg,
    ];
    args.extend_from_slice(extra);
    let run = unlace(&args)?;
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(run.stdout.is_empty(), "--output keeps stdout clean");
    Ok(saved)
}

/// Returns the JSON value saved at `path`.
fn json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

/// Returns the value at JSON `pointer` in `value`, or `null` when absent.
fn at<'value>(value: &'value Value, pointer: &str) -> &'value Value {
    value.pointer(pointer).unwrap_or(&Value::Null)
}

#[test]
fn should_print_the_summary_report_and_exit_zero_by_default() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;

    let run = unlace(&["analyze", "--root", repo.arg()])?;

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(run.stdout.starts_with("Unlace report — "), "{}", run.stdout);
    assert!(run.stdout.contains("Shared findings (1)"), "{}", run.stdout);
    assert!(
        run.stdout
            .contains("cycle [violation] `pkg/a.py`/`pkg/b.py`"),
        "{}",
        run.stdout
    );
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    Ok(())
}

#[test]
fn should_render_the_summary_face_when_requested_explicitly() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;

    let run = unlace(&[
        "analyze",
        "--root",
        repo.arg(),
        "--format",
        "summary",
        "--verbose",
        "--full",
    ])?;

    assert_eq!(run.code, Some(0));
    assert!(run.stdout.starts_with("Unlace report — "), "{}", run.stdout);
    assert!(
        run.stdout.contains("Effective configuration"),
        "{}",
        run.stdout
    );
    Ok(())
}

#[test]
fn should_run_only_the_profiles_selected_by_mode() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;

    for (mode, anchored, greenfield) in [
        ("anchored", true, false),
        ("greenfield", false, true),
        ("both", true, true),
    ] {
        let value = json(&saved_result(&repo, &["--mode", mode])?)?;

        assert_eq!(
            at(&value, "/profiles/anchored").is_object(),
            anchored,
            "--mode {mode}"
        );
        assert_eq!(
            at(&value, "/profiles/greenfield").is_object(),
            greenfield,
            "--mode {mode}"
        );
    }
    Ok(())
}

#[test]
fn should_layer_search_flags_over_the_profile_defaults() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;

    let value = json(&saved_result(
        &repo,
        &[
            "--mode",
            "anchored",
            "--seed",
            "7",
            "-k",
            "2",
            "--jobs",
            "1",
            "--allow-cross-package-moves",
        ],
    )?)?;
    let parameters = at(&value, "/profiles/anchored/parameters");

    assert_eq!(at(parameters, "/seed"), 7);
    assert_eq!(at(parameters, "/candidates"), 2);
    assert_eq!(
        at(parameters, "/relocation/allow-cross-package-moves"),
        true
    );
    Ok(())
}

#[test]
fn should_load_the_root_config_and_reject_an_invalid_one() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;
    repo.write("unlace.toml", "[analysis]\nprofiles = [\"greenfield\"]\n")?;

    let value = json(&saved_result(&repo, &[])?)?;
    assert!(at(&value, "/profiles/anchored").is_null(), "{value}");
    assert!(at(&value, "/profiles/greenfield").is_object(), "{value}");

    repo.write("unlace.toml", "[analysis]\nprofiles = []\n")?;
    let run = unlace(&["analyze", "--root", repo.arg()])?;

    assert_eq!(run.code, Some(1));
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(
        run.stderr
            .starts_with("error[CONFIG_INVALID]: invalid configuration at `analysis.profiles`"),
        "{}",
        run.stderr
    );
    Ok(())
}

#[test]
fn should_validate_the_config_after_applying_cli_overrides() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;

    let run = unlace(&["analyze", "--root", repo.arg(), "-k", "0"])?;

    assert_eq!(run.code, Some(1));
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert_eq!(
        run.stderr,
        "error[CONFIG_INVALID]: invalid configuration at `profiles.anchored.candidates`: \
         must be at least 1\n"
    );
    Ok(())
}

#[test]
fn should_warn_and_use_defaults_for_a_missing_explicit_config() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;
    let missing = repo.path("absent.toml");
    let missing_arg = missing.to_str().unwrap_or_default();

    let run = unlace(&["analyze", "--root", repo.arg(), "--config", missing_arg])?;

    assert_eq!(run.code, Some(0));
    assert!(run.stdout.starts_with("Unlace report — "), "{}", run.stdout);
    assert_eq!(
        run.stderr,
        format!("warning: config {missing_arg} not found; using built-in defaults\n")
    );
    Ok(())
}

#[test]
fn should_fail_with_input_unreadable_when_the_output_cannot_be_written() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;
    let blocked = repo.path("missing-dir/result.json");

    let run = unlace(&[
        "analyze",
        "--root",
        repo.arg(),
        "--output",
        blocked.to_str().unwrap_or_default(),
    ])?;

    assert_eq!(run.code, Some(1));
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(
        run.stderr
            .starts_with("error[INPUT_UNREADABLE]: cannot read "),
        "{}",
        run.stderr
    );
    assert!(!blocked.exists());
    Ok(())
}

#[test]
fn should_report_an_adapter_failure_with_its_cause_chain() -> Result<(), String> {
    let repo = Repo::new(&[("pkg/broken.py", "def oops(:\n")])?;

    let run = unlace(&["analyze", "--root", repo.arg()])?;

    assert_eq!(run.code, Some(1));
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    let mut lines = run.stderr.lines();
    assert_eq!(
        lines.next(),
        Some("error[ADAPTER_PARSE_FAILURE]: adapter failed to parse source")
    );
    assert!(
        lines
            .next()
            .is_some_and(|line| line.starts_with("  caused by: ")),
        "{}",
        run.stderr
    );
    Ok(())
}

#[test]
fn should_exit_one_on_a_usage_error_and_zero_on_help() -> Result<(), String> {
    let usage = unlace(&["analyze", "--bogus-flag"])?;
    assert_eq!(
        usage.code,
        Some(1),
        "a usage error never takes the gating code 2"
    );
    assert!(usage.stdout.is_empty(), "{}", usage.stdout);
    assert!(usage.stderr.contains("--bogus-flag"), "{}", usage.stderr);

    let help = unlace(&["--help"])?;
    assert_eq!(help.code, Some(0));
    assert!(help.stdout.contains("Usage: unlace"), "{}", help.stdout);
    Ok(())
}

#[test]
fn should_list_print_and_reject_trees_of_a_saved_result() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;
    let saved = saved_result(&repo, &[])?;
    let input = saved.to_str().unwrap_or_default();

    let listing = unlace(&["tree", "--input", input])?;
    assert_eq!(listing.code, Some(0));
    assert_eq!(listing.stdout, "available candidates for anchored:\n");

    let current = unlace(&["tree", "--input", input, "--current", "--symbols"])?;
    assert_eq!(current.code, Some(0));
    for line in [
        " [packageGroup]",
        "\n    pkg [domain]",
        "\n      pkg [folder]",
        "\n        pkg/a.py [file] 1 sloc\n          - alpha (file)\n",
    ] {
        assert!(
            current.stdout.contains(line),
            "{line:?}: {}",
            current.stdout
        );
    }

    let shallow = unlace(&["tree", "--input", input, "--current", "--depth", "1"])?;
    assert_eq!(shallow.code, Some(0));
    assert!(
        shallow.stdout.contains(" [package]\n"),
        "{}",
        shallow.stdout
    );
    assert!(
        !shallow.stdout.contains("[domain]"),
        "depth 1 stops below the package: {}",
        shallow.stdout
    );

    let absent = unlace(&[
        "tree",
        "--input",
        input,
        "--mode",
        "greenfield",
        "--candidate",
        "1",
    ])?;
    assert_eq!(absent.code, Some(1));
    assert_eq!(
        absent.stderr,
        "error[CANDIDATE_NOT_FOUND]: no candidate at index 1 for mode greenfield\n"
    );
    Ok(())
}

#[test]
fn should_diff_the_current_layout_against_itself_as_no_moves() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;
    let saved = saved_result(&repo, &[])?;

    let run = unlace(&[
        "diff",
        "--input",
        saved.to_str().unwrap_or_default(),
        "current",
        "current",
    ])?;

    assert_eq!(run.code, Some(0));
    assert!(run.stdout.starts_with("no moves\n"), "{}", run.stdout);
    Ok(())
}

#[test]
fn should_gate_violations_only_on_a_listed_hard_class() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;

    let ungated = unlace(&["violations", "--root", repo.arg()])?;
    assert_eq!(ungated.code, Some(0));
    assert!(
        ungated
            .stdout
            .starts_with("cycle [violation] pkg/a.py, pkg/b.py :: "),
        "{}",
        ungated.stdout
    );

    let other_class = unlace(&[
        "violations",
        "--root",
        repo.arg(),
        "--fail-on",
        "capacity",
        "--full",
    ])?;
    assert_eq!(
        other_class.code,
        Some(0),
        "a cycle does not trip a capacity gate"
    );

    let listed = unlace(&[
        "violations",
        "--root",
        repo.arg(),
        "--fail-on",
        "cycle",
        "--jobs",
        "1",
    ])?;
    assert_eq!(listed.code, Some(2), "a listed hard cycle gates");
    Ok(())
}

#[test]
fn should_emit_violations_as_the_result_json() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;

    let run = unlace(&["violations", "--root", repo.arg(), "--format", "json"])?;
    let value: Value = serde_json::from_str(&run.stdout).map_err(|error| error.to_string())?;

    assert_eq!(run.code, Some(0));
    assert_eq!(at(&value, "/current/sharedFindings/0/kind"), "cycle");
    Ok(())
}

#[test]
fn should_reject_an_unknown_fail_on_class_as_invalid_config() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;

    let run = unlace(&["violations", "--root", repo.arg(), "--fail-on", "bogus"])?;

    assert_eq!(run.code, Some(1));
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(
        run.stderr
            .starts_with("error[CONFIG_INVALID]: invalid configuration at `--fail-on`: "),
        "{}",
        run.stderr
    );
    Ok(())
}

#[test]
fn should_render_a_saved_result_as_markdown() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;
    let saved = saved_result(&repo, &[])?;
    let input = saved.to_str().unwrap_or_default();

    let printed = unlace(&["report", "--input", input, "--verbose", "--full"])?;
    assert_eq!(printed.code, Some(0));
    assert!(
        printed.stdout.starts_with("# Unlace report — "),
        "{}",
        printed.stdout
    );
    assert!(
        printed.stdout.contains("## Effective configuration"),
        "{}",
        printed.stdout
    );

    let markdown = repo.path("report.md");
    let written = unlace(&[
        "report",
        "--input",
        input,
        "--output",
        markdown.to_str().unwrap_or_default(),
    ])?;
    assert_eq!(written.code, Some(0));
    assert!(written.stdout.is_empty(), "{}", written.stdout);
    let file = std::fs::read_to_string(&markdown).map_err(|error| error.to_string())?;
    assert!(file.starts_with("# Unlace report — "), "{file}");
    Ok(())
}

#[test]
fn should_refuse_a_saved_result_that_is_not_a_current_analysis() -> Result<(), String> {
    let repo = Repo::new(&[
        ("garbled.json", "{ not json"),
        ("future.json", "{\"schemaVersion\": 999}"),
    ])?;
    let hollow = format!("{{\"schemaVersion\": {RESULT_SCHEMA_VERSION}}}");
    repo.write("hollow.json", &hollow)?;
    let future = format!("unsupported result schemaVersion 999; expected {RESULT_SCHEMA_VERSION}");

    for (name, reason) in [
        ("absent.json", "No such file or directory"),
        ("garbled.json", "key must be a string"),
        ("future.json", future.as_str()),
        ("hollow.json", "missing field"),
    ] {
        let input = repo.path(name);
        let run = unlace(&["report", "--input", input.to_str().unwrap_or_default()])?;

        assert_eq!(run.code, Some(1), "{name}");
        assert!(run.stdout.is_empty(), "{name}: {}", run.stdout);
        assert!(
            run.stderr
                .starts_with("error[INPUT_UNREADABLE]: cannot read "),
            "{}",
            run.stderr
        );
        assert!(run.stderr.contains(reason), "{name}: {}", run.stderr);
    }
    Ok(())
}

#[test]
fn should_honor_an_explicit_config_that_exists_without_a_warning() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;
    repo.write("custom.toml", "[analysis]\nprofiles = [\"greenfield\"]\n")?;
    let custom = repo.path("custom.toml");

    let run = unlace(&[
        "analyze",
        "--root",
        repo.arg(),
        "--format",
        "json",
        "--config",
        custom.to_str().unwrap_or_default(),
    ])?;
    let value: Value = serde_json::from_str(&run.stdout).map_err(|error| error.to_string())?;

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert!(at(&value, "/profiles/anchored").is_null(), "{value}");
    assert!(at(&value, "/profiles/greenfield").is_object(), "{value}");
    Ok(())
}

#[test]
fn should_refuse_an_empty_root_without_writing_the_output_file() -> Result<(), String> {
    let repo = Repo::new(&[("notes.txt", "no sources here\n")])?;
    let saved = repo.path("result.json");

    let run = unlace(&[
        "analyze",
        "--root",
        repo.arg(),
        "--output",
        saved.to_str().unwrap_or_default(),
    ])?;

    assert_eq!(run.code, Some(1));
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(
        run.stderr
            .starts_with("error[SNAPSHOT_INVALID]: snapshot failed validation\n"),
        "{}",
        run.stderr
    );
    assert!(!saved.exists());
    Ok(())
}

#[test]
fn should_exit_one_when_tree_or_diff_cannot_read_the_saved_result() -> Result<(), String> {
    let repo = Repo::new(&[])?;
    let absent = repo.path("absent.json");
    let input = absent.to_str().unwrap_or_default();

    for args in [
        vec!["tree", "--input", input, "--current"],
        vec!["diff", "--input", input, "current", "current"],
    ] {
        let run = unlace(&args)?;

        assert_eq!(run.code, Some(1), "{args:?}");
        assert!(run.stdout.is_empty(), "{args:?}: {}", run.stdout);
        assert!(
            run.stderr
                .starts_with(&format!("error[INPUT_UNREADABLE]: cannot read {input}: ")),
            "{args:?}: {}",
            run.stderr
        );
    }
    Ok(())
}

#[test]
fn should_exit_one_when_violations_cannot_configure_snapshot_or_analyze_the_root()
-> Result<(), String> {
    let invalid = Repo::new(CYCLIC)?;
    invalid.write("unlace.toml", "[analysis]\nprofiles = []\n")?;
    let empty = Repo::new(&[("notes.txt", "no sources here\n")])?;
    let missing = Repo::new(&[])?;
    let missing_root = missing.path("gone");

    for (root, expected) in [
        (
            invalid.arg(),
            "error[CONFIG_INVALID]: invalid configuration at `analysis.profiles`",
        ),
        (
            missing_root.to_str().unwrap_or_default(),
            "error[INPUT_UNREADABLE]: cannot read ",
        ),
        (
            empty.arg(),
            "error[SNAPSHOT_INVALID]: snapshot failed validation\n",
        ),
    ] {
        let run = unlace(&["violations", "--root", root, "--fail-on", "cycle"])?;

        assert_eq!(run.code, Some(1), "{root}");
        assert!(run.stdout.is_empty(), "{root}: {}", run.stdout);
        assert!(run.stderr.starts_with(expected), "{root}: {}", run.stderr);
    }
    Ok(())
}

#[test]
fn should_gate_violations_on_the_findings_of_whichever_single_profile_runs() -> Result<(), String> {
    let repo = Repo::new(CYCLIC)?;

    for profile in ["anchored", "greenfield"] {
        repo.write(
            "unlace.toml",
            &format!("[analysis]\nprofiles = [\"{profile}\"]\n"),
        )?;

        let run = unlace(&["violations", "--root", repo.arg(), "--fail-on", "cycle"])?;

        assert_eq!(run.code, Some(2), "{profile}: {}", run.stderr);
        assert!(
            run.stdout
                .starts_with("cycle [violation] pkg/a.py, pkg/b.py :: "),
            "{profile}: {}",
            run.stdout
        );
    }
    Ok(())
}
