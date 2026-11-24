//! Shared test-only fixtures for the assertion tests.

use std::collections::BTreeSet;

use unlace_engine::result::{ContainerNode, Level, SymbolPlacement};

use crate::harness::inputs::{EvalInputs, FaceInputs, discover_packages};
use crate::metrics::{members, structural_placement};
use crate::target::FaceMode;

pub(in crate::harness) fn file(name: &str) -> ContainerNode {
    ContainerNode {
        name: name.to_owned(),
        level: Level::File,
        children: None,
        symbols: None,
        production_sloc: None,
    }
}

/// A file node declaring `symbols` as living in it.
pub(in crate::harness) fn file_with_symbols(name: &str, symbols: &[&str]) -> ContainerNode {
    ContainerNode {
        symbols: Some(
            symbols
                .iter()
                .map(|symbol| SymbolPlacement {
                    name: (*symbol).to_owned(),
                    display: None,
                    visibility: Level::File,
                })
                .collect(),
        ),
        ..file(name)
    }
}

pub(in crate::harness) fn node(
    level: Level,
    name: &str,
    children: Vec<ContainerNode>,
) -> ContainerNode {
    ContainerNode {
        name: name.to_owned(),
        level,
        children: Some(children),
        symbols: None,
        production_sloc: None,
    }
}

/// A candidate tree where `billing` dissolved: invoice and pricing welded
/// into a domain named `helpers`, telemetry intact.
pub(in crate::harness) fn torn_candidate() -> ContainerNode {
    node(
        Level::PackageGroup,
        "root",
        vec![node(
            Level::Package,
            "app",
            vec![node(
                Level::Domain,
                "helpers",
                vec![
                    file("billing/invoice.py"),
                    file("billing/pricing.py"),
                    file("pipeline.py"),
                ],
            )],
        )],
    )
}

/// A candidate tree that keeps real directories as folders.
pub(in crate::harness) fn laminar_candidate() -> ContainerNode {
    node(
        Level::PackageGroup,
        "root",
        vec![node(
            Level::Package,
            "app",
            vec![
                node(
                    Level::Folder,
                    "billing",
                    vec![file("billing/invoice.py"), file("billing/pricing.py")],
                ),
                node(Level::Folder, "telemetry", vec![file("telemetry/sink.py")]),
                file("pipeline.py"),
            ],
        )],
    )
}

pub(in crate::harness) fn inputs_with(candidate: &ContainerNode) -> EvalInputs<'_> {
    inputs_with_faces(&[(FaceMode::Anchored, candidate, None)])
}

/// Inputs over the laminar current layout with one evaluated face per
/// `(face, candidate tree, hard capacity findings left behind)` entry.
pub(in crate::harness) fn inputs_with_faces<'a>(
    faces: &[(FaceMode, &'a ContainerNode, Option<u32>)],
) -> EvalInputs<'a> {
    let current = laminar_candidate();
    let census: BTreeSet<String> = members(&current).into_iter().collect();
    EvalInputs {
        current_placement: structural_placement(&current),
        packages: discover_packages(&current, &census),
        census,
        faces: faces
            .iter()
            .map(|(face, tree, capacity_remaining)| {
                (
                    *face,
                    FaceInputs {
                        tree,
                        capacity_remaining: *capacity_remaining,
                    },
                )
            })
            .collect(),
    }
}
