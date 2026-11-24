//! Companion-affinity emission for signature companions.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use smol_str::SmolStr;
use unlace_ir::{Affinity, AffinityKind, Node, NodeId, NodeKind};

use crate::parse::ParsedModule;

use super::super::ExportTable;
use super::super::resolution::{Resolver, resolve_imports};
use super::re_export::ReExportLink;

pub(in crate::bind) fn emit_companion_affinities(
    modules: &[ParsedModule],
    resolver: &Resolver,
    exports: &ExportTable,
    local: &HashMap<SmolStr, HashMap<SmolStr, NodeId>>,
    nodes: &[Node],
    re_export_links: &[ReExportLink],
) -> Vec<Affinity> {
    let kinds: HashMap<NodeId, NodeKind> = nodes.iter().map(|node| (node.id, node.kind)).collect();
    let mut targets_by_source: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();
    for link in re_export_links {
        if link.namespace {
            continue;
        }
        targets_by_source
            .entry(link.source)
            .or_default()
            .insert(link.target);
    }
    let re_export_targets: HashMap<NodeId, NodeId> = targets_by_source
        .into_iter()
        .filter(|(_, targets)| targets.len() == 1)
        .flat_map(|(source, targets)| targets.into_iter().map(move |target| (source, target)))
        .collect();
    let mut owners_by_companion: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for module in modules {
        let module_local = local.get(&module.path);
        let imported = resolve_imports(module, resolver, exports);
        // every declaration was assigned a node in its module's table
        let owned = module_local.into_iter().flat_map(|table| {
            module.declarations.iter().filter_map(|declaration| {
                table
                    .get(&declaration.name)
                    .map(|&owner| (declaration, owner))
            })
        });
        for (declaration, owner) in owned {
            for name in &declaration.signature_companions {
                let companion = imported
                    .get(name)
                    .map(|(target, _)| *target)
                    .or_else(|| module_local.and_then(|table| table.get(name)).copied());
                let companion = companion.and_then(|companion| {
                    originating_re_export_target(companion, &re_export_targets)
                });
                if let Some(companion) = companion
                    && kinds.get(&companion) == Some(&NodeKind::Type)
                {
                    owners_by_companion
                        .entry(companion)
                        .or_default()
                        .push(owner);
                }
            }
        }
    }
    owners_by_companion
        .into_iter()
        .filter_map(|(companion, owners)| {
            let [owner] = owners.as_slice() else {
                return None;
            };
            Some(Affinity {
                owner: *owner,
                companion,
                kind: AffinityKind::CompanionOwner,
            })
        })
        .collect()
}

/// Follows affinity identity through barrel bindings to the declaration that
/// originated the exported name.
///
/// This lookup is intentionally affinity-only: ordinary imports continue to
/// resolve to barrel nodes so dependency and re-export edge semantics remain
/// unchanged. A malformed link cycle has no authoritative origin, so it emits
/// no affinity rather than choosing an arbitrary barrel node.
fn originating_re_export_target(
    start: NodeId,
    re_export_targets: &HashMap<NodeId, NodeId>,
) -> Option<NodeId> {
    // an acyclic chain takes each link at most once, so a longer one is a cycle
    let mut chain = std::iter::successors(Some(start), |node| re_export_targets.get(node).copied());
    let origin = chain.by_ref().take(re_export_targets.len() + 1).last();
    origin.filter(|_| chain.next().is_none())
}
