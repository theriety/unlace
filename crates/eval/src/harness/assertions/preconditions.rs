//! Precondition evaluation: filters violations by class, severity, and location.

use unlace_engine::result::{
    AnalyzeResult, Severity as EngineSeverity, Violation, ViolationKind as EngineViolationKind,
};

use crate::harness::Verdict;
use crate::target::{Precondition, PreconditionKind, SeverityFilter, ViolationClass};

/// Returns the deduplicated findings visible across the executed profiles.
pub(in crate::harness) fn all_findings(result: &AnalyzeResult) -> Vec<Violation> {
    let mut findings = result.current.shared_findings.clone();
    for profile in [
        result.profiles.anchored.as_ref(),
        result.profiles.greenfield.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        findings.extend(profile.current.unique_findings.iter().cloned());
    }
    let mut unique = Vec::new();
    for finding in findings {
        let fresh = !unique.contains(&finding);
        unique.extend(fresh.then_some(finding));
    }
    unique
}

/// Preconditions verified against the current layout before scoring.
pub(in crate::harness) fn evaluate_preconditions(
    preconditions: &[Precondition],
    files: u32,
    violations: &[Violation],
) -> Vec<Verdict> {
    preconditions
        .iter()
        .map(|precondition| {
            let label = precondition_label(precondition);
            let (passed, observed) = match precondition.kind {
                PreconditionKind::FileCount => {
                    let min = precondition.min.unwrap_or(0);
                    let max = precondition.max.unwrap_or(u32::MAX);
                    (
                        files >= min && files <= max,
                        format!("summary.files = {files} against [{min}, {max}]"),
                    )
                }
                PreconditionKind::ViolationPresent | PreconditionKind::ViolationAbsent => {
                    let matches = matching_violations(precondition, violations);
                    let present = !matches.is_empty();
                    let expect_present = precondition.kind == PreconditionKind::ViolationPresent;
                    let classes: Vec<String> = matches
                        .iter()
                        .map(|violation| violation_class_name(violation.kind))
                        .collect();
                    let observed = if present {
                        format!("found [{}]", classes.join(", "))
                    } else {
                        "found none".to_owned()
                    };
                    (present == expect_present, observed)
                }
            };
            Verdict {
                label,
                passed,
                detail: format!("{observed}; because {}", precondition.because),
            }
        })
        .collect()
}

/// A precondition's stable label.
fn precondition_label(precondition: &Precondition) -> String {
    let filters = class_filters(precondition);
    match precondition.kind {
        PreconditionKind::FileCount => "file_count".to_owned(),
        PreconditionKind::ViolationPresent => {
            format!("violation_present({filters})")
        }
        PreconditionKind::ViolationAbsent => {
            format!("violation_absent({filters})")
        }
    }
}

/// The violation class plus optional suffix/severity a precondition filters on.
fn class_filters(precondition: &Precondition) -> String {
    let class = match precondition.violation {
        Some(ViolationClass::Cycle) => violation_class_name(EngineViolationKind::Cycle),
        Some(ViolationClass::Polarity) => violation_class_name(EngineViolationKind::Polarity),
        Some(ViolationClass::Capacity) => violation_class_name(EngineViolationKind::Capacity),
        Some(ViolationClass::Visibility) => violation_class_name(EngineViolationKind::Visibility),
        Some(ViolationClass::TestOnly) => violation_class_name(EngineViolationKind::TestOnly),
        None => "any".to_owned(),
    };
    let suffix = precondition
        .location_suffix
        .as_ref()
        .map(|suffix| format!("@{suffix}"))
        .unwrap_or_default();
    let severity = precondition
        .severity
        .map(|severity| match severity {
            SeverityFilter::Violation => "::violation",
            SeverityFilter::Borderline => "::borderline",
        })
        .unwrap_or_default();
    format!("{class}{suffix}{severity}")
}

/// The DTO violation kind rendered for messages.
fn violation_class_name(kind: EngineViolationKind) -> String {
    match kind {
        EngineViolationKind::Cycle => "cycle".to_owned(),
        EngineViolationKind::Polarity => "polarity".to_owned(),
        EngineViolationKind::Capacity => "capacity".to_owned(),
        EngineViolationKind::Visibility => "visibility".to_owned(),
        EngineViolationKind::TestOnly => "testOnly".to_owned(),
    }
}

/// Class equality between the target's vocabulary and the DTO's.
fn class_matches(precondition: &Precondition, violation: &Violation) -> bool {
    let Some(wanted) = precondition.violation else {
        return true;
    };
    matches!(
        (wanted, violation.kind),
        (ViolationClass::Cycle, EngineViolationKind::Cycle)
            | (ViolationClass::Polarity, EngineViolationKind::Polarity)
            | (ViolationClass::Capacity, EngineViolationKind::Capacity)
            | (ViolationClass::Visibility, EngineViolationKind::Visibility)
            | (ViolationClass::TestOnly, EngineViolationKind::TestOnly)
    )
}

/// Severity filter; absent means any severity qualifies.
fn severity_matches(precondition: &Precondition, violation: &Violation) -> bool {
    match precondition.severity {
        None => true,
        Some(SeverityFilter::Violation) => violation.severity == EngineSeverity::Violation,
        Some(SeverityFilter::Borderline) => violation.severity == EngineSeverity::Borderline,
    }
}

/// The violations matching a precondition's class, severity, and dot-segment
/// location filters.
fn matching_violations<'a>(
    precondition: &Precondition,
    violations: &'a [Violation],
) -> Vec<&'a Violation> {
    violations
        .iter()
        .filter(|violation| {
            class_matches(precondition, violation)
                && severity_matches(precondition, violation)
                && precondition
                    .location_suffix
                    .as_deref()
                    .is_none_or(|suffix| {
                        violation
                            .location
                            .iter()
                            .any(|location| location_suffix_matches(location, suffix))
                    })
        })
        .collect()
}

/// Dot-segment location matching: the location's last dot-segment equals the
/// suffix, or the location ends with `.<suffix>`.
fn location_suffix_matches(location: &str, suffix: &str) -> bool {
    location.ends_with(&format!(".{suffix}"))
        || location.rsplit('.').next().unwrap_or(location) == suffix
}

#[cfg(test)]
mod tests {
    use unlace_engine::result::{
        Severity as EngineSeverity, Violation, ViolationKind as EngineViolationKind,
    };

    use crate::target::{Precondition, PreconditionKind, SeverityFilter, ViolationClass};

    use super::{evaluate_preconditions, location_suffix_matches, matching_violations};

    fn finding(kind: EngineViolationKind, severity: EngineSeverity) -> Violation {
        Violation {
            kind,
            severity,
            location: vec!["app.hub".to_owned()],
            detail: "observed".to_owned(),
            break_suggestions: None,
            capacity: None,
            import_cycle: None,
        }
    }

    fn violation_check(
        kind: PreconditionKind,
        violation: Option<ViolationClass>,
        severity: Option<SeverityFilter>,
    ) -> Precondition {
        Precondition {
            kind,
            min: None,
            max: None,
            violation,
            location_suffix: Some("hub".to_owned()),
            severity,
            because: "the fixture is built that way".to_owned(),
        }
    }

    #[test]
    fn violation_suffix_matches_on_dot_segment_boundaries() {
        assert!(location_suffix_matches("a.hub", "hub"));
        assert!(location_suffix_matches("hub", "hub"));
        assert!(!location_suffix_matches("hub.sub", "hu"));
        let violations = vec![
            Violation {
                kind: EngineViolationKind::Capacity,
                severity: EngineSeverity::Borderline,
                location: vec!["src".to_owned(), "hub".to_owned()],
                detail: "over cap".to_owned(),
                import_cycle: None,
                break_suggestions: None,
                capacity: None,
            },
            Violation {
                kind: EngineViolationKind::Capacity,
                severity: EngineSeverity::Borderline,
                location: vec!["ingest.hub".to_owned()],
                detail: "dotted container".to_owned(),
                import_cycle: None,
                break_suggestions: None,
                capacity: None,
            },
        ];
        let precondition = Precondition {
            kind: PreconditionKind::ViolationAbsent,
            min: None,
            max: None,
            violation: Some(ViolationClass::Capacity),
            location_suffix: Some("hub".to_owned()),
            severity: Some(SeverityFilter::Borderline),
            because: "no borderline hub breach".to_owned(),
        };
        let matches = matching_violations(&precondition, &violations);
        assert_eq!(
            matches.len(),
            2,
            "dot-segment suffix 'hub' matches the bare segment and the dotted tail"
        );
        let precondition_narrower = Precondition {
            kind: PreconditionKind::ViolationPresent,
            severity: Some(SeverityFilter::Violation),
            ..precondition
        };
        assert_eq!(
            matching_violations(&precondition_narrower, &violations),
            Vec::<&Violation>::new()
        );
    }

    #[test]
    fn violation_present_names_and_matches_each_class_exactly() {
        let findings: Vec<Violation> = [
            EngineViolationKind::Cycle,
            EngineViolationKind::Polarity,
            EngineViolationKind::Capacity,
            EngineViolationKind::Visibility,
            EngineViolationKind::TestOnly,
        ]
        .into_iter()
        .map(|kind| finding(kind, EngineSeverity::Violation))
        .collect();
        for (class, name) in [
            (ViolationClass::Cycle, "cycle"),
            (ViolationClass::Polarity, "polarity"),
            (ViolationClass::Capacity, "capacity"),
            (ViolationClass::Visibility, "visibility"),
            (ViolationClass::TestOnly, "testOnly"),
        ] {
            let verdicts = evaluate_preconditions(
                &[violation_check(
                    PreconditionKind::ViolationPresent,
                    Some(class),
                    None,
                )],
                0,
                &findings,
            );
            let verdict = verdicts.first();
            assert_eq!(
                verdict.map(|verdict| verdict.label.clone()),
                Some(format!("violation_present({name}@hub)"))
            );
            assert_eq!(
                verdict.map(|verdict| verdict.detail.clone()),
                Some(format!(
                    "found [{name}]; because the fixture is built that way"
                )),
                "exactly the {name} finding matches"
            );
            assert!(verdict.is_some_and(|verdict| verdict.passed));
        }
    }

    #[test]
    fn violation_absent_fails_on_a_matching_finding_and_holds_on_none() {
        let breach = [finding(
            EngineViolationKind::Capacity,
            EngineSeverity::Violation,
        )];
        let hard = violation_check(
            PreconditionKind::ViolationAbsent,
            Some(ViolationClass::Capacity),
            Some(SeverityFilter::Violation),
        );
        let failed = evaluate_preconditions(std::slice::from_ref(&hard), 0, &breach);
        let failed_verdict = failed.first();
        assert_eq!(
            failed_verdict.map(|verdict| verdict.label.as_str()),
            Some("violation_absent(capacity@hub::violation)")
        );
        assert!(failed_verdict.is_some_and(|verdict| !verdict.passed));

        let soft = violation_check(
            PreconditionKind::ViolationAbsent,
            Some(ViolationClass::Capacity),
            Some(SeverityFilter::Borderline),
        );
        let held = evaluate_preconditions(&[soft], 0, &breach);
        let held_verdict = held.first();
        assert_eq!(
            held_verdict.map(|verdict| verdict.label.as_str()),
            Some("violation_absent(capacity@hub::borderline)")
        );
        assert_eq!(
            held_verdict.map(|verdict| verdict.detail.as_str()),
            Some("found none; because the fixture is built that way"),
            "a hard finding never satisfies a borderline filter"
        );
        assert!(held_verdict.is_some_and(|verdict| verdict.passed));
    }

    #[test]
    fn a_classless_violation_precondition_matches_any_class() {
        let verdicts = evaluate_preconditions(
            &[violation_check(
                PreconditionKind::ViolationPresent,
                None,
                None,
            )],
            0,
            &[finding(
                EngineViolationKind::Polarity,
                EngineSeverity::Borderline,
            )],
        );
        let verdict = verdicts.first();
        assert_eq!(
            verdict.map(|verdict| verdict.label.as_str()),
            Some("violation_present(any@hub)")
        );
        assert!(verdict.is_some_and(|verdict| verdict.passed));
    }
}
