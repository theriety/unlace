//! Pure arithmetic of the evidence model: dependency weights, token overlap, and
//! the weighted and structural evidence scores.

use std::collections::BTreeSet;

use unlace_ir::EdgeKind;

use crate::config::ProfileConfig;
use crate::result::EvidenceSignals;

pub(super) fn dependency_weight(kind: EdgeKind, config: &ProfileConfig) -> f64 {
    match kind {
        EdgeKind::ValueImport => config.weights.value_import,
        EdgeKind::TypeReference => config.weights.type_reference,
        EdgeKind::Inheritance => config.weights.inheritance,
        EdgeKind::Call => config.weights.call,
        EdgeKind::ReExport => config.weights.re_export,
    }
}
pub(super) fn jaccard_tokens(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    let union = left.union(right).count();
    if union == 0 {
        0.0
    } else {
        usize_ratio(left.intersection(right).count(), union)
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn usize_as_f64(value: usize) -> f64 {
    value as f64
}

pub(super) fn usize_ratio(numerator: usize, denominator: usize) -> f64 {
    usize_as_f64(numerator) / usize_as_f64(denominator)
}
pub(super) fn weighted_evidence(e: EvidenceSignals, config: &ProfileConfig) -> f64 {
    let w = config.qualification.weights;
    let total = w.unique_owner
        + w.role_affinity
        + w.destination_advantage
        + w.destination_cohesion
        + w.producer_evidence
        + w.architectural_reach;
    (e.unique_owner * w.unique_owner
        + e.role_affinity * w.role_affinity
        + e.destination_advantage * w.destination_advantage
        + e.destination_cohesion * w.destination_cohesion
        + e.producer_evidence * w.producer_evidence
        + e.architectural_reach * w.architectural_reach)
        / total
}
pub(super) fn structural_evidence(e: EvidenceSignals, config: &ProfileConfig) -> f64 {
    let w = config.qualification.weights;
    let total = w.unique_owner
        + w.destination_advantage
        + w.destination_cohesion
        + w.producer_evidence
        + w.architectural_reach;
    if total == 0.0 {
        0.0
    } else {
        (e.unique_owner * w.unique_owner
            + e.destination_advantage * w.destination_advantage
            + e.destination_cohesion * w.destination_cohesion
            + e.producer_evidence * w.producer_evidence
            + e.architectural_reach * w.architectural_reach)
            / total
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use unlace_ir::EdgeKind;

    use super::{dependency_weight, jaccard_tokens};
    use crate::config::ProfileConfig;

    #[test]
    fn should_weigh_each_dependency_kind_by_its_own_configured_weight() {
        let mut config = ProfileConfig::default();
        config.weights.value_import = 2.0;
        config.weights.type_reference = 3.0;
        config.weights.inheritance = 4.0;
        config.weights.call = 5.0;
        config.weights.re_export = 6.0;

        let weights = [
            EdgeKind::ValueImport,
            EdgeKind::TypeReference,
            EdgeKind::Inheritance,
            EdgeKind::Call,
            EdgeKind::ReExport,
        ]
        .map(|kind| dependency_weight(kind, &config).to_bits());

        assert_eq!(weights, [2.0, 3.0, 4.0, 5.0, 6.0].map(f64::to_bits));
    }

    #[test]
    fn should_measure_token_overlap_as_jaccard_and_zero_for_two_empty_sets() {
        let tokens = |words: &[&str]| {
            words
                .iter()
                .map(|word| (*word).to_owned())
                .collect::<BTreeSet<_>>()
        };

        assert!(jaccard_tokens(&tokens(&[]), &tokens(&[])).abs() < f64::EPSILON);
        assert!(
            (jaccard_tokens(&tokens(&["record", "state"]), &tokens(&["state", "store"]))
                - 1.0 / 3.0)
                .abs()
                < f64::EPSILON
        );
    }
}
