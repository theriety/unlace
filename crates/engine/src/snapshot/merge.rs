//! Fragment merging: re-interns per-adapter ids into one namespace and rebuilds
//! the container tree from file paths.

use std::collections::{HashMap, HashSet};

use smol_str::SmolStr;
use unlace_ir::laminar::{Companions, build_laminar_tree_with_companions};
use unlace_ir::{
    ContainerId, FileImport, IntermediateRepresentation, IrFragment, Layout, Node, NodeId,
    ScopeLevel,
};

use super::visibility::apply_visibility_scopes;

/// Merges per-adapter fragments into one [`IntermediateRepresentation`],
/// re-interning every fragment's node ids into a single dense namespace and
/// rebuilding the container tree from every file path so package, domain, and
/// folder boundaries follow the manifest-derived `layout` — not the per-adapter
/// positional path slices the fragments arrive with.
///
/// Each fragment's own file-level containers name the repo-relative path of the
/// file every node lives in; those paths seed [`build_laminar_tree`], and each
/// node is reattached to its file's container in the rebuilt tree. A file an
/// adapter states as a folder companion hangs under its folder's container, as
/// that folder's entry (ADR-31).
///
/// Import facts are concatenated as stated. A fragment whose adapter states
/// its imports adds every file it carries to the import coverage, so the
/// snapshot can tell a file that imports nothing from one whose imports are
/// unknown. A file the fragment lists in `files_without_imports` stays out of
/// the coverage, and its stated imports are dropped with it. A fragment whose
/// language has no load order marks every file it carries as unordered.
pub(super) fn merge_fragments(
    fragments: Vec<IrFragment>,
    root_name: &str,
    layout: &Layout,
) -> IntermediateRepresentation {
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges = Vec::new();
    let mut affinities = Vec::new();
    let mut visibility_scopes = Vec::new();
    let mut exposures = Vec::new();
    let mut advisory_uses = Vec::new();
    let mut file_facts = Vec::new();
    let mut test_support_roots = Vec::new();
    let mut imports = Vec::new();
    let mut import_coverage = Vec::new();
    let mut unordered_files = Vec::new();
    // the file path each merged node belongs to, parallel to `nodes`.
    let mut node_paths: Vec<SmolStr> = Vec::new();
    let mut all_paths: Vec<SmolStr> = Vec::new();

    let mut node_offset = 0_u32;
    for mut fragment in fragments {
        // file-level containers are keyed by full repo-relative path; map each
        // fragment-local container id to that path so nodes can be re-homed.
        let file_path: HashMap<u32, SmolStr> = fragment
            .containers
            .iter()
            .filter(|container| container.level == ScopeLevel::File)
            .map(|container| (container.id.0, container.name.clone()))
            .collect();
        all_paths.extend(file_path.values().cloned());
        let facts = take_import_facts(&mut fragment);
        imports.extend(facts.imports);
        import_coverage.extend(facts.coverage);
        unordered_files.extend(facts.unordered);

        // each fragment's ids span 0..=max; advancing the base by one past its
        // own maximum guarantees the next fragment's re-interned ids never
        // collide.
        let node_span = id_span(fragment.nodes.iter().map(|node| node.id.0));
        for mut node in fragment.nodes {
            let path = file_path
                .get(&node.container.0)
                .cloned()
                .unwrap_or_default();
            node.id = NodeId(node.id.0 + node_offset);
            node.owner = node.owner.map(|owner| NodeId(owner.0 + node_offset));
            nodes.push(node);
            node_paths.push(path);
        }
        for mut edge in fragment.edges {
            edge.source = NodeId(edge.source.0 + node_offset);
            edge.target = NodeId(edge.target.0 + node_offset);
            edges.push(edge);
        }
        for mut affinity in fragment.affinities {
            affinity.owner = NodeId(affinity.owner.0 + node_offset);
            affinity.companion = NodeId(affinity.companion.0 + node_offset);
            affinities.push(affinity);
        }
        for mut visibility_scope in fragment.visibility_scopes {
            visibility_scope.node = NodeId(visibility_scope.node.0 + node_offset);
            visibility_scopes.push(visibility_scope);
        }
        for mut exposure in fragment.exposures {
            exposure.exposer = NodeId(exposure.exposer.0 + node_offset);
            exposure.exposed = NodeId(exposure.exposed.0 + node_offset);
            exposures.push(exposure);
        }
        for mut advisory in fragment.advisory_uses {
            advisory.user = NodeId(advisory.user.0 + node_offset);
            advisory.used = NodeId(advisory.used.0 + node_offset);
            advisory_uses.push(advisory);
        }
        file_facts.extend(fragment.file_facts);
        test_support_roots.extend(fragment.test_support_roots);
        node_offset += node_span;
    }

    let companions = Companions::from_facts(&file_facts);
    let built = build_laminar_tree_with_companions(&all_paths, root_name, layout, &companions);
    for (node, path) in nodes.iter_mut().zip(&node_paths) {
        node.container = built.files.get(path).copied().unwrap_or(ContainerId(0));
    }
    let ladders = apply_visibility_scopes(
        &mut nodes,
        &built.tree,
        &built.files,
        &edges,
        &visibility_scopes,
        &exposures,
        &advisory_uses,
    );

    let mut ir = IntermediateRepresentation::new(nodes, edges, built.tree);
    ir.affinities = affinities;
    ir.scope_ladders = ladders;
    ir.advisory_uses = advisory_uses;
    ir.file_facts = file_facts;
    ir.test_support_roots = test_support_roots;
    ir.imports = imports;
    ir.import_coverage = import_coverage;
    ir.unordered_files = unordered_files;
    ir
}

/// One fragment's import facts as the merge keeps them.
struct ImportFacts {
    /// The stated imports whose importer is covered.
    imports: Vec<FileImport>,
    /// The files whose imports are stated.
    coverage: Vec<SmolStr>,
    /// The files whose language has no load order.
    unordered: Vec<SmolStr>,
}

/// Takes `fragment`'s imports and reads its coverage. An adapter that does not
/// state its imports has no coverage, so any imports it sent would sit outside
/// it and are dropped; a file it lists as a gap leaves the coverage with every
/// import it holds, so every kept import stays inside the coverage.
fn take_import_facts(fragment: &mut IrFragment) -> ImportFacts {
    // read the container list, not a map, so the order is stable.
    let files: Vec<SmolStr> = fragment
        .containers
        .iter()
        .filter(|container| container.level == ScopeLevel::File)
        .map(|container| container.name.clone())
        .collect();
    let stated = std::mem::take(&mut fragment.imports);
    let gaps: HashSet<&SmolStr> = fragment.files_without_imports.iter().collect();
    let (imports, coverage) = if fragment.states_imports {
        (
            stated
                .into_iter()
                .filter(|import| !gaps.contains(&import.importer))
                .collect(),
            files
                .iter()
                .filter(|file| !gaps.contains(file))
                .cloned()
                .collect(),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    let unordered = if fragment.unordered_imports {
        files
    } else {
        Vec::new()
    };
    ImportFacts {
        imports,
        coverage,
        unordered,
    }
}

/// Returns one past the maximum id in `ids`, i.e. the id range width a fragment
/// occupies, or zero when the fragment is empty.
pub(super) fn id_span(ids: impl Iterator<Item = u32>) -> u32 {
    ids.max().map_or(0, |max| max + 1)
}
