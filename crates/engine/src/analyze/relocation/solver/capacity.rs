//! Physical-capacity vetoes and the capacity remainder for polish evidence.

use std::collections::BTreeMap;

use unlace_core::cluster::{ClusterId, Partition};

use crate::analyze::findings::{
    RealRoots, physical_folder_entries_repository_relative, physical_folder_findings,
    walk_all_capacity,
};
use crate::analyze::relocation::mirror::PolishEvidence;
use crate::analyze::relocation::{CandidateTree, PipelineSolver};
use crate::config::CapacityConfig;
use crate::result::{CapacityRemainder, ContainerNode, Level, Severity};

impl PipelineSolver<'_> {
    pub(in crate::analyze::relocation) fn physical_entry_counts(
        &self,
        parts: &Partition,
    ) -> BTreeMap<Vec<String>, u32> {
        let assembled = self.assemble(parts);
        physical_folder_entries_repository_relative(
            &assembled.tree,
            Some(&assembled.namespace_by_file),
        )
    }

    pub(in crate::analyze::relocation) fn physical_entry_counts_with_polish_evidence(
        &self,
        parts: &Partition,
        evidence: &PolishEvidence,
    ) -> BTreeMap<Vec<String>, u32> {
        let assembled = self.assemble_with_polish_evidence(parts, evidence);
        physical_folder_entries_repository_relative(
            &assembled.tree,
            Some(&assembled.namespace_by_file),
        )
    }

    pub(in crate::analyze) fn permits_physical_capacity(
        &self,
        parts: &Partition,
        scc: u32,
        target: ClusterId,
    ) -> bool {
        if self.caps.folder == 0 {
            return true;
        }
        let mut assignment = parts.assignment().to_vec();
        let Some(slot) = assignment.get_mut(scc as usize) else {
            return false;
        };
        if *slot == target {
            return true;
        }
        let before = self.physical_entry_counts(parts);
        *slot = target;
        let moved = Partition::from_assignment(assignment, parts.cluster_count());
        let after = self.physical_entry_counts(&moved);
        before.keys().chain(after.keys()).all(|path| {
            let prior = before
                .get(path)
                .copied()
                .unwrap_or(0)
                .saturating_sub(self.caps.folder);
            let next = after
                .get(path)
                .copied()
                .unwrap_or(0)
                .saturating_sub(self.caps.folder);
            next <= prior
        })
    }

    pub(in crate::analyze::relocation) fn permits_mirror_physical_capacity(
        &self,
        parts: &Partition,
        scc: u32,
        target: ClusterId,
        evidence: &PolishEvidence,
        prospective: &PolishEvidence,
    ) -> bool {
        if self.caps.folder == 0 {
            return true;
        }
        let before = self.physical_entry_counts_with_polish_evidence(parts, evidence);
        let mut assignment = parts.assignment().to_vec();
        let Some(slot) = assignment.get_mut(scc as usize) else {
            return false;
        };
        *slot = target;
        let moved = Partition::from_assignment(assignment, parts.cluster_count());
        let after = self.physical_entry_counts_with_polish_evidence(&moved, prospective);
        before.keys().chain(after.keys()).all(|path| {
            let prior = before
                .get(path)
                .copied()
                .unwrap_or(0)
                .saturating_sub(self.caps.folder);
            let next = after
                .get(path)
                .copied()
                .unwrap_or(0)
                .saturating_sub(self.caps.folder);
            next <= prior
        })
    }

    /// Re-checks a candidate with physical folder semantics while retaining
    /// the DTO walk for file and upper-level findings.
    pub(in crate::analyze) fn capacity_remainder_with_polish_evidence(
        &self,
        parts: &Partition,
        evidence: &PolishEvidence,
        rendered: &ContainerNode,
        capacity: &CapacityConfig,
    ) -> CapacityRemainder {
        let assembled = self.assemble_with_polish_evidence(parts, evidence);
        Self::capacity_remainder_for_assembled(&assembled, rendered, capacity)
    }

    fn capacity_remainder_for_assembled(
        assembled: &CandidateTree,
        rendered: &ContainerNode,
        capacity: &CapacityConfig,
    ) -> CapacityRemainder {
        let folder_hard = physical_folder_findings(
            &physical_folder_entries_repository_relative(
                &assembled.tree,
                Some(&assembled.namespace_by_file),
            ),
            capacity.folder,
        )
        .into_iter()
        .filter(|finding| finding.severity == Severity::Violation)
        .count();
        let roots = RealRoots::from_tree_keyed(&assembled.tree, &assembled.key_by_id);
        let other: Vec<Level> = walk_all_capacity(rendered, capacity, &roots)
            .into_iter()
            .filter(|(level, finding)| {
                *level != Level::Folder && finding.severity == Severity::Violation
            })
            .map(|(level, _)| level)
            .collect();
        let remaining = folder_hard.saturating_add(other.len());
        let file_level = other.iter().filter(|&&level| level == Level::File).count();
        CapacityRemainder {
            remaining: u32::try_from(remaining).unwrap_or(u32::MAX),
            file_level: u32::try_from(file_level).unwrap_or(u32::MAX),
        }
    }
}

#[cfg(test)]
mod tests {
    use smol_str::SmolStr;
    use unlace_ir::{ContainerTree, ScopeLevel};

    use unlace_ir::{Polarity, Snapshot};

    use super::*;
    use crate::analyze::rendering::render_tree;
    use crate::analyze::test_policy::TestPolicy;
    use crate::analyze::test_support::{container, edge, node, snapshot};
    use crate::config::AnalyzeConfig;

    /// `a/a.ts` calls `b/b.ts`; each file sits alone in its own folder.
    fn two_folder_snapshot() -> Snapshot {
        snapshot(
            vec![
                node(0, "caller", 3, Polarity::Production),
                node(1, "callee", 4, Polarity::Production),
            ],
            vec![edge(0, 1)],
            vec![
                container(0, "app", ScopeLevel::PackageGroup, None),
                container(1, "a", ScopeLevel::Folder, Some(0)),
                container(2, "b", ScopeLevel::Folder, Some(0)),
                container(3, "a/a.ts", ScopeLevel::File, Some(1)),
                container(4, "b/b.ts", ScopeLevel::File, Some(2)),
            ],
        )
    }

    /// Whether the folder cap `folder` permits moving one file into the other's
    /// folder, and
    /// whether it permits a move, ordinary or mirrored, of an scc the partition
    /// does not hold.
    fn capacity_verdicts(folder: u32) -> (bool, bool, bool) {
        let snapshot = two_folder_snapshot();
        let tests = TestPolicy::defaults();
        let mut config = AnalyzeConfig::default();
        config.profiles.greenfield.capacity.folder = folder;
        let profile = &config.profiles.greenfield;
        let solver = PipelineSolver::new(
            &snapshot,
            profile,
            profile.objective.coefficients(),
            false,
            &tests,
        );
        let parts = solver.real_partition.clone();
        let evidence = PolishEvidence::default();
        let caller = parts.cluster_of(0).unwrap_or(ClusterId(u32::MAX));
        let callee = parts.cluster_of(1).unwrap_or(ClusterId(u32::MAX));
        assert_ne!(caller, callee, "each file starts in its own folder");
        (
            solver.permits_physical_capacity(&parts, 0, callee),
            solver.permits_physical_capacity(&parts, 9, callee),
            solver.permits_mirror_physical_capacity(&parts, 9, callee, &evidence, &evidence),
        )
    }

    #[test]
    fn should_veto_a_move_that_overfills_a_capped_folder() {
        assert_eq!(capacity_verdicts(1), (false, false, false));
    }

    #[test]
    fn should_veto_no_move_under_an_unbounded_folder_cap() {
        assert_eq!(capacity_verdicts(0), (true, true, true));
    }

    #[test]
    fn should_permit_an_scc_to_stay_in_its_own_full_folder() {
        let snapshot = two_folder_snapshot();
        let tests = TestPolicy::defaults();
        let mut config = AnalyzeConfig::default();
        config.profiles.greenfield.capacity.folder = 1;
        let profile = &config.profiles.greenfield;
        let solver = PipelineSolver::new(
            &snapshot,
            profile,
            profile.objective.coefficients(),
            false,
            &tests,
        );
        let parts = solver.real_partition.clone();
        let home = parts.cluster_of(0).unwrap_or(ClusterId(u32::MAX));

        assert!(solver.permits_physical_capacity(&parts, 0, home));
    }

    /// A candidate whose two sibling domains both echo their package `cts` is
    /// dropped by the renderer, yet its overrun is a hard capacity finding the
    /// remainder must count, as pricing and the findings do.
    #[test]
    fn should_count_a_domain_echoing_its_package_in_the_capacity_remainder() {
        let tree = ContainerTree::new(vec![
            container(0, "ws", ScopeLevel::PackageGroup, None),
            container(1, "cts", ScopeLevel::Package, Some(0)),
            container(2, "cts (cts.core)", ScopeLevel::Domain, Some(1)),
            container(3, "cts (cts.zz)", ScopeLevel::Domain, Some(1)),
            container(4, "cts/core/a", ScopeLevel::Folder, Some(2)),
            container(5, "cts/core/a/f.ts", ScopeLevel::File, Some(4)),
            container(6, "cts/io/b", ScopeLevel::Folder, Some(2)),
            container(7, "cts/io/b/f.ts", ScopeLevel::File, Some(6)),
            container(8, "cts/zz", ScopeLevel::Folder, Some(3)),
            container(9, "cts/zz/f.ts", ScopeLevel::File, Some(8)),
        ]);
        let key_by_id: BTreeMap<u32, SmolStr> =
            [(2, SmolStr::new("cts")), (3, SmolStr::new("cts"))]
                .into_iter()
                .collect();
        let rendered = render_tree(&tree, &[], &|_| None, &key_by_id).ok();
        let assembled = CandidateTree {
            tree,
            placement: BTreeMap::new(),
            pass_start_file_by_candidate: BTreeMap::new(),
            zone_by_file: BTreeMap::new(),
            namespace_by_file: BTreeMap::new(),
            package_by_file: BTreeMap::new(),
            key_by_id,
        };
        let capacity = CapacityConfig {
            file: 1000,
            folder: 1000,
            domain: 1,
            package: 1000,
            package_group: 1000,
        };

        let remainder = rendered.map(|rendered| {
            PipelineSolver::capacity_remainder_for_assembled(&assembled, &rendered, &capacity)
        });

        assert_eq!(
            remainder,
            Some(CapacityRemainder {
                remaining: 1,
                file_level: 0
            })
        );
    }
}
