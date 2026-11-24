//! Resolution of a proposal's subject, source, and destination to nodes and
//! evidence places in the pass-start snapshot.

use std::collections::BTreeSet;

use unlace_ir::{ContainerId, NodeId, NodeKind};

use crate::analyze::advice::evidence::{EvidenceIndex, EvidencePlace};
use crate::analyze::advice::proposal::AdviceProposal;
use crate::result::{RelocationProposal, SymbolKind};

impl EvidenceIndex<'_> {
    pub(super) fn subject_nodes(&self, proposal: &AdviceProposal) -> BTreeSet<NodeId> {
        match &proposal.proposal {
            RelocationProposal::Symbol { relocation } => {
                let source = self.resolve_file_path(&relocation.from_path);
                let expected_kind = match relocation.kind {
                    SymbolKind::Symbol => NodeKind::Symbol,
                    SymbolKind::Type => NodeKind::Type,
                };
                let matches = self
                    .ir
                    .nodes
                    .iter()
                    .filter(|node| {
                        node.name == relocation.symbol
                            && node.kind == expected_kind
                            && source == Some(node.container)
                    })
                    .map(|node| node.id)
                    .collect::<Vec<_>>();
                // A report atom must resolve to exactly one pass-start declaration.  Ambiguous
                // identity carries no qualifying evidence and therefore remains review-only.
                if matches.len() == 1 {
                    matches.into_iter().collect()
                } else {
                    BTreeSet::new()
                }
            }
            RelocationProposal::File { relocation } => {
                relocation.files.first().map_or_else(BTreeSet::new, |file| {
                    self.ir
                        .nodes
                        .iter()
                        .filter(|node| self.resolve_file_path(&file.path) == Some(node.container))
                        .map(|node| node.id)
                        .collect()
                })
            }
        }
    }
    fn repository_relative<'b>(&self, path: &'b str) -> &'b str {
        let Some(root) = self.dataset_root.as_deref() else {
            return path;
        };
        if path == root {
            ""
        } else {
            path.strip_prefix(root)
                .and_then(|rest| rest.strip_prefix('/'))
                .unwrap_or(path)
        }
    }
    fn resolve_file_path(&self, path: &str) -> Option<ContainerId> {
        self.files_by_path
            .get(self.repository_relative(path))
            .copied()
            .flatten()
    }
    fn resolve_folder_path(&self, path: &str) -> Option<ContainerId> {
        self.folders_by_path
            .get(self.repository_relative(path))
            .copied()
            .flatten()
    }
    pub(super) fn resolve_destination(&self, proposal: &AdviceProposal) -> Option<EvidencePlace> {
        match proposal.proposal {
            RelocationProposal::Symbol { .. } => self
                .resolve_file_path(&proposal.destination)
                .map(EvidencePlace::Container),
            RelocationProposal::File { .. } => self.resolve_folder_place(&proposal.destination),
        }
    }
    pub(super) fn resolve_source(&self, proposal: &AdviceProposal) -> Option<EvidencePlace> {
        match &proposal.proposal {
            RelocationProposal::Symbol { relocation } => self
                .resolve_file_path(&relocation.from_path)
                .map(EvidencePlace::Container),
            RelocationProposal::File { relocation } => relocation
                .files
                .first()
                .and_then(|f| self.resolve_file_path(&f.path))
                .and_then(|id| self.physical_folder_for_file(id)),
        }
    }
    pub(super) fn resolve_folder_place(&self, path: &str) -> Option<EvidencePlace> {
        let relative = self.repository_relative(path);
        self.resolve_folder_path(relative)
            .map(|container| EvidencePlace::PhysicalFolder {
                container,
                path: relative.to_owned(),
            })
    }
    pub(super) fn physical_folder_for_file(&self, file: ContainerId) -> Option<EvidencePlace> {
        self.physical_folder_of_file(file)
            .map(|(container, path)| EvidencePlace::PhysicalFolder {
                container,
                path: path.to_owned(),
            })
    }
    /// The physical folder directly holding `file`: its parent container and
    /// its folder path, or `None` when `file` is no file or has no parent.
    pub(super) fn physical_folder_of_file(&self, file: ContainerId) -> Option<(ContainerId, &str)> {
        let path = self.file_paths_by_id.get(&file)?;
        let folder_path = path.rsplit_once('/').map_or("", |(folder, _)| folder);
        // `file_paths_by_id` holds only containers of the same tree.
        let container = self
            .containers
            .get(&file)
            .and_then(|container| container.parent)?;
        Some((container, folder_path))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use unlace_ir::{ContainerId, NodeId, ScopeLevel};

    use crate::analyze::advice::evidence::{EvidenceIndex, EvidencePlace};
    use crate::analyze::advice::tests::{symbol_advice, typed_node};
    use crate::analyze::test_support::{container, snapshot};

    #[test]
    fn should_resolve_paths_verbatim_when_no_single_dataset_root_exists() {
        let snapshot = snapshot(
            vec![typed_node(0, "RecordState", 2)],
            Vec::new(),
            vec![
                container(0, "alpha", ScopeLevel::PackageGroup, None),
                container(1, "beta", ScopeLevel::PackageGroup, None),
                container(2, "src/state.ts", ScopeLevel::File, Some(0)),
            ],
        );
        let index = EvidenceIndex::new(&snapshot);

        assert_eq!(
            index.subject_nodes(&symbol_advice("RecordState", "src/state.ts", "src/x.ts")),
            BTreeSet::from([NodeId(0)])
        );
        assert!(
            index
                .subject_nodes(&symbol_advice(
                    "RecordState",
                    "alpha/src/state.ts",
                    "src/x.ts"
                ))
                .is_empty(),
            "without one dataset root no group prefix is stripped"
        );
    }

    #[test]
    fn should_resolve_the_dataset_root_itself_as_the_repository_root_folder() {
        let snapshot = snapshot(
            vec![typed_node(0, "RecordState", 1)],
            Vec::new(),
            vec![
                container(0, "dataset", ScopeLevel::PackageGroup, None),
                container(1, "state.ts", ScopeLevel::File, Some(0)),
            ],
        );
        let index = EvidenceIndex::new(&snapshot);

        assert_eq!(
            index.resolve_folder_place("dataset"),
            Some(EvidencePlace::PhysicalFolder {
                container: ContainerId(0),
                path: String::new(),
            })
        );
        assert_eq!(index.resolve_folder_place("dataset/src"), None);
    }

    #[test]
    fn should_find_no_physical_folder_for_a_file_at_the_tree_root() {
        let snapshot = snapshot(
            vec![typed_node(0, "RecordState", 0)],
            Vec::new(),
            vec![container(0, "state.ts", ScopeLevel::File, None)],
        );
        let index = EvidenceIndex::new(&snapshot);

        assert_eq!(index.physical_folder_for_file(ContainerId(0)), None);
    }
}
