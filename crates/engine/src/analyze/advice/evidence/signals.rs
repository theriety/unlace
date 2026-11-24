//! The evidence signals of one candidate destination: ownership, role affinity,
//! cohesion, and architectural reach.

use std::collections::BTreeSet;

use unlace_ir::{AffinityKind, ContainerId, NodeId, Polarity};

use crate::analyze::advice::evidence::measure::{
    dependency_weight, jaccard_tokens, usize_as_f64, usize_ratio,
};
use crate::analyze::advice::evidence::{EvidenceIndex, EvidencePlace};
use crate::analyze::advice::proposal::AdviceProposal;
use crate::config::ProfileConfig;
use crate::narrate::tokenize;
use crate::result::EvidenceSignals;

impl EvidenceIndex<'_> {
    pub(super) fn signals(
        &self,
        subjects: &BTreeSet<NodeId>,
        destination: &EvidencePlace,
        proposal: &AdviceProposal,
        config: &ProfileConfig,
    ) -> EvidenceSignals {
        let owners = self
            .ir
            .affinities
            .iter()
            .filter(|a| a.kind == AffinityKind::CompanionOwner && subjects.contains(&a.companion))
            .filter_map(|a| self.nodes.get(&a.owner).copied())
            .collect::<Vec<_>>();
        let owners_in_destination = owners
            .iter()
            .filter(|node| self.node_at_evidence_place(node, destination))
            .count();
        let subject_tokens = subjects
            .iter()
            .filter_map(|id| self.nodes.get(id))
            .flat_map(|node| tokenize(&node.name))
            .collect::<BTreeSet<_>>();
        let destination_tokens = self
            .destination_nodes_at(destination)
            .into_iter()
            .flat_map(|node| tokenize(&node.name))
            .chain(
                Some(self.place_name(destination, proposal))
                    .into_iter()
                    .flat_map(|name| tokenize(&name)),
            )
            .collect::<BTreeSet<_>>();
        let source = self.resolve_source(proposal);
        let mut wd = 0.0;
        let mut ws = 0.0;
        let mut wall = 0.0;
        let mut neighbors = BTreeSet::new();
        let mut consumers = BTreeSet::new();
        for edge in &self.ir.edges {
            let (neighbor, incoming) =
                if subjects.contains(&edge.source) && !subjects.contains(&edge.target) {
                    (edge.target, false)
                } else if subjects.contains(&edge.target) && !subjects.contains(&edge.source) {
                    (edge.source, true)
                } else {
                    continue;
                };
            let Some(node) = self
                .nodes
                .get(&neighbor)
                .copied()
                .filter(|node| node.polarity == Polarity::Production)
            else {
                continue;
            };
            let weight = edge.confidence * dependency_weight(edge.kind, config);
            if weight <= 0.0 {
                continue;
            }
            wall += weight;
            if self.node_at_evidence_place(node, destination) {
                wd += weight;
                neighbors.insert(node.id);
            }
            if source
                .as_ref()
                .is_some_and(|place| self.node_at_evidence_place(node, place))
            {
                ws += weight;
            }
            if incoming {
                consumers.insert(node.container);
            }
        }
        let source_cohesion = ((wd - ws) / wd.max(ws).max(f64::EPSILON)).clamp(0.0, 1.0);
        let destination_cohesion = if wall > 0.0 {
            let breadth = match neighbors.len() {
                0 => 0.0,
                1 => 0.5,
                _ => 1.0,
            };
            (wd / wall) * breadth
        } else {
            0.0
        };
        EvidenceSignals {
            unique_owner: f64::from(owners.len() == 1 && owners_in_destination == 1),
            role_affinity: jaccard_tokens(&subject_tokens, &destination_tokens),
            source_cohesion,
            destination_cohesion,
            producer_evidence: if owners.is_empty() {
                0.0
            } else {
                usize_ratio(owners_in_destination, owners.len())
            },
            architectural_reach: self.architectural_reach(destination, &consumers),
        }
    }
    fn architectural_reach(
        &self,
        destination: &EvidencePlace,
        consumer_files: &BTreeSet<ContainerId>,
    ) -> f64 {
        if consumer_files.len() < 2 {
            return 0.0;
        }
        let folders = consumer_files
            .iter()
            .filter_map(|id| self.physical_folder_of_file(*id))
            .map(|(_, name)| name.split('/').map(str::to_owned).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let Some(first) = folders.first() else {
            return 0.0;
        };
        let common_len = (0..first.len())
            .take_while(|index| {
                folders
                    .iter()
                    .all(|parts| parts.get(*index) == first.get(*index))
            })
            .count();
        let Some(destination_name) = self.physical_folder_name(destination) else {
            return 0.0;
        };
        let destination_parts = destination_name
            .split('/')
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if destination_parts.len() == common_len
            && first.get(..common_len) == Some(destination_parts.as_slice())
        {
            1.0
        } else if destination_parts.len() < common_len
            && first.get(..destination_parts.len()) == Some(destination_parts.as_slice())
        {
            1.0 / (1.0 + usize_as_f64(common_len - destination_parts.len()))
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod reach_tests {
    use std::collections::BTreeSet;

    use unlace_ir::{ContainerId, NodeId, Polarity, ScopeLevel, Snapshot};

    use crate::analyze::advice::evidence::{EvidenceIndex, EvidencePlace};
    use crate::analyze::advice::tests::{symbol_advice, typed_node};
    use crate::analyze::test_support::{container, node, reexport, snapshot, type_ref};
    use crate::config::ProfileConfig;
    use crate::result::EvidenceSignals;

    fn signals_at(
        snapshot: &Snapshot,
        subject: u32,
        (symbol, from, to): (&str, &str, &str),
        place: &EvidencePlace,
    ) -> EvidenceSignals {
        let index = EvidenceIndex::new(snapshot);
        let subjects = BTreeSet::from([NodeId(subject)]);
        index.signals(
            &subjects,
            place,
            &symbol_advice(symbol, from, to),
            &ProfileConfig::default(),
        )
    }

    fn conceptual(path: &str) -> EvidencePlace {
        EvidencePlace::ConceptualFolder(path.to_owned())
    }

    #[test]
    fn should_scale_reach_by_how_far_the_destination_sits_above_the_consumer_lca() {
        let snapshot = snapshot(
            vec![
                typed_node(0, "RecordState", 4),
                node(1, "leftConsumer", 5, Polarity::Production),
                node(2, "rightConsumer", 6, Polarity::Production),
            ],
            vec![type_ref(1, 0), type_ref(2, 0)],
            vec![
                container(0, "workspace", ScopeLevel::PackageGroup, None),
                container(1, "origin", ScopeLevel::Folder, Some(0)),
                container(2, "shared/inner/left", ScopeLevel::Folder, Some(0)),
                container(3, "shared/inner/right", ScopeLevel::Folder, Some(0)),
                container(4, "origin/state.ts", ScopeLevel::File, Some(1)),
                container(5, "shared/inner/left/use.ts", ScopeLevel::File, Some(2)),
                container(6, "shared/inner/right/use.ts", ScopeLevel::File, Some(3)),
            ],
        );
        let proposal = ("RecordState", "origin/state.ts", "shared/inner/left/use.ts");
        let reach =
            |path: &str| signals_at(&snapshot, 0, proposal, &conceptual(path)).architectural_reach;

        assert!((reach("shared/inner") - 1.0).abs() < f64::EPSILON);
        assert!((reach("shared") - 0.5).abs() < f64::EPSILON);
        assert!(reach("shared/inner/left").abs() < f64::EPSILON);
        assert!(reach("origin").abs() < f64::EPSILON);
        assert!(
            signals_at(&snapshot, 0, proposal, &conceptual("shared/inner"))
                .destination_cohesion
                .abs()
                < f64::EPSILON,
            "a conceptual folder holds no node, so no dependency lands in it"
        );
    }

    #[test]
    fn should_give_no_reach_when_consumers_or_the_destination_lack_a_physical_folder() {
        let snapshot = snapshot(
            vec![
                typed_node(0, "LooseState", 3),
                node(1, "looseLeft", 4, Polarity::Production),
                node(2, "looseRight", 5, Polarity::Production),
                typed_node(3, "FolderState", 3),
                node(4, "sharedLeft", 6, Polarity::Production),
                node(5, "sharedRight", 7, Polarity::Production),
            ],
            vec![
                type_ref(1, 0),
                type_ref(2, 0),
                type_ref(4, 3),
                type_ref(5, 3),
            ],
            vec![
                container(0, "workspace", ScopeLevel::PackageGroup, None),
                container(1, "origin", ScopeLevel::Folder, Some(0)),
                container(2, "shared", ScopeLevel::Folder, Some(0)),
                container(3, "origin/state.ts", ScopeLevel::File, Some(1)),
                container(4, "loose-left.ts", ScopeLevel::File, None),
                container(5, "loose-right.ts", ScopeLevel::File, None),
                container(6, "shared/left.ts", ScopeLevel::File, Some(2)),
                container(7, "shared/right.ts", ScopeLevel::File, Some(2)),
            ],
        );
        let loose = ("LooseState", "origin/state.ts", "shared/left.ts");
        let folder = ("FolderState", "origin/state.ts", "shared/left.ts");
        let reach = |subject, proposal, file| {
            signals_at(
                &snapshot,
                subject,
                proposal,
                &EvidencePlace::Container(ContainerId(file)),
            )
            .architectural_reach
        };

        assert!(
            reach(0, loose, 6).abs() < f64::EPSILON,
            "parentless consumer files share no physical folder"
        );
        assert!((reach(3, folder, 6) - 1.0).abs() < f64::EPSILON);
        assert!(
            reach(3, folder, 4).abs() < f64::EPSILON,
            "a parentless destination file sits in no physical folder"
        );
        let index = EvidenceIndex::new(&snapshot);
        assert_eq!(index.physical_folder_for_file(ContainerId(4)), None);
        assert_eq!(
            index.physical_folder_for_file(ContainerId(2)),
            None,
            "a folder is not a file and has no physical folder of its own"
        );
    }

    #[test]
    fn should_ignore_zero_weight_re_exports_as_evidence() {
        let snapshot = snapshot(
            vec![
                typed_node(0, "RecordState", 4),
                node(1, "leftBarrel", 5, Polarity::Production),
                node(2, "rightBarrel", 6, Polarity::Production),
            ],
            vec![reexport(1, 0), reexport(2, 0)],
            vec![
                container(0, "workspace", ScopeLevel::PackageGroup, None),
                container(1, "origin", ScopeLevel::Folder, Some(0)),
                container(2, "index/left", ScopeLevel::Folder, Some(0)),
                container(3, "index/right", ScopeLevel::Folder, Some(0)),
                container(4, "origin/state.ts", ScopeLevel::File, Some(1)),
                container(5, "index/left/barrel.ts", ScopeLevel::File, Some(2)),
                container(6, "index/right/barrel.ts", ScopeLevel::File, Some(3)),
            ],
        );
        let proposal = ("RecordState", "origin/state.ts", "index/left/barrel.ts");
        let at_barrel = signals_at(
            &snapshot,
            0,
            proposal,
            &EvidencePlace::Container(ContainerId(5)),
        );

        assert!(at_barrel.destination_cohesion.abs() < f64::EPSILON);
        assert!(at_barrel.source_cohesion.abs() < f64::EPSILON);
        assert!(
            signals_at(&snapshot, 0, proposal, &conceptual("index"))
                .architectural_reach
                .abs()
                < f64::EPSILON,
            "re-exporting barrels are no consumers, so their common folder earns no reach"
        );
        let index = EvidenceIndex::new(&snapshot);
        assert_eq!(
            index.alternatives(
                &BTreeSet::from([NodeId(0)]),
                &conceptual("elsewhere"),
                &symbol_advice(proposal.0, proposal.1, proposal.2),
                &ProfileConfig::default(),
            ),
            vec![EvidencePlace::Container(ContainerId(4))],
            "only the stay placement is an alternative; zero-weight barrels are not"
        );
    }

    #[test]
    fn should_offset_the_destination_pull_by_the_pull_still_left_at_the_source() {
        let fixture = |edges| {
            snapshot(
                vec![
                    typed_node(0, "RecordState", 3),
                    node(1, "originUse", 3, Polarity::Production),
                    node(2, "sharedUse", 4, Polarity::Production),
                ],
                edges,
                vec![
                    container(0, "workspace", ScopeLevel::PackageGroup, None),
                    container(1, "origin", ScopeLevel::Folder, Some(0)),
                    container(2, "shared", ScopeLevel::Folder, Some(0)),
                    container(3, "origin/state.ts", ScopeLevel::File, Some(1)),
                    container(4, "shared/use.ts", ScopeLevel::File, Some(2)),
                ],
            )
        };
        let proposal = ("RecordState", "origin/state.ts", "shared/use.ts");
        let place = EvidencePlace::Container(ContainerId(4));
        let pulled_only_away = fixture(vec![type_ref(2, 0)]);
        let pulled_both_ways = fixture(vec![type_ref(1, 0), type_ref(2, 0)]);

        let away = signals_at(&pulled_only_away, 0, proposal, &place).source_cohesion;
        let both = signals_at(&pulled_both_ways, 0, proposal, &place).source_cohesion;

        assert!((away - 1.0).abs() < f64::EPSILON, "{away}");
        assert!(
            both.abs() < f64::EPSILON,
            "an equal pull at the source cancels the destination pull: {both}"
        );
    }
}
