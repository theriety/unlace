//! Qualification and relocation checks, including mirror-template ambiguity detection.

use crate::error::UnlaceError;

use super::ceilings::{non_negative_finite, unit_interval};
use crate::config::mirror::{mirror_templates_overlap, validate_mirror_template};
use crate::config::{AnalyzeConfig, MirrorTemplate, QualificationConfig, RelocationConfig};

impl AnalyzeConfig {
    pub(super) fn validate_qualification(
        prefix: &str,
        qualification: &QualificationConfig,
    ) -> Result<(), UnlaceError> {
        let key = |suffix: &str| format!("{prefix}.{suffix}");
        for (suffix, value) in [
            (
                "qualification.minimum-evidence",
                qualification.minimum_evidence,
            ),
            (
                "qualification.minimum-structural",
                qualification.minimum_structural,
            ),
            (
                "qualification.minimum-ambiguity-margin",
                qualification.minimum_ambiguity_margin,
            ),
        ] {
            unit_interval(&key(suffix), value)?;
        }
        let qualification_weights = qualification.weights;
        for (suffix, value) in [
            ("unique-owner", qualification_weights.unique_owner),
            ("role-affinity", qualification_weights.role_affinity),
            (
                "destination-advantage",
                qualification_weights.destination_advantage,
            ),
            (
                "destination-cohesion",
                qualification_weights.destination_cohesion,
            ),
            ("producer-evidence", qualification_weights.producer_evidence),
            (
                "architectural-reach",
                qualification_weights.architectural_reach,
            ),
        ] {
            non_negative_finite(&key(&format!("qualification.weights.{suffix}")), value)?;
        }
        let qualification_total = qualification_weights.unique_owner
            + qualification_weights.role_affinity
            + qualification_weights.destination_advantage
            + qualification_weights.destination_cohesion
            + qualification_weights.producer_evidence
            + qualification_weights.architectural_reach;
        non_negative_finite(&key("qualification.weights"), qualification_total)?;
        if qualification_total <= 0.0 {
            return Err(UnlaceError::ConfigInvalid {
                key: Some(key("qualification.weights")),
                reason: "at least one qualification weight must be positive".to_owned(),
            });
        }
        Ok(())
    }

    pub(super) fn validate_relocation(
        prefix: &str,
        relocation: &RelocationConfig,
    ) -> Result<(), UnlaceError> {
        for (field, patterns) in [
            ("forbid-file-moves", &relocation.forbid_file_moves),
            ("forbid-symbol-moves", &relocation.forbid_symbol_moves),
        ] {
            for (index, pattern) in patterns.iter().enumerate() {
                let key = format!("{prefix}.relocation.{field}[{index}]");
                if pattern.is_empty() {
                    return Err(UnlaceError::ConfigInvalid {
                        key: Some(key),
                        reason: "a pattern must not be empty".to_owned(),
                    });
                }
                glob::Pattern::new(pattern).map_err(|error| UnlaceError::ConfigInvalid {
                    key: Some(key),
                    reason: error.to_string(),
                })?;
            }
        }

        non_negative_finite(
            &format!("{prefix}.relocation.symbol-gain-threshold"),
            relocation.symbol_gain_threshold,
        )?;

        let mirror_prefix = format!("{prefix}.relocation.test-mirroring.rules");
        let mut parsed_sources: Vec<MirrorTemplate> = Vec::new();
        for (rule_index, rule) in relocation.test_mirroring.rules.iter().enumerate() {
            let source_key = format!("{mirror_prefix}[{rule_index}].source");
            let source = validate_mirror_template(&rule.source, &source_key)?;
            // Sources are pairwise disjoint past this check, so a test template
            // can only be ambiguous against the tests of its own rule.
            if parsed_sources
                .iter()
                .any(|existing| mirror_templates_overlap(existing, &source))
            {
                return Err(UnlaceError::ConfigInvalid {
                    key: Some(source_key),
                    reason: "overlapping source template is ambiguous".to_owned(),
                });
            }
            let mut parsed_tests = Vec::new();
            for (test_index, test) in rule.tests.iter().enumerate() {
                let test_key = format!("{mirror_prefix}[{rule_index}].tests[{test_index}]");
                let template = validate_mirror_template(test, &test_key)?;
                if parsed_tests
                    .iter()
                    .any(|existing| mirror_templates_overlap(existing, &template))
                {
                    return Err(UnlaceError::ConfigInvalid {
                        key: Some(test_key),
                        reason: "overlapping test template is ambiguous".to_owned(),
                    });
                }
                parsed_tests.push(template);
            }
            parsed_sources.push(source);
        }
        Ok(())
    }
}
