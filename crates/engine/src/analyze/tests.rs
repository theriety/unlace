#![allow(clippy::assertions_on_constants)]

use unlace_ir::{Polarity, ScopeLevel};

use crate::config::AnalyzeConfig;

use super::*;
use crate::analyze::test_support::*;

#[test]
fn should_be_deterministic_in_its_hash() {
    let make = || {
        snapshot(
            vec![node(0, "a", 0, Polarity::Production)],
            vec![],
            vec![container(0, "file", ScopeLevel::File, None)],
        )
    };

    let first = analyze(&make(), &AnalyzeConfig::default())
        .map(|result| result.snapshot_hash)
        .unwrap_or_default();
    let second = analyze(&make(), &AnalyzeConfig::default())
        .map(|result| result.snapshot_hash)
        .unwrap_or_default();

    assert_eq!(first, second);
    assert_ne!(first, "");
}

#[test]
fn should_serialize_the_resolved_tests_section_once_at_the_top_level() {
    let snapshot = snapshot(
        vec![node(0, "item", 0, Polarity::Production)],
        vec![],
        vec![container(0, "src/item.rs", ScopeLevel::File, None)],
    );
    let mut config = AnalyzeConfig::default();
    config.tests.patterns = vec!["checks/**".to_owned(), "!checks/keep.ts".to_owned()];
    config.tests.builtins = false;

    let serialized = analyze(&snapshot, &config)
        .ok()
        .and_then(|result| serde_json::to_value(result).ok())
        .unwrap_or(serde_json::Value::Null);

    assert_eq!(
        serialized.get("tests"),
        Some(&serde_json::json!({
            "helperCap": 600,
            "patterns": ["checks/**", "!checks/keep.ts"],
            "builtins": false
        }))
    );
    assert!(
        serialized
            .pointer("/profiles/anchored/parameters/tests")
            .is_none(),
        "no per-profile copy"
    );
}

#[test]
fn should_serialize_profile_results_as_schema_version_twelve_without_modes() {
    let snapshot = snapshot(
        vec![node(0, "item", 0, Polarity::Production)],
        vec![],
        vec![container(0, "src/item.rs", ScopeLevel::File, None)],
    );

    let serialized = analyze(&snapshot, &AnalyzeConfig::default())
        .ok()
        .and_then(|result| serde_json::to_value(result).ok())
        .unwrap_or(serde_json::Value::Null);

    assert_eq!(
        serialized
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64),
        Some(12)
    );
    // version 9 carries the per-profile package-wall parameter (ADR-17).
    assert_eq!(
        serialized
            .pointer("/profiles/anchored/parameters/relocation/allow-cross-package-moves")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert!(serialized.pointer("/current/tree").is_some());
    assert!(serialized.pointer("/current/sharedFindings").is_some());
    assert!(serialized.get("profiles").is_some());
    assert!(serialized.pointer("/advice/recommended").is_some());
    assert!(serialized.pointer("/advice/reviewCandidates").is_some());
    assert!(serialized.pointer("/advice/rejected").is_none());
    assert!(serialized.get("modes").is_none());
}

/// Two cyclic files in two folders: enough structure for findings and moves.
fn cyclic_pair_snapshot() -> Snapshot {
    snapshot(
        vec![
            node(0, "alpha", 2, Polarity::Production),
            node(1, "beta", 3, Polarity::Production),
        ],
        vec![edge(0, 1), edge(1, 0)],
        vec![
            container(0, "left", ScopeLevel::Folder, None),
            container(1, "right", ScopeLevel::Folder, None),
            container(2, "left/alpha.ts", ScopeLevel::File, Some(0)),
            container(3, "right/beta.ts", ScopeLevel::File, Some(1)),
        ],
    )
}

#[test]
fn should_produce_identical_output_in_a_scoped_pool_and_the_global_pool() {
    let snapshot = cyclic_pair_snapshot();
    let mut scoped = AnalyzeConfig::default();
    scoped.analysis.jobs = 2;
    let serialize = |config: &AnalyzeConfig| {
        analyze(&snapshot, config)
            .ok()
            .and_then(|result| serde_json::to_string(&result).ok())
    };

    let global_output = serialize(&AnalyzeConfig::default());
    let scoped_output = serialize(&scoped);

    assert!(global_output.is_some(), "the analysis succeeds");
    assert_eq!(scoped_output, global_output);
}

#[test]
fn should_reject_an_unvalidated_config_with_an_invalid_test_glob() {
    let mut config = AnalyzeConfig::default();
    config.tests.patterns = vec!["checks/**".to_owned(), "checks/[".to_owned()];

    let result = analyze(&cyclic_pair_snapshot(), &config);

    assert!(
        matches!(&result, Err(UnlaceError::ConfigInvalid { key: Some(key), .. })
            if key == "tests.patterns[1]"),
        "{:?}",
        result.err()
    );
}

#[test]
fn should_reject_a_snapshot_whose_container_tree_has_no_root() {
    let result = analyze(
        &snapshot(Vec::new(), Vec::new(), Vec::new()),
        &AnalyzeConfig::default(),
    );

    assert!(
        matches!(&result, Err(UnlaceError::SnapshotInvalid { .. })),
        "{:?}",
        result.err()
    );
}

#[test]
fn should_report_only_the_greenfield_profile_when_it_alone_is_selected() {
    let mut config = AnalyzeConfig::default();
    config.select_mode(crate::config::Mode::Greenfield);

    let result = analyze(&cyclic_pair_snapshot(), &config);

    let profiles = result.as_ref().ok().map(|result| &result.profiles);
    assert!(profiles.is_some_and(|profiles| profiles.anchored.is_none()));
    let greenfield = profiles.and_then(|profiles| profiles.greenfield.as_ref());
    assert!(
        greenfield.is_some_and(|profile| !profile.current.unique_findings.is_empty()),
        "the cycle is reported as the greenfield profile's own finding"
    );
    assert!(
        result
            .as_ref()
            .is_ok_and(|result| result.current.shared_findings.is_empty()),
        "nothing is shared when one profile runs"
    );
}
