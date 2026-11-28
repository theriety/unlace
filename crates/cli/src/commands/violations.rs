//! `unlace violations`: the CI gate over the current codebase.
//!
//! The command snapshots the root, derives the current tree's violations
//! (cycles, polarity breaches, over-exports, and capacity findings against the
//! configured caps) via the engine, and reports them. `--fail-on <classes>`
//! raises the exit decision to a gating match when a hard violation of a listed
//! class is present; capacity findings within ±10% of a cap are reported as
//! `borderline` and never gate. When some or all files state no imports, the
//! table ends with the cycle coverage note (ADR-35), so a gate log tells zero
//! cycles apart from cycles the analysis could not see; the exit decision does
//! not change unless the run opts in with `--fail-on cycle-coverage`, which
//! gates whenever the coverage is not complete.

use std::io::Write;
use std::path::PathBuf;

use unlace_engine::{
    CycleAnalysis, CycleCoverage, Severity, UnlaceError, Violation, ViolationKind, analyze,
    snapshot_from_root,
};

use crate::commands::{ConfigOverrides, findings, resolve_config};
use crate::render::{RenderOptions, write_json, write_violation_report};

/// The output format of the violations face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationFormat {
    /// The default table face.
    Table,
    /// The serialized `AnalyzeResult` JSON.
    Json,
}

/// One parsed `--fail-on` selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailOn {
    /// A violation class, optionally narrowed to a single severity.
    ///
    /// A bare `kind` selector (severity `None`) gates on any hard violation of
    /// that class. A `kind:severity` selector narrows the match to that
    /// severity. Because a borderline finding never gates CI (the ±10% capacity
    /// band), a `kind:borderline` selector matches nothing — it parses, but no
    /// finding can satisfy it.
    Finding {
        /// The violation class this selector gates on.
        kind: ViolationKind,
        /// The severity to narrow to; `None` accepts any gating severity.
        severity: Option<Severity>,
    },
    /// The cycle coverage (ADR-35): gates when some or all analysed files
    /// state no imports, so a run that cannot see every import cycle fails
    /// instead of passing silently. Opt-in; it takes no severity.
    CycleCoverage,
}

/// The parsed inputs of a `violations` run.
#[derive(Debug)]
pub struct ViolationsArgs {
    /// The repository root to analyze.
    pub root: PathBuf,
    /// The config file path (defaults apply when it is absent).
    pub config: PathBuf,
    /// The parallelism hint (`--jobs`).
    pub jobs: Option<u32>,
    /// The violation selectors that gate (`--fail-on`).
    pub fail_on: Vec<FailOn>,
    /// The output format.
    pub format: ViolationFormat,
    /// Shows every finding in the table instead of the first few; the gate
    /// always weighs all findings.
    pub full: bool,
}

/// The gating outcome of a `violations` run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// No listed class matched; exit `0`.
    Clean,
    /// A listed class matched a hard violation, or `cycle-coverage` is listed
    /// and the cycle coverage is not complete; exit `2`.
    Gated,
}

/// Parses a comma-separated `--fail-on` value into gating selectors.
///
/// Each token is a class name (`cycle`, `polarity`, `capacity`, `visibility`,
/// `testOnly`) optionally followed by `:severity` (`violation` or
/// `borderline`) to narrow the match, or `cycle-coverage` (also spelled
/// `cycleCoverage`), which takes no severity. A bare class gates on any hard
/// violation of that class. Blank tokens are skipped and repeated selectors
/// kept once, in first-seen order.
///
/// # Errors
///
/// Returns [`UnlaceError::ConfigInvalid`] when a class or severity name is
/// unknown, or when `cycle-coverage` carries a severity.
pub fn parse_fail_on(value: &str) -> Result<Vec<FailOn>, UnlaceError> {
    let mut selectors = Vec::new();
    for token in value.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let (class, severity_token) = match token.split_once(':') {
            Some((class, severity)) => (class, Some(severity)),
            None => (token, None),
        };
        let selector = if is_cycle_coverage(class) {
            if severity_token.is_some() {
                return Err(UnlaceError::ConfigInvalid {
                    key: Some("--fail-on".to_owned()),
                    reason: format!("`{class}` takes no severity; write `{class}` alone"),
                });
            }
            FailOn::CycleCoverage
        } else {
            FailOn::Finding {
                kind: parse_kind(class)?,
                severity: severity_token.map(parse_severity).transpose()?,
            }
        };
        if !selectors.contains(&selector) {
            selectors.push(selector);
        }
    }
    Ok(selectors)
}

/// Whether a `--fail-on` class token names the cycle-coverage gate.
fn is_cycle_coverage(class: &str) -> bool {
    matches!(class, "cycle-coverage" | "cycleCoverage")
}

/// Parses a `--fail-on` class token into a [`ViolationKind`].
///
/// # Errors
///
/// Returns [`UnlaceError::ConfigInvalid`] when the class name is unknown.
fn parse_kind(class: &str) -> Result<ViolationKind, UnlaceError> {
    match class {
        "cycle" => Ok(ViolationKind::Cycle),
        "polarity" => Ok(ViolationKind::Polarity),
        "capacity" => Ok(ViolationKind::Capacity),
        "visibility" => Ok(ViolationKind::Visibility),
        "testOnly" | "test-only" => Ok(ViolationKind::TestOnly),
        other => Err(UnlaceError::ConfigInvalid {
            key: Some("--fail-on".to_owned()),
            reason: format!(
                "unknown violation class `{other}`; expected cycle, polarity, capacity, visibility, testOnly, or cycle-coverage"
            ),
        }),
    }
}

/// Parses a `--fail-on` severity token into a [`Severity`].
///
/// # Errors
///
/// Returns [`UnlaceError::ConfigInvalid`] when the severity name is unknown.
fn parse_severity(severity: &str) -> Result<Severity, UnlaceError> {
    match severity {
        "violation" => Ok(Severity::Violation),
        "borderline" => Ok(Severity::Borderline),
        other => Err(UnlaceError::ConfigInvalid {
            key: Some("--fail-on".to_owned()),
            reason: format!("unknown severity `{other}`; expected violation or borderline"),
        }),
    }
}

/// Runs `violations`, writing the report to `out` and returning the gate outcome.
///
/// # Errors
///
/// Returns any [`UnlaceError`] from config loading, snapshotting, or analysis.
pub fn run(args: &ViolationsArgs, out: &mut impl Write) -> Result<Outcome, UnlaceError> {
    let overrides = ConfigOverrides {
        jobs: args.jobs,
        ..ConfigOverrides::default()
    };
    let config = resolve_config(&args.config, overrides)?;
    let snapshot = snapshot_from_root(&args.root, &config)?;
    let result = analyze(&snapshot, &config)?;
    let findings = findings(&result);

    match args.format {
        ViolationFormat::Table => {
            write_violation_report(
                &findings,
                &result.current.cycle_coverage,
                RenderOptions {
                    full: args.full,
                    ..RenderOptions::default()
                },
                out,
            )
            .map_err(|error| write_error(&error))?;
        }
        ViolationFormat::Json => write_json(&result, out).map_err(|error| write_error(&error))?,
    }

    Ok(gate(
        &findings,
        &result.current.cycle_coverage,
        &args.fail_on,
    ))
}

/// Decides the gate outcome: a hard (non-borderline) violation matching a listed
/// selector gates, and so does incomplete cycle coverage when `cycle-coverage`
/// is listed; borderline findings never do.
///
/// A finding gates when its class matches a selector and either the selector
/// names no severity or names the finding's severity — but only ever for a hard
/// [`Severity::Violation`], so a borderline finding can never gate regardless of
/// the selector.
fn gate(violations: &[Violation], coverage: &CycleCoverage, fail_on: &[FailOn]) -> Outcome {
    let gated = fail_on.iter().any(|selector| match *selector {
        FailOn::CycleCoverage => coverage.status != CycleAnalysis::Complete,
        FailOn::Finding { kind, severity } => violations
            .iter()
            .any(|violation| matches(kind, severity, violation)),
    });
    if gated {
        Outcome::Gated
    } else {
        Outcome::Clean
    }
}

/// Returns whether a hard violation satisfies a finding selector of `kind`
/// narrowed to `severity`.
///
/// A borderline finding never matches (it never gates CI); a hard violation
/// matches when its class equals `kind` and the selector either names no
/// severity or names [`Severity::Violation`].
fn matches(kind: ViolationKind, severity: Option<Severity>, violation: &Violation) -> bool {
    if violation.severity != Severity::Violation || kind != violation.kind {
        return false;
    }
    severity.is_none_or(|severity| severity == violation.severity)
}

/// Wraps a rendering I/O failure into a [`UnlaceError`].
fn write_error(error: &std::io::Error) -> UnlaceError {
    UnlaceError::InputUnreadable {
        path: PathBuf::from("<stdout>"),
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_parse_a_comma_separated_fail_on_list() {
        let selectors = parse_fail_on("cycle, capacity").unwrap_or_default();

        assert!(selectors.contains(&FailOn::Finding {
            kind: ViolationKind::Cycle,
            severity: None,
        }));
        assert!(selectors.contains(&FailOn::Finding {
            kind: ViolationKind::Capacity,
            severity: None,
        }));
    }

    #[test]
    fn should_parse_a_kind_with_a_severity_qualifier() {
        let selectors = parse_fail_on("capacity:violation").unwrap_or_default();

        assert_eq!(
            selectors,
            vec![FailOn::Finding {
                kind: ViolationKind::Capacity,
                severity: Some(Severity::Violation),
            }]
        );
    }

    #[test]
    fn should_parse_a_borderline_severity_qualifier() {
        let selectors = parse_fail_on("capacity:borderline").unwrap_or_default();

        assert_eq!(
            selectors,
            vec![FailOn::Finding {
                kind: ViolationKind::Capacity,
                severity: Some(Severity::Borderline),
            }]
        );
    }

    #[test]
    fn should_reject_an_unknown_fail_on_class() {
        assert!(matches!(
            parse_fail_on("bogus"),
            Err(UnlaceError::ConfigInvalid { .. })
        ));
    }

    #[test]
    fn should_reject_an_unknown_severity_qualifier() {
        assert!(matches!(
            parse_fail_on("capacity:bogus"),
            Err(UnlaceError::ConfigInvalid { .. })
        ));
    }

    #[test]
    fn should_gate_a_severity_qualified_selector_on_a_matching_hard_violation() {
        let hard = Violation {
            kind: ViolationKind::Capacity,
            severity: Severity::Violation,
            location: Vec::new(),
            detail: String::new(),
            import_cycle: None,
            break_suggestions: None,
            capacity: None,
        };
        let fail_on = parse_fail_on("capacity:violation").unwrap_or_default();

        assert_eq!(
            gate(&[hard], &CycleCoverage::default(), &fail_on),
            Outcome::Gated
        );
    }

    #[test]
    fn should_never_gate_a_borderline_qualified_selector() {
        // a borderline finding never gates, so `capacity:borderline` matches nothing.
        let borderline = Violation {
            kind: ViolationKind::Capacity,
            severity: Severity::Borderline,
            location: Vec::new(),
            detail: String::new(),
            import_cycle: None,
            break_suggestions: None,
            capacity: None,
        };
        let fail_on = parse_fail_on("capacity:borderline").unwrap_or_default();

        assert_eq!(
            gate(&[borderline], &CycleCoverage::default(), &fail_on),
            Outcome::Clean
        );
    }

    #[test]
    fn should_gate_only_on_a_hard_violation_of_a_listed_class() {
        let hard = Violation {
            kind: ViolationKind::Capacity,
            severity: Severity::Violation,
            location: Vec::new(),
            detail: String::new(),
            import_cycle: None,
            break_suggestions: None,
            capacity: None,
        };
        let fail_on = vec![FailOn::Finding {
            kind: ViolationKind::Capacity,
            severity: None,
        }];

        assert_eq!(
            gate(&[hard], &CycleCoverage::default(), &fail_on),
            Outcome::Gated
        );
    }

    #[test]
    fn should_not_gate_on_a_borderline_finding() {
        let borderline = Violation {
            kind: ViolationKind::Capacity,
            severity: Severity::Borderline,
            location: Vec::new(),
            detail: String::new(),
            import_cycle: None,
            break_suggestions: None,
            capacity: None,
        };
        let fail_on = vec![FailOn::Finding {
            kind: ViolationKind::Capacity,
            severity: None,
        }];

        assert_eq!(
            gate(&[borderline], &CycleCoverage::default(), &fail_on),
            Outcome::Clean
        );
    }

    #[test]
    fn should_not_gate_a_class_outside_the_fail_on_set() {
        let hard = Violation {
            kind: ViolationKind::Cycle,
            severity: Severity::Violation,
            location: Vec::new(),
            detail: String::new(),
            import_cycle: None,
            break_suggestions: None,
            capacity: None,
        };
        let fail_on = vec![FailOn::Finding {
            kind: ViolationKind::Capacity,
            severity: None,
        }];

        assert_eq!(
            gate(&[hard], &CycleCoverage::default(), &fail_on),
            Outcome::Clean
        );
    }

    /// The table `violations` prints for `findings` under `coverage`.
    fn table(findings: &[Violation], coverage: &CycleCoverage) -> String {
        let mut out = Vec::new();
        let written =
            write_violation_report(findings, coverage, RenderOptions::default(), &mut out);
        assert!(written.is_ok());
        String::from_utf8(out).unwrap_or_default()
    }

    #[test]
    fn should_note_partial_cycle_coverage_without_changing_the_gate() {
        let partial = CycleCoverage {
            status: CycleAnalysis::Partial,
            files_without_imports: 2,
        };
        let fail_on = parse_fail_on("cycle").unwrap_or_default();

        let text = table(&[], &partial);

        assert!(
            text.starts_with(
                "no violations\nCycle analysis is partial: 2 files state no imports, so a cycle \
                 through them is shown only where the symbol graph sees one.\n"
            ),
            "{text}"
        );
        assert_eq!(gate(&[], &partial, &fail_on), Outcome::Clean);
    }

    #[test]
    fn should_note_unavailable_cycle_coverage_after_the_findings() {
        let unavailable = CycleCoverage {
            status: CycleAnalysis::Unavailable,
            files_without_imports: 4,
        };
        let polarity = Violation {
            kind: ViolationKind::Polarity,
            severity: Severity::Violation,
            location: vec!["src/a.rs".to_owned()],
            detail: "test reaches production".to_owned(),
            import_cycle: None,
            break_suggestions: None,
            capacity: None,
        };

        let text = table(&[polarity], &unavailable);

        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.get(..2),
            Some(
                [
                    "polarity [violation] src/a.rs :: test reaches production",
                    "Cycle analysis is unavailable: no analysed file states its imports, so no \
                     cycle finding does not mean no cycle.",
                ]
                .as_slice()
            )
        );
    }

    #[test]
    fn should_gate_on_a_hard_violation_among_borderline_findings() {
        let cap = |severity| Violation {
            kind: ViolationKind::Capacity,
            severity,
            location: vec!["c".to_owned()],
            detail: "over".to_owned(),
            import_cycle: None,
            break_suggestions: None,
            capacity: None,
        };
        let mut findings = vec![cap(Severity::Borderline); 6];
        findings.push(Violation {
            kind: ViolationKind::Cycle,
            ..cap(Severity::Violation)
        });
        let fail_on = [FailOn::Finding {
            kind: ViolationKind::Cycle,
            severity: None,
        }];

        assert_eq!(
            gate(&findings, &CycleCoverage::default(), &fail_on),
            Outcome::Gated
        );
    }

    #[test]
    fn should_parse_the_polarity_and_visibility_classes() {
        let selectors = parse_fail_on("polarity,visibility").unwrap_or_default();

        assert_eq!(
            selectors,
            vec![
                FailOn::Finding {
                    kind: ViolationKind::Polarity,
                    severity: None,
                },
                FailOn::Finding {
                    kind: ViolationKind::Visibility,
                    severity: None,
                },
            ]
        );
    }

    #[test]
    fn should_parse_both_test_only_spellings_and_drop_duplicates_and_blanks() {
        let selectors = parse_fail_on("testOnly, ,test-only,cycle,cycle").unwrap_or_default();

        assert_eq!(
            selectors,
            vec![
                FailOn::Finding {
                    kind: ViolationKind::TestOnly,
                    severity: None,
                },
                FailOn::Finding {
                    kind: ViolationKind::Cycle,
                    severity: None,
                },
            ]
        );
    }

    /// A cycle coverage of `status` with `without` files stating no imports.
    fn coverage(status: CycleAnalysis, without: u32) -> CycleCoverage {
        CycleCoverage {
            status,
            files_without_imports: without,
        }
    }

    #[test]
    fn should_parse_both_cycle_coverage_spellings_as_one_selector_in_order() {
        let selectors =
            parse_fail_on("cycle,cycle-coverage, cycleCoverage,cycle").unwrap_or_default();

        assert_eq!(
            selectors,
            vec![
                FailOn::Finding {
                    kind: ViolationKind::Cycle,
                    severity: None,
                },
                FailOn::CycleCoverage,
            ]
        );
    }

    #[test]
    fn should_reject_a_severity_on_the_cycle_coverage_selector() {
        assert!(matches!(
            parse_fail_on("cycle-coverage:violation"),
            Err(UnlaceError::ConfigInvalid { ref reason, .. }) if reason.contains("takes no severity")
        ));
    }

    #[test]
    fn should_list_cycle_coverage_among_the_expected_classes_when_rejecting_junk() {
        let rejected = parse_fail_on("cycle,coverage");

        assert!(
            matches!(
                rejected,
                Err(UnlaceError::ConfigInvalid { ref reason, .. })
                    if reason == "unknown violation class `coverage`; expected cycle, polarity, \
                                  capacity, visibility, testOnly, or cycle-coverage"
            ),
            "{rejected:?}"
        );
    }

    #[test]
    fn should_gate_cycle_coverage_only_when_the_coverage_is_incomplete() {
        let fail_on = [FailOn::CycleCoverage];

        assert_eq!(
            gate(&[], &coverage(CycleAnalysis::Complete, 0), &fail_on),
            Outcome::Clean
        );
        assert_eq!(
            gate(&[], &coverage(CycleAnalysis::Partial, 1), &fail_on),
            Outcome::Gated
        );
        assert_eq!(
            gate(&[], &coverage(CycleAnalysis::Unavailable, 3), &fail_on),
            Outcome::Gated
        );
    }

    #[test]
    fn should_fail_naming_the_sink_when_either_face_cannot_be_written() {
        let root = std::env::temp_dir().join(format!(
            "unlace-violations-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        ));
        let _ = std::fs::create_dir_all(&root);
        let _ = std::fs::write(root.join("a.py"), "def alpha():\n    return 1\n");

        for format in [ViolationFormat::Table, ViolationFormat::Json] {
            let args = ViolationsArgs {
                root: root.clone(),
                config: root.join("unlace.toml"),
                jobs: None,
                fail_on: Vec::new(),
                format,
                full: false,
            };
            let mut closed: &mut [u8] = &mut [];

            let outcome = run(&args, &mut closed);

            assert!(
                matches!(
                    outcome,
                    Err(UnlaceError::InputUnreadable { ref path, .. })
                        if path.as_os_str() == "<stdout>"
                ),
                "{format:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
