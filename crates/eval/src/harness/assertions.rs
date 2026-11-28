//! Preconditions, target assertions, and non-gating observations.

use std::collections::BTreeSet;

use crate::metrics;
use crate::target::{AssertBlock, FaceMode, ReferenceSet};

use super::inputs::EvalInputs;
use super::{PairF1Report, Verdict};

mod advice;
mod churn;
mod cycles;
#[cfg(test)]
pub(in crate::harness) mod fixtures;
mod membership;
mod observations;
mod preconditions;
mod shape;

use churn::{evaluate_capacity_relief, evaluate_move_budget, evaluate_non_inversion};
use membership::{evaluate_path_set, evaluate_preserve_dir, evaluate_preserve_symbol_home};
use observations::reference_pairs;
use shape::{evaluate_name_alignment, evaluate_no_synthetic_bucket, evaluate_size_band};

pub(super) use advice::{drifted_advice_paths, evaluate_advice};
pub(super) use cycles::evaluate_cycle_import_cuts;
pub(super) use observations::observe_modes;
pub(super) use preconditions::{all_findings, evaluate_preconditions};

/// Evaluates every assertion in the block once per face it applies to, then the
/// report-only pair-F1 per evaluated face.
pub(super) fn evaluate_assertions(
    block: &AssertBlock,
    inputs: &EvalInputs<'_>,
    reference: Option<&ReferenceSet>,
) -> (Vec<Verdict>, PairF1Report) {
    let mut verdicts = Vec::new();
    for assertion in &block.preserve_dir {
        let faces = faces_for(assertion.mode, &block.modes).into_iter();
        let pin = faces.map(|face| evaluate_preserve_dir(assertion, face, inputs));
        verdicts.extend(apply_expect_fail(
            assertion.expect_fail.as_deref(),
            pin.collect(),
        ));
    }
    for assertion in &block.keep_together {
        let faces = faces_for(assertion.mode, &block.modes).into_iter();
        let pin = faces.map(|face| evaluate_path_set(assertion, face, inputs, true));
        verdicts.extend(apply_expect_fail(
            assertion.expect_fail.as_deref(),
            pin.collect(),
        ));
    }
    for assertion in &block.separate {
        let faces = faces_for(assertion.mode, &block.modes).into_iter();
        let pin = faces.map(|face| evaluate_path_set(assertion, face, inputs, false));
        verdicts.extend(apply_expect_fail(
            assertion.expect_fail.as_deref(),
            pin.collect(),
        ));
    }
    for band in &block.size_band {
        for face in faces_for(band.mode, &block.modes) {
            verdicts.push(evaluate_size_band(band, face, inputs));
        }
    }
    for budget in &block.move_budget {
        verdicts.push(evaluate_move_budget(budget, inputs));
    }
    for bucket in &block.no_synthetic_bucket {
        for face in faces_for(bucket.mode, &block.modes) {
            verdicts.push(evaluate_no_synthetic_bucket(bucket, face, inputs));
        }
    }
    for alignment in &block.name_alignment {
        for face in faces_for(alignment.mode, &block.modes) {
            verdicts.push(evaluate_name_alignment(alignment, face, inputs));
        }
    }
    for relief in &block.capacity_relief {
        verdicts.push(evaluate_capacity_relief(relief, inputs));
    }
    for inversion in &block.non_inversion {
        verdicts.push(evaluate_non_inversion(inversion, inputs));
    }
    for home in &block.preserve_symbol_home {
        let faces = faces_for(home.mode, &block.modes).into_iter();
        let pin = faces.map(|face| evaluate_preserve_symbol_home(home, face, inputs));
        verdicts.extend(apply_expect_fail(
            home.expect_fail.as_deref(),
            pin.collect(),
        ));
    }

    let mut pair_f1: PairF1Report = Vec::new();
    if let Some(reference) = reference {
        let universe: BTreeSet<String> = reference
            .container
            .iter()
            .flat_map(|container| container.files.iter().cloned())
            .collect();
        let reference_pairs = reference_pairs(reference);
        for (face, face_inputs) in &inputs.faces {
            let predicted = metrics::co_membership_pairs(face_inputs.tree, &universe);
            let (precision, recall, f1) = metrics::pair_f1(&reference_pairs, &predicted);
            pair_f1.push((*face, precision, recall, f1));
        }
    }
    (verdicts, pair_f1)
}

/// Applies one pin's temporary `expect_fail` marker across every verdict the
/// pin produced (all its faces at once).
///
/// Without a marker the verdicts pass through unchanged. With one, a pin that
/// fails on any face is the expected witness: every verdict passes and its
/// detail names the reason before the original observation. A marked pin that
/// passes on every face reports one failure instead, so a stale marker is
/// removed rather than silently hiding a fixed pin. The mechanism exists only
/// while the false-move witnesses are red; the finished eval sources drop it.
fn apply_expect_fail(reason: Option<&str>, verdicts: Vec<Verdict>) -> Vec<Verdict> {
    let Some(reason) = reason else {
        return verdicts;
    };
    if verdicts.iter().all(|verdict| verdict.passed) {
        let label = verdicts
            .first()
            .map_or_else(|| "expect_fail".to_owned(), |verdict| verdict.label.clone());
        return vec![Verdict {
            label,
            passed: false,
            detail: format!("unexpectedly passed; remove expect_fail ({reason})"),
        }];
    }
    verdicts
        .into_iter()
        .map(|verdict| Verdict {
            label: verdict.label,
            passed: true,
            detail: format!("expected-fail ({reason}): {}", verdict.detail),
        })
        .collect()
}

/// The faces an assertion applies to: its own override, else the block default.
fn faces_for(own: Option<FaceMode>, block_modes: &[FaceMode]) -> Vec<FaceMode> {
    match own {
        Some(face) => vec![face],
        None => block_modes.to_vec(),
    }
}

#[cfg(test)]
mod expect_fail_tests {
    use super::apply_expect_fail;
    use crate::harness::Verdict;

    const REASON: &str = "F1: shape.ts leaves its companion folder";

    fn verdict(label: &str, passed: bool) -> Verdict {
        Verdict {
            label: label.to_owned(),
            passed,
            detail: format!("observed {label}"),
        }
    }

    /// The `(label, passed, detail)` of each verdict, for whole-value asserts.
    fn shape(verdicts: &[Verdict]) -> Vec<(&str, bool, &str)> {
        verdicts
            .iter()
            .map(|verdict| {
                (
                    verdict.label.as_str(),
                    verdict.passed,
                    verdict.detail.as_str(),
                )
            })
            .collect()
    }

    #[test]
    fn should_leave_verdicts_unchanged_without_a_marker() {
        let verdicts = vec![
            verdict("pin#anchored", true),
            verdict("pin#greenfield", false),
        ];

        let applied = apply_expect_fail(None, verdicts);

        assert_eq!(
            shape(&applied),
            vec![
                ("pin#anchored", true, "observed pin#anchored"),
                ("pin#greenfield", false, "observed pin#greenfield"),
            ]
        );
    }

    #[test]
    fn should_pass_every_face_of_a_marked_pin_that_fails_on_any_face() {
        let verdicts = vec![
            verdict("pin#anchored", true),
            verdict("pin#greenfield", false),
        ];

        let applied = apply_expect_fail(Some(REASON), verdicts);

        assert_eq!(
            shape(&applied),
            vec![
                (
                    "pin#anchored",
                    true,
                    "expected-fail (F1: shape.ts leaves its companion folder): observed pin#anchored"
                ),
                (
                    "pin#greenfield",
                    true,
                    "expected-fail (F1: shape.ts leaves its companion folder): observed pin#greenfield"
                ),
            ]
        );
    }

    #[test]
    fn should_fail_a_marked_pin_that_passes_on_every_face() {
        let verdicts = vec![
            verdict("pin#anchored", true),
            verdict("pin#greenfield", true),
        ];

        let applied = apply_expect_fail(Some(REASON), verdicts);

        assert_eq!(
            shape(&applied),
            vec![(
                "pin#anchored",
                false,
                "unexpectedly passed; remove expect_fail (F1: shape.ts leaves its companion folder)"
            )]
        );
    }

    #[test]
    fn should_fall_back_to_a_generic_label_for_a_marked_pin_with_no_verdicts() {
        let applied = apply_expect_fail(Some(REASON), Vec::new());

        assert_eq!(
            shape(&applied),
            vec![(
                "expect_fail",
                false,
                "unexpectedly passed; remove expect_fail (F1: shape.ts leaves its companion folder)"
            )]
        );
    }
}
