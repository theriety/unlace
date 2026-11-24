//! Projects physical folder members and restores retained cycle homes.

use std::collections::BTreeMap;

use smol_str::SmolStr;
use unlace_core::cluster::Partition;
use unlace_ir::ContainerId;

use crate::analyze::layout::ContainerArena;
use crate::analyze::relocation::mirror::PolishEvidence;
use crate::analyze::relocation::{FileInfo, PipelineSolver};
use crate::analyze::scoring::ContainerSpec;

impl PipelineSolver<'_> {
    /// Groups physical members independently of their atomic solver representative.
    pub(super) fn projected_folder_members(
        &self,
        parts: &Partition,
        members: &[u32],
        polish_evidence: &PolishEvidence,
        fallback: &SmolStr,
    ) -> BTreeMap<SmolStr, Vec<u32>> {
        let mut groups: BTreeMap<SmolStr, Vec<u32>> = BTreeMap::new();
        for &vertex in members {
            let key = self
                .projected_folder_for_vertex(parts, vertex, polish_evidence)
                .map_or(fallback.clone(), |projected| {
                    projected.repository_relative(self.root_name.as_str())
                });
            groups.entry(key).or_default().push(vertex);
        }
        groups
    }

    /// Reuses the original ancestor chain for a physical cycle home with no
    /// representative cluster. Exact existing containers are reused so the
    /// restored files share their package and domain with ordinary residents.
    pub(super) fn restore_cycle_folder(
        &self,
        file: &FileInfo,
        arena: &mut ContainerArena,
        restored_ids: &mut BTreeMap<ContainerId, ContainerId>,
    ) -> Option<ContainerId> {
        let by_id: BTreeMap<_, _> = self
            .snapshot
            .ir()
            .containers
            .containers()
            .iter()
            .map(|container| (container.id, container))
            .collect();
        // `Snapshot::assemble` validates the tree, so every parent link
        // resolves and the walk ends only at the root.
        let ancestors: Vec<_> = std::iter::successors(
            by_id.get(&ContainerId(file.container)).copied(),
            |container| container.parent.and_then(|id| by_id.get(&id).copied()),
        )
        .skip(1)
        .collect();
        let mut parent = None;
        for container in ancestors.into_iter().rev() {
            let id = restored_ids
                .get(&container.id)
                .copied()
                .or_else(|| {
                    arena
                        .containers
                        .iter()
                        .find(|existing| {
                            existing.parent == parent
                                && existing.level == container.level
                                && existing.name == container.name
                                && existing.synthetic == container.synthetic
                        })
                        .map(|existing| existing.id)
                })
                .unwrap_or_else(|| {
                    arena.push(ContainerSpec {
                        name: &container.name,
                        level: container.level,
                        parent,
                        synthetic: container.synthetic,
                        label_keyed: container.label_keyed,
                    })
                });
            restored_ids.insert(container.id, id);
            parent = Some(id);
        }
        parent
    }
}
