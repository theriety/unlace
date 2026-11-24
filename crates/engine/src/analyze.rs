//! The pure analysis pass: snapshot plus config in, [`AnalyzeResult`] out.
//!
//! [`analyze`] performs no I/O and holds no global state, so an identical
//! snapshot, config, and seed always produce an identical result (AD-5). It runs
//! the full restructuring pipeline — SCC condensation, the real-directory folder
//! partition, objective-driven polish, upper-level acyclic clustering, and
//! scoring — to return up to `k` candidate layouts per requested mode, each
//! narrated against the current tree. Alongside the candidates it derives the
//! current tree's structural violations (cycles, polarity breaches, over-exports)
//! and scores the current layout.

use std::collections::BTreeMap;

use unlace_ir::Snapshot;

use crate::config::{AnalyzeConfig, ProfileName};
use crate::error::UnlaceError;
use crate::result::{AnalyzeResult, CurrentTree, Profiles, RESULT_SCHEMA_VERSION, TestSettings};

/// The capacity borderline band: a finding within ±10% of a cap is borderline
/// and never gates CI (reference `BORDERLINE_CAPACITY_MARGIN`).
pub const BORDERLINE_CAPACITY_MARGIN: f64 = 0.1;

mod advice;
mod findings;
mod layout;
mod real_paths;
mod relocation;
mod rendering;
mod scoring;
mod search;
pub(crate) mod test_policy;

use advice::build_advice;
use findings::{profile_findings, summarize};
use rendering::render_tree;
use search::build_profile_result;
use test_policy::TestPolicy;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

/// Analyzes `snapshot` under `config`, returning the owned [`AnalyzeResult`].
///
/// The pass is pure: it reads only the snapshot and config and returns owned
/// data. It derives the current tree's violations (cycles, polarity breaches,
/// over-exports), scores the current layout, then runs the restructuring pipeline
/// to produce one [`ProfileResult`](crate::result::ProfileResult) of up to `k`
/// diverse candidates per requested mode.
///
/// Move planning needs a single root: a snapshot whose container tree is a
/// forest with several roots is rendered, scored and checked for findings, but
/// offered no candidates, so it gets no move advice.
///
/// # Errors
///
/// Returns [`UnlaceError::SnapshotInvalid`] if the snapshot's container tree
/// cannot be rendered (it is otherwise pre-validated at assembly).
pub fn analyze(snapshot: &Snapshot, config: &AnalyzeConfig) -> Result<AnalyzeResult, UnlaceError> {
    // `[analysis].jobs` caps the search's parallelism through a scoped pool so
    // the library behaves exactly like the CLI (AD-5); results are
    // thread-count invariant (NFR-1), so `0` (use the global pool) and any
    // positive count all produce byte-identical output.
    let jobs = usize::try_from(config.analysis.jobs).unwrap_or(usize::MAX);
    let scoped = if jobs == 0 {
        None
    } else {
        rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build()
            .ok()
    };
    match scoped {
        Some(pool) => pool.install(|| analyze_inner(snapshot, config)),
        // `0` asks for the global pool; scoped-pool creation fails only on
        // resource exhaustion, and the global pool yields the same bytes, so
        // degrading is safe.
        None => analyze_inner(snapshot, config),
    }
}

/// The body of [`analyze`], run inside whatever rayon pool the caller scoped.
fn analyze_inner(
    snapshot: &Snapshot,
    config: &AnalyzeConfig,
) -> Result<AnalyzeResult, UnlaceError> {
    let ir = snapshot.ir();
    let tree = &ir.containers;

    let current_node = render_tree(
        tree,
        &ir.nodes,
        &|node| Some(node.container),
        &BTreeMap::new(),
    )?;
    let selected = &config.analysis.profiles;
    let tests = TestPolicy::new(&config.tests)?;
    let mut anchored_findings = Vec::new();
    let mut greenfield_findings = Vec::new();
    let mut anchored = None;
    let mut greenfield = None;

    if selected.contains(&ProfileName::Anchored) {
        let profile = &config.profiles.anchored;
        anchored_findings = profile_findings(snapshot, &current_node, profile, &tests);
        // fails only on a candidate-assembly invariant breach: the current tree
        // already rendered, and a candidate keeps its root and namespaces.
        // coverage: unreachable, a candidate tree keeps the rendered root and every pass-start namespace, so building one never fails
        let result = build_profile_result(snapshot, profile, &anchored_findings, &tests)?;
        anchored = Some(result);
    }
    if selected.contains(&ProfileName::Greenfield) {
        let profile = &config.profiles.greenfield;
        greenfield_findings = profile_findings(snapshot, &current_node, profile, &tests);
        // fails only on a candidate-assembly invariant breach: the current tree
        // already rendered, and a candidate keeps its root and namespaces.
        // coverage: unreachable, a candidate tree keeps the rendered root and every pass-start namespace, so building one never fails
        let result = build_profile_result(snapshot, profile, &greenfield_findings, &tests)?;
        greenfield = Some(result);
    }

    let shared_findings = if anchored.is_some() && greenfield.is_some() {
        anchored_findings
            .iter()
            .filter(|finding| greenfield_findings.contains(finding))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if let Some(result) = anchored.as_mut() {
        result.current.unique_findings = anchored_findings
            .into_iter()
            .filter(|finding| !shared_findings.contains(finding))
            .collect();
    }
    if let Some(result) = greenfield.as_mut() {
        result.current.unique_findings = greenfield_findings
            .into_iter()
            .filter(|finding| !shared_findings.contains(finding))
            .collect();
    }

    let profiles = Profiles {
        anchored,
        greenfield,
    };
    let advice = build_advice(snapshot, config, &profiles);
    let mut result = AnalyzeResult {
        schema_version: RESULT_SCHEMA_VERSION,
        snapshot_hash: snapshot.hash().to_hex().to_string(),
        summary: summarize(snapshot),
        tests: TestSettings::from(&config.tests),
        current: CurrentTree {
            tree: current_node,
            shared_findings,
        },
        profiles,
        advice,
    };
    // the analysis keys paths under the package-group label; the serialized
    // result carries real repo-relative paths only.
    real_paths::strip_group_prefix(&mut result);
    Ok(result)
}
