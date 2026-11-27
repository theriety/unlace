use std::collections::BTreeMap;

use smol_str::SmolStr;
use unlace_ir::laminar::{Companions, build_laminar_tree_with_companions};
use unlace_ir::{
    Container, ContainerId, ContainerTree, FileFact, IntermediateRepresentation, Layout, NodeKind,
    Snapshot,
};

use super::UnitFacts;
use crate::analyze::scoring::score_candidate;
use crate::analyze::test_support::sloc_node;
use crate::config::CapacityConfig;

const ENTRY: &str = "src/lexer/block.ts";
const MEMBER: &str = "src/lexer/block/mdcx.ts";
const SIBLING: &str = "src/lexer/block/table.ts";
const OUTSIDE: &str = "src/lexer/source.ts";
const NESTED: &str = "src/lexer/block/inline/emphasis.ts";
const LOOKALIKE: &str = "src/lexer/blockquote/quote.ts";

/// Every fixture file, in node order: node `i` is the declaration of path `i`.
const ORDER: [&str; 6] = [ENTRY, MEMBER, SIBLING, OUTSIDE, NESTED, LOOKALIKE];

/// Node id of the production file body that sits in [`MEMBER`] beside its
/// declaration.
const BODY: u32 = 6;

/// A file id no pass-start file has.
const UNKNOWN: ContainerId = ContainerId(u32::MAX);

/// A unit `src/lexer/block/` with its entry `block.ts`, a member one folder
/// deeper, a file outside it and a folder whose name only starts with the
/// unit's, one declaration per file (node `i` lives in path `i`) plus a file
/// body in [`MEMBER`]. With `companion` false the folder carries no companion
/// fact, so no unit exists.
struct Fixture {
    snapshot: Snapshot,
    files: BTreeMap<&'static str, ContainerId>,
}

#[allow(clippy::panic)] // a broken fixture must fail loudly, never hide behind a fallback
fn fixture(companion: bool) -> Fixture {
    let paths: Vec<SmolStr> = ORDER.into_iter().map(SmolStr::new).collect();
    let facts: Vec<FileFact> = if companion {
        vec![FileFact::folder_companion(ENTRY, "src/lexer/block")]
    } else {
        Vec::new()
    };
    let layout = Layout {
        package_roots: Vec::new(),
        source_roots: vec![SmolStr::new("src")],
    };
    let built =
        build_laminar_tree_with_companions(&paths, "mdc", &layout, &Companions::from_facts(&facts));
    let files: BTreeMap<&'static str, ContainerId> = ORDER
        .into_iter()
        .filter_map(|path| Some((path, *built.files.get(path)?)))
        .collect();
    let mut nodes: Vec<_> = ORDER
        .into_iter()
        .enumerate()
        .filter_map(|(index, path)| {
            // reason: six fixture nodes index well inside u32.
            #[allow(clippy::cast_possible_truncation)]
            let id = index as u32;
            Some(sloc_node(id, path, *files.get(path)?, 10))
        })
        .collect();
    if let Some(member) = files.get(MEMBER) {
        let mut body = sloc_node(BODY, "body", *member, 4);
        body.kind = NodeKind::FileBody;
        nodes.push(body);
    }
    let mut ir = IntermediateRepresentation::new(nodes, Vec::new(), built.tree);
    ir.file_facts = facts;
    let snapshot = Snapshot::assemble(ir)
        .unwrap_or_else(|error| panic!("test snapshot failed to assemble: {error}"));
    Fixture { snapshot, files }
}

impl Fixture {
    fn file(&self, path: &str) -> ContainerId {
        self.files.get(path).copied().unwrap_or(UNKNOWN)
    }

    /// Node id of the declaration that starts in `path`.
    fn node(path: &str) -> u32 {
        ORDER
            .iter()
            .position(|candidate| *candidate == path)
            .and_then(|index| u32::try_from(index).ok())
            .unwrap_or(u32::MAX)
    }

    /// The pass-start file of every node.
    fn start(&self) -> BTreeMap<u32, ContainerId> {
        self.snapshot
            .ir()
            .nodes
            .iter()
            .map(|node| (node.id.0, node.container))
            .collect()
    }

    /// The flows when each `(node, destination)` pair moves and every other
    /// node stays in its pass-start file; a `None` destination leaves that
    /// node unplaced.
    fn flows_of_moves(&self, moves: &[(u32, Option<ContainerId>)]) -> (u32, u32) {
        let start = self.start();
        UnitFacts::new(&self.snapshot).declaration_flows(&|node| {
            moves.iter().find(|(moved, _)| *moved == node).map_or_else(
                || start.get(&node).copied(),
                |(_, destination)| *destination,
            )
        })
    }

    /// The flows when only the declaration of `from` moves, into `to`.
    fn flows_of_one_move(&self, from: &str, to: &str) -> (u32, u32) {
        self.flows_of_moves(&[(Self::node(from), Some(self.file(to)))])
    }

    /// The pass-start tree with `path` reparented under the folder that holds
    /// `beside`.
    fn tree_moving(&self, path: &str, beside: &str) -> ContainerTree {
        let containers = self.snapshot.ir().containers.containers();
        let target_parent = containers
            .iter()
            .find(|container| container.id == self.file(beside))
            .and_then(|container| container.parent);
        let moved: Vec<Container> = containers
            .iter()
            .cloned()
            .map(|mut container| {
                if container.id == self.file(path) {
                    container.parent = target_parent;
                }
                container
            })
            .collect();
        ContainerTree::new(moved)
    }

    /// The file outflows of the pass-start tree with `path` reparented under
    /// the folder that holds `beside`, where the moved candidate file stands
    /// for `stands_for` (no pass-start file when `None`) and every other file
    /// for itself.
    fn file_outflows_standing_for(
        &self,
        path: &str,
        beside: &str,
        stands_for: Option<ContainerId>,
    ) -> u32 {
        let mut pass_start: BTreeMap<ContainerId, ContainerId> =
            self.files.values().map(|file| (*file, *file)).collect();
        match stands_for {
            Some(start) => pass_start.insert(self.file(path), start),
            None => pass_start.remove(&self.file(path)),
        };
        UnitFacts::new(&self.snapshot).file_outflows(&self.tree_moving(path, beside), &pass_start)
    }

    /// The file outflows of the pass-start tree with `path` reparented under
    /// the folder that holds `beside`.
    fn file_outflows_moving(&self, path: &str, beside: &str) -> u32 {
        self.file_outflows_standing_for(path, beside, Some(self.file(path)))
    }
}

#[test]
fn should_count_a_declaration_moved_from_the_folder_into_its_entry_as_inflow() {
    let fixture = fixture(true);

    assert_eq!(fixture.flows_of_one_move(MEMBER, ENTRY), (1, 0));
}

#[test]
fn should_not_count_a_declaration_moved_from_the_entry_into_its_folder() {
    let fixture = fixture(true);

    assert_eq!(fixture.flows_of_one_move(ENTRY, MEMBER), (0, 0));
}

#[test]
fn should_count_a_declaration_leaving_the_unit_as_outflow() {
    let fixture = fixture(true);

    assert_eq!(fixture.flows_of_one_move(MEMBER, OUTSIDE), (0, 1));
    assert_eq!(fixture.flows_of_one_move(ENTRY, OUTSIDE), (0, 1));
}

#[test]
fn should_not_count_a_declaration_entering_the_unit_from_outside() {
    let fixture = fixture(true);

    assert_eq!(fixture.flows_of_one_move(OUTSIDE, ENTRY), (0, 0));
}

#[test]
fn should_count_a_member_file_hoisted_to_the_parent_once_as_outflow() {
    let fixture = fixture(true);

    assert_eq!(fixture.file_outflows_moving(MEMBER, OUTSIDE), 1);
}

#[test]
fn should_count_neither_term_for_a_move_within_the_folder() {
    let fixture = fixture(true);

    assert_eq!(fixture.flows_of_one_move(MEMBER, SIBLING), (0, 0));
    assert_eq!(fixture.file_outflows_moving(MEMBER, SIBLING), 0);
}

#[test]
fn should_count_neither_term_for_an_unmoved_layout() {
    let fixture = fixture(true);

    assert_eq!(fixture.flows_of_one_move(MEMBER, MEMBER), (0, 0));
    assert_eq!(fixture.file_outflows_moving(MEMBER, MEMBER), 0);
}

#[test]
fn should_count_neither_term_for_a_folder_without_a_companion() {
    let fixture = fixture(false);

    assert_eq!(fixture.flows_of_one_move(MEMBER, ENTRY), (0, 0));
    assert_eq!(fixture.flows_of_one_move(MEMBER, OUTSIDE), (0, 0));
    assert_eq!(fixture.file_outflows_moving(MEMBER, OUTSIDE), 0);
}

#[test]
fn should_count_a_declaration_moved_from_a_nested_folder_into_the_entry_as_inflow() {
    let fixture = fixture(true);

    assert_eq!(fixture.flows_of_one_move(NESTED, ENTRY), (1, 0));
}

#[test]
fn should_count_a_nested_file_hoisted_out_of_the_unit_as_outflow() {
    let fixture = fixture(true);

    assert_eq!(fixture.file_outflows_moving(NESTED, OUTSIDE), 1);
}

#[test]
fn should_keep_a_nested_file_moved_up_to_the_unit_folder_inside_the_unit() {
    let fixture = fixture(true);

    assert_eq!(fixture.file_outflows_moving(NESTED, MEMBER), 0);
}

#[test]
fn should_not_treat_a_folder_that_only_shares_the_unit_name_prefix_as_inside_it() {
    let fixture = fixture(true);

    assert_eq!(fixture.flows_of_one_move(LOOKALIKE, ENTRY), (0, 0));
    assert_eq!(fixture.flows_of_one_move(LOOKALIKE, OUTSIDE), (0, 0));
}

#[test]
fn should_not_count_a_file_body_moved_into_the_entry() {
    let fixture = fixture(true);

    assert_eq!(
        fixture.flows_of_moves(&[(BODY, Some(fixture.file(ENTRY)))]),
        (0, 0)
    );
}

#[test]
fn should_skip_an_unplaced_declaration_and_still_count_the_others() {
    let fixture = fixture(true);

    let flows = fixture.flows_of_moves(&[
        (Fixture::node(MEMBER), None),
        (Fixture::node(SIBLING), Some(fixture.file(ENTRY))),
    ]);

    assert_eq!(flows, (1, 0));
}

#[test]
fn should_count_a_declaration_placed_in_a_file_unknown_at_pass_start_as_outflow_only() {
    let fixture = fixture(true);

    assert_eq!(
        fixture.flows_of_moves(&[(Fixture::node(MEMBER), Some(UNKNOWN))]),
        (0, 1)
    );
}

#[test]
fn should_not_count_a_hoisted_candidate_file_without_a_pass_start_file() {
    let fixture = fixture(true);

    assert_eq!(fixture.file_outflows_standing_for(MEMBER, OUTSIDE, None), 0);
}

#[test]
fn should_not_count_a_hoisted_candidate_file_standing_for_an_unknown_pass_start_file() {
    let fixture = fixture(true);

    assert_eq!(
        fixture.file_outflows_standing_for(MEMBER, OUTSIDE, Some(UNKNOWN)),
        0
    );
}

#[test]
fn should_carry_entry_inflow_and_unit_outflow_into_the_scored_candidate() {
    let fixture = fixture(true);
    let start = fixture.start();
    let (member, sibling) = (Fixture::node(MEMBER), Fixture::node(SIBLING));
    let (into_entry, out_of_unit) = (fixture.file(ENTRY), fixture.file(OUTSIDE));
    let placement = |node: u32| {
        if node == member {
            Some(into_entry)
        } else if node == sibling {
            Some(out_of_unit)
        } else {
            start.get(&node).copied()
        }
    };
    let pass_start: BTreeMap<ContainerId, ContainerId> =
        fixture.files.values().map(|file| (*file, *file)).collect();

    let candidate = score_candidate(
        &fixture.snapshot,
        &placement,
        &pass_start,
        &fixture.snapshot.ir().containers,
        &BTreeMap::new(),
        0.0,
        &CapacityConfig::default(),
        1.0,
        1.0,
    );

    assert_eq!((candidate.entry_inflows, candidate.unit_outflows), (1, 1));
}
