//! Per-folder member views the candidate assembly reads.

use std::collections::{BTreeMap, BTreeSet};

use smol_str::SmolStr;
use unlace_core::cluster::Partition;
use unlace_core::cluster::seed::SeedLevel;
use unlace_core::graph::csr::Csr;
use unlace_ir::{Container, ContainerId, ContainerTree, ScopeLevel};

use crate::analyze::layout::{NameTally, cluster_level, home_affinity, plurality, vote};
use crate::analyze::relocation::mirror::PolishEvidence;
use crate::analyze::relocation::partition_keys::split_by_key;
use crate::analyze::relocation::{CandidateTree, FileInfo, PipelineSolver};

impl PipelineSolver<'_> {
    /// Groups the file-graph vertices by the folder cluster their SCC lands in,
    /// members sorted by file path for deterministic emission.
    pub(super) fn folder_members(&self, parts: &Partition) -> BTreeMap<u32, Vec<u32>> {
        let mut members_of: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        let clustered = (0..self.files.len())
            .zip(&self.condensation.membership)
            .filter_map(|(vertex, scc)| {
                let cluster = parts.cluster_of(scc.0);
                debug_assert!(cluster.is_some(), "the folder partition covers every scc");
                cluster.map(|cluster| (vertex, cluster))
            });
        for (vertex, cluster) in clustered {
            members_of
                .entry(cluster.0)
                .or_default()
                .push(u32::try_from(vertex).unwrap_or(u32::MAX));
        }
        for members in members_of.values_mut() {
            members.sort_by(|&left, &right| {
                let left_name = self
                    .files
                    .get(left as usize)
                    .map_or("", |file| file.name.as_str());
                let right_name = self
                    .files
                    .get(right as usize)
                    .map_or("", |file| file.name.as_str());
                left_name.cmp(right_name).then(left.cmp(&right))
            });
        }
        members_of
    }

    /// Each folder cluster's members drawn in the cluster's own folder.
    ///
    /// [`Self::projected_folder_members`] draws a cross-package cycle member
    /// whose retained home is another folder in that home, under its own
    /// package, so it must not key the cluster it is solved in. A cluster none
    /// of whose members stay keeps them all, so no cluster goes unkeyed.
    pub(super) fn drawn_members(
        &self,
        parts: &Partition,
        members_of: &BTreeMap<u32, Vec<u32>>,
        evidence: &PolishEvidence,
    ) -> BTreeMap<u32, Vec<u32>> {
        members_of
            .iter()
            .map(|(&folder, members)| {
                let key = self
                    .real_folder_names
                    .get(folder as usize)
                    .cloned()
                    .unwrap_or(SmolStr::new_static("workspace"));
                let drawn = self
                    .projected_folder_members(parts, members, evidence, &key)
                    .remove(&key)
                    .filter(|drawn| !drawn.is_empty())
                    .unwrap_or(members.clone());
                (folder, drawn)
            })
            .collect()
    }

    /// Dominant laminar home key (via `key`, weighted by production SLOC) per
    /// base vertex of an upper clustering level.
    ///
    /// `cluster_of` maps a folder cluster to the base vertex it contributes to at
    /// this level — the identity for the domain level (each folder is a vertex),
    /// the domain partition for the package level (each domain is a vertex). An
    /// absent (empty) vertex falls back to `workspace`. The result seeds and gains
    /// [`cluster_level`] so containers group by home directory.
    pub(super) fn home_keys(
        &self,
        members_of: &BTreeMap<u32, Vec<u32>>,
        vertex_count: usize,
        cluster_of: impl Fn(u32) -> u32,
        key: impl Fn(&FileInfo) -> &SmolStr,
    ) -> Vec<SmolStr> {
        let mut tally: NameTally = BTreeMap::new();
        for (&folder, members) in members_of {
            let cluster = cluster_of(folder);
            for file in members
                .iter()
                .filter_map(|&vertex| self.files.get(vertex as usize))
            {
                vote(&mut tally, cluster, key(file).clone(), file.production_sloc);
            }
        }
        (0..vertex_count)
            .map(|vertex| {
                tally
                    .get(&u32::try_from(vertex).unwrap_or(u32::MAX))
                    .map_or_else(|| SmolStr::new("workspace"), plurality)
            })
            .collect()
    }

    /// Clusters domains into packages with the wall lifted, keeping every
    /// domain that holds a folded file (ADR-21) apart from the others by its
    /// pin.
    pub(super) fn cluster_packages(
        &self,
        domain_quotient: &Csr,
        package_homes: &[SmolStr],
        domain_pins: Option<&[SmolStr]>,
    ) -> Partition {
        let clustered = cluster_level(
            domain_quotient,
            &self.caps,
            SeedLevel::Package,
            &home_affinity(package_homes),
        );
        match domain_pins {
            Some(domain_pins) => split_by_key(&clustered, domain_pins),
            None => clustered,
        }
    }

    /// The candidate of a repository with no files: its root package group
    /// alone.
    pub(super) fn empty_candidate(&self) -> CandidateTree {
        let root = Container {
            id: ContainerId(0),
            name: self.root_name.clone(),
            level: ScopeLevel::PackageGroup,
            parent: None,
            synthetic: false,
            label_keyed: false,
        };
        CandidateTree {
            tree: ContainerTree::new(vec![root]),
            placement: BTreeMap::new(),
            pass_start_file_by_candidate: BTreeMap::new(),
            zone_by_file: BTreeMap::new(),
            namespace_by_file: BTreeMap::new(),
            package_by_file: BTreeMap::new(),
            key_by_id: BTreeMap::new(),
        }
    }

    /// Keys each folder cluster by the pass-start package of a folded file it
    /// holds (ADR-21), or the empty key when it holds none; `None` when the
    /// restart folded nothing.
    pub(super) fn folded_folder_pins(
        &self,
        members_of: &BTreeMap<u32, Vec<u32>>,
        folder_count: usize,
        evidence: &PolishEvidence,
    ) -> Option<Vec<SmolStr>> {
        if evidence.folds.is_empty() {
            return None;
        }
        let folded: BTreeSet<u32> = evidence.folds.iter().map(|fold| fold.file).collect();
        let mut pins = vec![SmolStr::default(); folder_count];
        for (&folder, members) in members_of {
            let pinned = members
                .iter()
                .filter_map(|&vertex| self.files.get(vertex as usize))
                .find(|file| folded.contains(&file.container));
            if let (Some(file), Some(slot)) = (pinned, pins.get_mut(folder as usize)) {
                slot.clone_from(&file.home.package);
            }
        }
        Some(pins)
    }
}

#[cfg(test)]
mod tests {
    use smol_str::SmolStr;
    use unlace_core::graph::csr::Csr;
    use unlace_ir::{Polarity, ScopeLevel};

    use crate::analyze::relocation::PipelineSolver;
    use crate::analyze::test_policy::TestPolicy;
    use crate::analyze::test_support::{container, edge, node, snapshot};
    use crate::config::AnalyzeConfig;

    #[test]
    fn should_keep_domains_pinned_to_different_packages_apart() {
        let snapshot = snapshot(
            vec![
                node(0, "caller", 2, Polarity::Production),
                node(1, "callee", 3, Polarity::Production),
            ],
            vec![edge(0, 1)],
            vec![
                container(0, "app", ScopeLevel::PackageGroup, None),
                container(1, "src", ScopeLevel::Folder, Some(0)),
                container(2, "src/a.ts", ScopeLevel::File, Some(1)),
                container(3, "src/b.ts", ScopeLevel::File, Some(1)),
            ],
        );
        let tests = TestPolicy::defaults();
        let config = AnalyzeConfig::default();
        let profile = &config.profiles.greenfield;
        let solver = PipelineSolver::new(
            &snapshot,
            profile,
            profile.objective.coefficients(),
            false,
            &tests,
        );
        // two bonded domains of one home cluster into one package …
        let quotient = Csr::from_weighted_edges(2, &[(0, 1, 5.0)]);
        let homes = [SmolStr::new("pkg"), SmolStr::new("pkg")];
        let free = solver.cluster_packages(&quotient, &homes, None);
        // … unless each holds a file folded back into a different package.
        let pins = [SmolStr::new("left"), SmolStr::new("right")];
        let pinned = solver.cluster_packages(&quotient, &homes, Some(&pins));

        assert_eq!(free.cluster_of(0), free.cluster_of(1));
        assert_ne!(pinned.cluster_of(0), pinned.cluster_of(1));
    }
}
