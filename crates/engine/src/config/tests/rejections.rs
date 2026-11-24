//! Rejection contracts for `unlace.toml`, exercised through [`load_config`].
//!
//! These live in the library's unit-test build, beside the other validation
//! tests, so one build exercises every validation branch.
//!
//! Every out-of-range value must surface as `ConfigInvalid` attributed to the
//! exact dotted key a user would edit, so each case below names the offending
//! document, the key it must be blamed on, and a fragment of the explanation.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::config::{AnalyzeConfig, Mode, ProfileName, load_config};
use crate::error::UnlaceError;

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

fn fixture_path() -> PathBuf {
    let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "unlace-config-rejection-{}-{sequence}.toml",
        std::process::id()
    ))
}

fn load_toml(document: &str) -> Result<AnalyzeConfig, UnlaceError> {
    let path = fixture_path();
    let written = fs::write(&path, document);
    assert!(
        written.is_ok(),
        "the temporary configuration must be writable"
    );
    let loaded = load_config(&path);
    let _removed = fs::remove_file(path);
    loaded
}

/// Asserts that `document` is rejected at `expected_key` with a reason
/// containing `expected_reason`.
fn assert_rejected_at(document: &str, expected_key: Option<&str>, expected_reason: &str) {
    let loaded = load_toml(document);
    assert!(
        matches!(
            &loaded,
            Err(UnlaceError::ConfigInvalid { key, reason })
                if key.as_deref() == expected_key && reason.contains(expected_reason)
        ),
        "document {document:?} must be rejected at {expected_key:?} with {expected_reason:?}; got {loaded:?}"
    );
}

#[test]
fn should_require_at_least_one_selected_profile() {
    assert_rejected_at(
        "[analysis]\nprofiles = []\n",
        Some("analysis.profiles"),
        "at least one parameter profile must be selected",
    );
}

#[test]
fn should_reject_a_profile_selected_twice() {
    assert_rejected_at(
        "[analysis]\nprofiles = [\"greenfield\", \"anchored\", \"greenfield\"]\n",
        Some("analysis.profiles"),
        "parameter profile names must be unique",
    );
}

#[test]
fn should_require_at_least_one_enabled_language() {
    assert_rejected_at(
        "[adapters]\nlanguages = []\n",
        Some("adapters.languages"),
        "at least one language must be enabled",
    );
}

#[test]
fn should_attribute_every_out_of_range_profile_value_to_its_key() {
    let cases = [
        ("capacity", "folder = 0", "capacity.folder", "at least 1"),
        ("capacity", "domain = 0", "capacity.domain", "at least 1"),
        ("capacity", "package = 0", "capacity.package", "at least 1"),
        (
            "capacity",
            "package-group = 0",
            "capacity.package-group",
            "at least 1",
        ),
        (
            "solver",
            "ilp-threshold = 0",
            "solver.ilp-threshold",
            "at least 1",
        ),
        (
            "objective",
            "imbalance = -0.5",
            "objective.imbalance",
            "non-negative",
        ),
        ("objective", "path = -0.5", "objective.path", "non-negative"),
        (
            "objective",
            "anchor = -0.5",
            "objective.anchor",
            "non-negative",
        ),
        (
            "objective",
            "capacity = -0.5",
            "objective.capacity",
            "non-negative",
        ),
        (
            "weights",
            "value-import = -1.0",
            "weights.value-import",
            "non-negative",
        ),
        (
            "weights",
            "inheritance = -1.0",
            "weights.inheritance",
            "non-negative",
        ),
        ("weights", "call = -1.0", "weights.call", "non-negative"),
        (
            "weights",
            "type-reference = -1.0",
            "weights.type-reference",
            "non-negative",
        ),
        (
            "weights",
            "re-export = -1.0",
            "weights.re-export",
            "non-negative",
        ),
        (
            "weights",
            "same-file-symbol = 0.5",
            "weights.same-file-symbol",
            "at least 1",
        ),
        (
            "diversity",
            "score-tolerance = -0.1",
            "diversity.score-tolerance",
            "non-negative",
        ),
        (
            "diversity",
            "min-distance = -0.1",
            "diversity.min-distance",
            "non-negative",
        ),
        (
            "diversity",
            "seeds-per-candidate = 0",
            "diversity.seeds-per-candidate",
            "at least 1",
        ),
    ];

    for profile in ["anchored", "greenfield"] {
        for (section, assignment, suffix, reason) in cases {
            assert_rejected_at(
                &format!("[profiles.{profile}.{section}]\n{assignment}\n"),
                Some(&format!("profiles.{profile}.{suffix}")),
                reason,
            );
        }
    }
}

#[test]
fn should_require_at_least_one_positive_qualification_weight() {
    assert_rejected_at(
        "[profiles.greenfield.qualification.weights]\n\
         unique-owner = 0.0\nrole-affinity = 0.0\nsource-cohesion = 0.0\n\
         destination-cohesion = 0.0\nproducer-evidence = 0.0\narchitectural-reach = 0.0\n",
        Some("profiles.greenfield.qualification.weights"),
        "at least one qualification weight must be positive",
    );
}

#[test]
fn should_reject_a_mirror_template_with_an_empty_path_segment() {
    assert_rejected_at(
        "[[profiles.anchored.relocation.test-mirroring.rules]]\n\
         source = \"src//{dir}/{stem}.ts\"\ntests = [\"spec/{dir}/{stem}.spec.ts\"]\n",
        Some("profiles.anchored.relocation.test-mirroring.rules[0].source"),
        "mirror template grammar",
    );
}

#[test]
fn should_reject_an_unknown_profile_table() {
    assert_rejected_at(
        "[profiles.experimental]\ncandidates = 3\n",
        None,
        "unknown field `experimental`",
    );
}

#[test]
fn should_reject_a_mistyped_value_inside_either_profile() {
    for profile in ["anchored", "greenfield"] {
        assert_rejected_at(
            &format!("[profiles.{profile}]\ncandidates = \"three\"\n"),
            None,
            "invalid type",
        );
    }
}

#[test]
fn should_reject_a_mistyped_top_level_value_without_a_key() {
    assert_rejected_at("[analysis]\njobs = \"many\"\n", None, "invalid type");
}

#[test]
fn should_reject_malformed_toml_without_a_key() {
    assert_rejected_at("[analysis\njobs = 1\n", None, "");
}

#[test]
fn should_load_a_valid_profile_override_from_disk() -> Result<(), String> {
    let config = load_toml("[profiles.anchored.capacity]\nfolder = 9\n")
        .map_err(|error| error.to_string())?;

    assert_eq!(config.profiles.anchored.capacity.folder, 9);
    assert_eq!(
        config.profiles.greenfield.capacity.folder,
        AnalyzeConfig::default().profiles.greenfield.capacity.folder
    );
    Ok(())
}

#[test]
fn should_report_a_missing_configuration_file_as_unreadable() {
    let path = fixture_path();

    let loaded = load_config(&path);

    assert!(
        matches!(&loaded, Err(UnlaceError::InputUnreadable { path: reported, reason })
            if *reported == path && !reason.is_empty()),
        "{loaded:?}"
    );
}

#[test]
fn should_select_both_profiles_for_the_both_mode() {
    let mut config = AnalyzeConfig::default();
    config.select_mode(Mode::Anchored);

    config.select_mode(Mode::Both);

    assert_eq!(
        config.analysis.profiles,
        vec![ProfileName::Anchored, ProfileName::Greenfield]
    );
}
