//! The conservative pool of alternative places a proposal is compared against.

use std::collections::BTreeSet;

use unlace_ir::{NodeId, Polarity};

use crate::analyze::advice::evidence::measure::dependency_weight;
use crate::analyze::advice::evidence::{EvidenceIndex, EvidencePlace};
use crate::analyze::advice::proposal::AdviceProposal;
use crate::config::ProfileConfig;
use crate::result::RelocationProposal;

impl EvidenceIndex<'_> {
    pub(super) fn alternatives(
        &self,
        subjects: &BTreeSet<NodeId>,
        destination: &EvidencePlace,
        proposal: &AdviceProposal,
        config: &ProfileConfig,
    ) -> Vec<EvidencePlace> {
        let wants_file = matches!(proposal.proposal, RelocationProposal::Symbol { .. });
        let mut places = BTreeSet::new();
        let mut consumer_folders = BTreeSet::new();
        if let Some(source) = self.resolve_source(proposal) {
            places.insert(source);
        }
        for edge in &self.ir.edges {
            if dependency_weight(edge.kind, config) * edge.confidence <= 0.0 {
                continue;
            }
            let other = if subjects.contains(&edge.source) && !subjects.contains(&edge.target) {
                Some(edge.target)
            } else if subjects.contains(&edge.target) && !subjects.contains(&edge.source) {
                Some(edge.source)
            } else {
                None
            };
            if let Some(node) = other
                .and_then(|id| self.nodes.get(&id))
                .filter(|node| node.polarity == Polarity::Production)
            {
                places.insert(if wants_file {
                    EvidencePlace::Container(node.container)
                } else {
                    self.physical_folder_for_file(node.container)
                        .unwrap_or(EvidencePlace::Container(node.container))
                });
            }
            if subjects.contains(&edge.target)
                && !subjects.contains(&edge.source)
                && let Some(folder) = self
                    .nodes
                    .get(&edge.source)
                    .filter(|node| node.polarity == Polarity::Production)
                    .and_then(|node| self.physical_folder_of_file(node.container))
                    .map(|(_, path)| path.to_owned())
            {
                consumer_folders.insert(folder);
            }
        }
        // The consumer LCA is a conceptual alternative for either relocation grain.  In
        // particular, a symbol proposed into one sibling branch must clearly beat the shared
        // folder even though symbol search itself ultimately nominates concrete files.
        if consumer_folders.len() >= 2 {
            let names = consumer_folders
                .iter()
                .map(|name| name.split('/').collect::<Vec<_>>())
                .collect::<Vec<_>>();
            // two or more consumer folders, so `first` always exists.
            let common = names
                .first()
                .map(|first| {
                    let common_len = (0..first.len())
                        .take_while(|i| names.iter().all(|parts| parts.get(*i) == first.get(*i)))
                        .count();
                    first
                        .iter()
                        .take(common_len)
                        .copied()
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .unwrap_or_default();
            if !common.is_empty() {
                let place = self
                    .resolve_folder_place(&common)
                    .unwrap_or(EvidencePlace::ConceptualFolder(common));
                places.insert(place);
            }
        }
        for affinity in &self.ir.affinities {
            if subjects.contains(&affinity.companion)
                && let Some(owner) = self.nodes.get(&affinity.owner)
            {
                places.insert(if wants_file {
                    EvidencePlace::Container(owner.container)
                } else {
                    self.physical_folder_for_file(owner.container)
                        .unwrap_or(EvidencePlace::Container(owner.container))
                });
            }
        }
        places.remove(destination);
        places.into_iter().collect()
    }
}

#[cfg(test)]
mod companion_tests {
    use std::collections::BTreeSet;

    use unlace_ir::{Affinity, AffinityKind, ContainerId, NodeId, Polarity, ScopeLevel};

    use crate::analyze::advice::evidence::{EvidenceIndex, EvidencePlace};
    use crate::analyze::advice::tests::{symbol_advice, typed_node};
    use crate::analyze::test_support::{container, node, snapshot_with_affinities};
    use crate::config::ProfileConfig;

    #[test]
    fn should_offer_only_the_owners_of_the_subjects_own_companions() {
        let owned = |owner, companion| Affinity {
            owner: NodeId(owner),
            companion: NodeId(companion),
            kind: AffinityKind::CompanionOwner,
        };
        let snapshot = snapshot_with_affinities(
            vec![
                typed_node(0, "RecordState", 4),
                node(1, "recordOwner", 5, Polarity::Production),
                typed_node(2, "OtherState", 4),
                node(3, "otherOwner", 6, Polarity::Production),
            ],
            Vec::new(),
            vec![owned(1, 0), owned(3, 2)],
            vec![
                container(0, "workspace", ScopeLevel::PackageGroup, None),
                container(1, "origin", ScopeLevel::Folder, Some(0)),
                container(2, "owner", ScopeLevel::Folder, Some(0)),
                container(3, "other", ScopeLevel::Folder, Some(0)),
                container(4, "origin/state.ts", ScopeLevel::File, Some(1)),
                container(5, "owner/record.ts", ScopeLevel::File, Some(2)),
                container(6, "other/other.ts", ScopeLevel::File, Some(3)),
            ],
        );
        let index = EvidenceIndex::new(&snapshot);
        let subjects = BTreeSet::from([NodeId(0)]);
        let elsewhere = EvidencePlace::ConceptualFolder("elsewhere".to_owned());
        let alternatives = |from: &str| {
            index.alternatives(
                &subjects,
                &elsewhere,
                &symbol_advice("RecordState", from, "owner/record.ts"),
                &ProfileConfig::default(),
            )
        };

        assert_eq!(
            alternatives("origin/state.ts"),
            vec![
                EvidencePlace::Container(ContainerId(4)),
                EvidencePlace::Container(ContainerId(5)),
            ]
        );
        assert_eq!(
            alternatives("missing/state.ts"),
            vec![EvidencePlace::Container(ContainerId(5))],
            "an unresolved source offers no stay alternative"
        );
    }
}
