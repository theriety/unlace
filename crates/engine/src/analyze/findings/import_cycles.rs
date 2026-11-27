//! Cycle findings on the file-import graph the adapters state (ADR-35).
//!
//! A cycle finding is a strongly connected component of two or more files in
//! the graph of [`FileImport`] facts. Parallel statements between one file
//! pair collapse into one edge that keeps every statement, and the finding
//! names the imports whose removal breaks the cycle ([`cut`]). Where a file
//! states no imports, the import graph cannot see a cycle through it, so a
//! cross-file component of the hard symbol graph that touches such a file is
//! reported as unavailable rather than left out.

use std::collections::{BTreeMap, BTreeSet};

use unlace_core::condense::condense;
use unlace_core::graph::csr::{HardnessFilter, build_csr};
use unlace_ir::{EdgeKind, Hardness, NodeId, ScopeLevel, Snapshot};

use crate::analyze::search::node_names;
use unlace_ir::LoadOrder;

use crate::result::{
    CycleAnalysis, CycleCoverage, ImportBreak, ImportCycle, ImportCycleStatus, ImportStatement,
    NoLoadOrder, Severity, SymbolReference, Violation, ViolationKind,
};

mod cut;
mod direction;
mod graph;

#[cfg(test)]
mod tests;

use direction::{EdgeTier, EntryFolders, lexicographic_weights};
use graph::{ImportGraph, strongly_connected};

/// Reports every cross-file import cycle with its cut, then every cross-file
/// symbol cycle the import graph cannot see because a file on it states no
/// imports.
///
/// The symbol cycles serve only as witnesses, so they come from the
/// condensation of the hard symbol graph alone; no break set is solved for
/// them.
pub(super) fn import_cycle_violations(snapshot: &Snapshot) -> Vec<Violation> {
    let mut violations = cut_violations(snapshot);
    let covered: BTreeSet<String> = violations
        .iter()
        .flat_map(|violation| violation.location.iter().cloned())
        .collect();
    violations.extend(unavailable_violations(
        snapshot,
        &symbol_components(snapshot),
        &covered,
    ));
    violations
}

/// The multi-member strongly connected components of the hard symbol graph,
/// each ascending by node id.
fn symbol_components(snapshot: &Snapshot) -> Vec<Vec<NodeId>> {
    let views = build_csr(snapshot, HardnessFilter::HardOnly);
    condense(&views.forward)
        .members
        .into_iter()
        .filter(|members| members.len() > 1)
        .collect()
}

/// Measures how much of the file-import graph the cycle findings see.
pub(in crate::analyze) fn cycle_coverage(snapshot: &Snapshot) -> CycleCoverage {
    let mut files = 0_u32;
    let mut without = 0_u32;
    for container in snapshot.ir().containers.containers() {
        if container.level != ScopeLevel::File {
            continue;
        }
        files = files.saturating_add(1);
        if !snapshot.imports_available_for(&container.name) {
            without = without.saturating_add(1);
        }
    }
    let status = if without == 0 {
        CycleAnalysis::Complete
    } else if without == files {
        CycleAnalysis::Unavailable
    } else {
        CycleAnalysis::Partial
    };
    CycleCoverage {
        status,
        files_without_imports: without,
    }
}

/// One finding per multi-file component of the import graph, naming its cut.
///
/// The cut is priced by [`direction`]: an entry's import into its own folder
/// is the last to remove, a child's import back up to an entry the first.
/// The finding is a violation when an import of the component runs at load
/// time, and borderline otherwise ([`component_load_order`]).
fn cut_violations(snapshot: &Snapshot) -> Vec<Violation> {
    let graph = ImportGraph::new(&snapshot.ir().imports);
    let references = carried_references(snapshot, &graph);
    let entries = EntryFolders::from_facts(&snapshot.ir().file_facts);
    strongly_connected(&graph)
        .into_iter()
        .map(|component| {
            // the component's edges as `(local, global)` pairs, ascending: local
            // indices follow the component's ascending path order, so both
            // orders agree.
            let local_edges: Vec<((usize, usize), (usize, usize))> = graph
                .edges()
                .filter_map(|(importer, imported)| {
                    let local_importer = component.binary_search(&importer).ok()?;
                    let local_imported = component.binary_search(&imported).ok()?;
                    Some(((local_importer, local_imported), (importer, imported)))
                })
                .collect();
            let priced: BTreeMap<(usize, usize), (u64, EdgeTier)> = local_edges
                .iter()
                .map(|&(local, (importer, imported))| {
                    let carried = references.get(&(importer, imported)).map_or(0, Vec::len);
                    let tier = EdgeTier::of(graph.file(importer), graph.file(imported), &entries);
                    (local, (u64::from(edge_weight(carried)), tier))
                })
                .collect();
            let solved = cut::solve(component.len(), &lexicographic_weights(&priced));
            // the cut lists local edges ascending, so its breaks keep that order.
            let breaks = local_edges
                .iter()
                .filter(|(local, _)| solved.edges.binary_search(local).is_ok())
                .map(|&(_, (importer, imported))| {
                    import_break(&graph, importer, imported, &references)
                })
                .collect();
            let location: Vec<String> = component
                .iter()
                .map(|&file| graph.file(file).to_owned())
                .collect();
            let cycle = ImportCycle {
                status: ImportCycleStatus::Cut,
                cut: breaks,
                exact: solved.exact,
                files_without_imports: Vec::new(),
                no_load_order: component_load_order(&graph, &component),
            };
            violation(location, cycle)
        })
        .collect()
}

/// Why no import of `component` runs at load time, or `None` when one does.
///
/// Every edge of a strongly connected component lies on a cycle, so one
/// eager statement anywhere in it is a load-time hazard for whichever entry
/// point closes that cycle; the component is safe only when none is eager.
/// It is [`NoLoadOrder::Unordered`] when every statement is unordered, and
/// [`NoLoadOrder::Deferred`] otherwise.
fn component_load_order(graph: &ImportGraph<'_>, component: &[usize]) -> Option<NoLoadOrder> {
    let mut unordered = true;
    for &importer in component {
        for &imported in component {
            for statement in graph.statements(importer, imported) {
                match statement.load_order {
                    LoadOrder::Eager => return None,
                    LoadOrder::Deferred => unordered = false,
                    LoadOrder::Unordered => {}
                }
            }
        }
    }
    Some(if unordered {
        NoLoadOrder::Unordered
    } else {
        NoLoadOrder::Deferred
    })
}

/// One unavailable finding per cross-file symbol cycle touching a file that
/// states no imports. A symbol cycle whose files all state their imports is
/// left to the import graph, which is authoritative for them.
///
/// A finding reports only what no cut finding already reports. It is dropped
/// when cut findings cover every file of the symbol cycle, or when every file
/// states its imports. Otherwise its location is the uncovered files plus the
/// fact-less ones, so a symbol cycle over `a`, `b` and a fact-less `c`, beside
/// an import cycle `a ⇄ b`, yields the cut over `a, b` and an unavailable
/// finding over `c` alone. A fact-less file a cut finding lists is still
/// named when the cycle reaches an uncovered file through it.
fn unavailable_violations(
    snapshot: &Snapshot,
    symbol_cycles: &[Vec<NodeId>],
    covered: &BTreeSet<String>,
) -> Vec<Violation> {
    let file_of = node_files(snapshot);
    let mut seen: BTreeSet<Vec<String>> = BTreeSet::new();
    symbol_cycles
        .iter()
        .filter_map(|members| {
            // distinct containers, not names: only proven same-file recursion
            // stays unreported.
            let containers: BTreeSet<(u32, &str)> = members
                .iter()
                .filter_map(|member| file_of.get(&member.0).copied())
                .collect();
            if containers.len() < 2 {
                return None;
            }
            let component: BTreeSet<&str> = containers.iter().map(|(_, file)| *file).collect();
            // missing is read over the whole component, before cut findings
            // narrow it: a fact-less file a cut finding also lists still
            // leaves an uncovered file's cycle unexplained.
            let missing: BTreeSet<&str> = component
                .iter()
                .copied()
                .filter(|file| !snapshot.imports_available_for(file))
                .collect();
            let uncovered: BTreeSet<&str> = component
                .iter()
                .copied()
                .filter(|file| !covered.contains(*file))
                .collect();
            if missing.is_empty() || uncovered.is_empty() {
                return None;
            }
            let location: Vec<String> = uncovered
                .union(&missing)
                .map(|file| (*file).to_owned())
                .collect();
            let missing: Vec<String> = missing.iter().map(|file| (*file).to_owned()).collect();
            if !seen.insert(location.clone()) {
                return None;
            }
            // a witness has no import facts to read, so only the language of
            // every file it runs through can clear it of load order.
            let unordered = component
                .iter()
                .all(|file| snapshot.imports_unordered_for(file));
            let cycle = ImportCycle {
                status: ImportCycleStatus::Unavailable,
                cut: Vec::new(),
                exact: false,
                files_without_imports: missing,
                no_load_order: unordered.then_some(NoLoadOrder::Unordered),
            };
            Some(violation(location, cycle))
        })
        .collect()
}

/// A cycle finding over `location` with its detail worded from `cycle`: a
/// violation when an import of it runs at load time, borderline otherwise.
fn violation(location: Vec<String>, cycle: ImportCycle) -> Violation {
    let severity = if cycle.no_load_order.is_some() {
        Severity::Borderline
    } else {
        Severity::Violation
    };
    Violation {
        kind: ViolationKind::Cycle,
        severity,
        detail: cycle.describe(&location, str::to_owned),
        location,
        break_suggestions: None,
        import_cycle: Some(cycle),
        capacity: None,
    }
}

/// The break for the collapsed edge `importer -> imported`: every statement
/// behind it and the references it carries.
fn import_break(
    graph: &ImportGraph<'_>,
    importer: usize,
    imported: usize,
    references: &CarriedReferences,
) -> ImportBreak {
    let statements: BTreeSet<ImportStatement> = graph
        .statements(importer, imported)
        .iter()
        .map(|import| ImportStatement {
            line: import.line,
            specifier: import.specifier.to_string(),
        })
        .collect();
    let carried: Vec<SymbolReference> = references
        .get(&(importer, imported))
        .cloned()
        .unwrap_or_default();
    ImportBreak {
        importer: graph.file(importer).to_owned(),
        imported: graph.file(imported).to_owned(),
        statements: statements.into_iter().collect(),
        weight: edge_weight(carried.len()),
        references: carried,
    }
}

/// The price of removing one import: the references it carries, and at least
/// one for deleting the statement itself.
pub(super) fn edge_weight(references: usize) -> u32 {
    u32::try_from(references.max(1)).unwrap_or(u32::MAX)
}

/// The references each import edge carries.
///
/// A reference is one distinct `(source, target)` symbol pair, by node id,
/// so two symbols that share a display name still count twice. The list is
/// ascending by display name, with the ids breaking a name tie.
type CarriedReferences = BTreeMap<(usize, usize), Vec<SymbolReference>>;

/// One edge's references, each with its source and target node ids so two
/// references with equal names stay distinct.
type IdentifiedReferences = BTreeSet<(SymbolReference, u32, u32)>;

/// The distinct hard symbol edges each import edge carries, counted by node
/// identity and named by display name.
///
/// An import carries a symbol edge when the edge runs from the importer to a
/// declaration of the imported file, or to a declaration the imported file
/// re-exports. Adapters resolve a call through a barrel to the original
/// declaration, so without the second case an import of a barrel would read
/// as carrying nothing and price at the minimum.
fn carried_references(snapshot: &Snapshot, graph: &ImportGraph<'_>) -> CarriedReferences {
    let file_of = node_files(snapshot);
    let names = node_names(snapshot);
    let re_exporters = re_exporting_files(snapshot, &file_of);
    let mut carried: BTreeMap<(usize, usize), IdentifiedReferences> = BTreeMap::new();
    for edge in &snapshot.ir().edges {
        if edge.hardness != Hardness::Hard {
            continue;
        }
        let (Some((_, source_file)), Some((_, target_file))) =
            (file_of.get(&edge.source.0), file_of.get(&edge.target.0))
        else {
            continue;
        };
        let Some(importer) = graph.index_of(source_file) else {
            continue;
        };
        let through = re_exporters.get(&edge.target.0).into_iter().flatten();
        for imported_file in std::iter::once(target_file).chain(through) {
            let Some(imported) = graph.index_of(imported_file) else {
                continue;
            };
            if !graph.has_edge(importer, imported) {
                continue;
            }
            let name = |id: u32| names.get(&id).cloned().unwrap_or_default();
            let reference = SymbolReference {
                source: name(edge.source.0),
                target: name(edge.target.0),
            };
            carried.entry((importer, imported)).or_default().insert((
                reference,
                edge.source.0,
                edge.target.0,
            ));
        }
    }
    carried
        .into_iter()
        .map(|(pair, references)| {
            let listed = references
                .into_iter()
                .map(|(reference, _, _)| reference)
                .collect();
            (pair, listed)
        })
        .collect()
}

/// Maps each declaration to the files that re-export it, following chains of
/// re-exports, so `a -> barrel2 -> barrel1 -> x` lists both barrels for `x`.
fn re_exporting_files<'a>(
    snapshot: &Snapshot,
    file_of: &BTreeMap<u32, (u32, &'a str)>,
) -> BTreeMap<u32, BTreeSet<&'a str>> {
    let mut forward: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for edge in &snapshot.ir().edges {
        if edge.kind == EdgeKind::ReExport {
            forward
                .entry(edge.source.0)
                .or_default()
                .push(edge.target.0);
        }
    }
    let mut re_exporters: BTreeMap<u32, BTreeSet<&'a str>> = BTreeMap::new();
    for &start in forward.keys() {
        let Some(&(_, barrel)) = file_of.get(&start) else {
            continue;
        };
        let mut pending = vec![start];
        let mut seen: BTreeSet<u32> = BTreeSet::new();
        while let Some(current) = pending.pop() {
            if !seen.insert(current) {
                continue;
            }
            match forward.get(&current) {
                Some(targets) => pending.extend(targets.iter().copied()),
                None => {
                    re_exporters.entry(current).or_default().insert(barrel);
                }
            }
        }
    }
    re_exporters
}

/// Maps each node id to the id and path of the file container holding it.
fn node_files(snapshot: &Snapshot) -> BTreeMap<u32, (u32, &str)> {
    let ir = snapshot.ir();
    let files: BTreeMap<u32, &str> = ir
        .containers
        .containers()
        .iter()
        .filter(|container| container.level == ScopeLevel::File)
        .map(|container| (container.id.0, container.name.as_str()))
        .collect();
    ir.nodes
        .iter()
        .filter_map(|node| {
            files
                .get(&node.container.0)
                .map(|file| (node.id.0, (node.container.0, *file)))
        })
        .collect()
}
