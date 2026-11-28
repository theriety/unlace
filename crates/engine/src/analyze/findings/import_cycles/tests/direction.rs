//! Cut direction (ADR-35): an entry's import into its own folder is the last
//! to remove, a child's import back up to an entry the first.

use std::collections::BTreeMap;

use unlace_ir::{Edge, FileFact, Node, Polarity, Snapshot};

use super::super::cut::EXACT_LIMIT;
use super::super::direction::{EdgeTier, EntryFolders, lexicographic_weights};
use super::{analyzed, cycles, first, import, snapshot_with};
use crate::analyze::test_support::{edge, node};
use crate::result::{ImportCycle, Violation};

/// One import `importer -> imported` on `line` carrying `references`
/// distinct symbol references.
type Priced<'a> = (&'a str, &'a str, u32, u32);

/// A snapshot over `files` whose imports each carry their stated references,
/// with `facts` as the adapters' file facts.
fn priced_snapshot(
    files: &[&str],
    imports: &[Priced<'_>],
    facts: Vec<FileFact>,
) -> Result<Snapshot, String> {
    let index = |file: &str| -> Result<u32, String> {
        let position = files
            .iter()
            .position(|known| *known == file)
            .ok_or_else(|| format!("{file} is not a fixture file"))?;
        u32::try_from(position).map_err(|error| error.to_string())
    };
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut next = 0_u32;
    for (position, &(importer, imported, _, references)) in imports.iter().enumerate() {
        let target = next;
        nodes.push(node(
            target,
            &format!("t{position}"),
            index(imported)?,
            Polarity::Production,
        ));
        next += 1;
        for reference in 0..references {
            nodes.push(node(
                next,
                &format!("s{position}_{reference}"),
                index(importer)?,
                Polarity::Production,
            ));
            edges.push(edge(next, target));
            next += 1;
        }
    }
    let stated = imports
        .iter()
        .map(|&(importer, imported, line, _)| import(importer, imported, "./dep", line))
        .collect();
    snapshot_with(files, nodes, edges, stated, files, |ir| {
        ir.file_facts = facts;
    })
}

/// The only cycle finding's import cycle.
fn only_cycle(snapshot: &Snapshot) -> Result<ImportCycle, String> {
    let result = analyzed(snapshot)?;
    let found: Vec<&Violation> = cycles(&result);
    assert_eq!(found.len(), 1, "{found:?}");
    first(&found)?
        .import_cycle
        .clone()
        .ok_or_else(|| "a cycle finding names its import cycle".to_owned())
}

/// The cut as `(importer, imported)` pairs.
fn cut_pairs(cycle: &ImportCycle) -> Vec<(&str, &str)> {
    cycle
        .cut
        .iter()
        .map(|removed| (removed.importer.as_str(), removed.imported.as_str()))
        .collect()
}

// --- tiers ------------------------------------------------------------------

fn tier(facts: &[FileFact], importer: &str, imported: &str) -> EdgeTier {
    EdgeTier::of(importer, imported, &EntryFolders::from_facts(facts))
}

#[test]
fn should_classify_imports_around_a_companion_entry() {
    let facts = [
        FileFact::folder_companion("src/lexer/block.ts", "src/lexer/block"),
        FileFact::folder_companion("src/lexer/container.ts", "src/lexer/container"),
    ];

    assert_eq!(
        tier(&facts, "src/lexer/block.ts", "src/lexer/block/leaf.ts"),
        EdgeTier::Descending
    );
    assert_eq!(
        tier(&facts, "src/lexer/block/leaf.ts", "src/lexer/block.ts"),
        EdgeTier::Ascending
    );
    // widened: a file below the entry's own directory reaches back up.
    assert_eq!(
        tier(&facts, "src/lexer/container/quote.ts", "src/lexer/block.ts"),
        EdgeTier::Ascending
    );
    // a sibling of the entry is not below it.
    assert_eq!(
        tier(&facts, "src/lexer/block.ts", "src/lexer/container.ts"),
        EdgeTier::Lateral
    );
    assert_eq!(
        tier(&facts, "src/lexer/source.ts", "src/lexer/block.ts"),
        EdgeTier::Lateral
    );
    // a prefix of the folder name is not inside it.
    assert_eq!(
        tier(&facts, "src/lexer/block.ts", "src/lexer/blocky/x.ts"),
        EdgeTier::Lateral
    );
}

#[test]
fn should_classify_imports_around_a_mod_rs_entry() {
    let facts = [FileFact::entry_of_folder("src/beta/mod.rs", "src/beta")];

    assert_eq!(
        tier(&facts, "src/beta/mod.rs", "src/beta/x.rs"),
        EdgeTier::Descending
    );
    assert_eq!(
        tier(&facts, "src/beta/x.rs", "src/beta/mod.rs"),
        EdgeTier::Ascending
    );
    assert_eq!(
        tier(&facts, "src/beta/deep/y.rs", "src/beta/mod.rs"),
        EdgeTier::Ascending
    );
    assert_eq!(
        tier(&facts, "src/other.rs", "src/beta/mod.rs"),
        EdgeTier::Lateral
    );
}

#[test]
fn should_classify_every_import_as_lateral_without_entry_facts() {
    assert_eq!(
        tier(
            &[FileFact::entry_file("src/a.ts")],
            "src/a.ts",
            "src/a/b.ts"
        ),
        EdgeTier::Lateral
    );
}

#[test]
fn should_classify_an_entry_importing_itself_as_lateral() {
    // `src/lib.rs` lies inside the `src` folder it is the entry of, so only
    // the self-import guard keeps it from reading as a descending import.
    let facts = [FileFact {
        entry_of: Some("src".into()),
        ..FileFact::entry_file("src/lib.rs")
    }];

    assert_eq!(tier(&facts, "src/lib.rs", "src/lib.rs"), EdgeTier::Lateral);
}

// --- weights ----------------------------------------------------------------

#[test]
fn should_fall_back_to_raw_weights_when_the_encoding_overflows() {
    let half = u64::MAX / 2 + 1;
    let edges: BTreeMap<(usize, usize), (u64, EdgeTier)> = [
        ((0, 1), (half, EdgeTier::Descending)),
        ((1, 0), (half, EdgeTier::Ascending)),
    ]
    .into_iter()
    .collect();

    let weights = lexicographic_weights(&edges);

    assert_eq!(
        weights,
        [((0, 1), half), ((1, 0), half)].into_iter().collect()
    );
}

#[test]
fn should_fall_back_to_raw_weights_at_every_overflowing_step_of_the_encoding() {
    // each case overflows one later step: `low = Σraw + 1`, `E·low`,
    // `E·low + low` (= high), `high + low` for a descending edge, and finally
    // `high + low + raw`.
    let quarter = 1_u64 << 62;
    let cases: [(&str, Vec<(u64, EdgeTier)>); 5] = [
        (
            "low",
            vec![(u64::MAX, EdgeTier::Lateral), (0, EdgeTier::Lateral)],
        ),
        (
            "count times low",
            vec![
                (quarter, EdgeTier::Lateral),
                (quarter - 1, EdgeTier::Lateral),
            ],
        ),
        ("high", vec![((1 << 63) - 1, EdgeTier::Lateral)]),
        ("high plus low", vec![((1 << 63) - 2, EdgeTier::Descending)]),
        (
            "high plus low plus raw",
            vec![(quarter, EdgeTier::Descending)],
        ),
    ];

    for (step, priced) in cases {
        let edges: BTreeMap<(usize, usize), (u64, EdgeTier)> = priced
            .iter()
            .enumerate()
            .map(|(index, &weight)| ((index, index + 1), weight))
            .collect();
        let raw: BTreeMap<(usize, usize), u64> = edges
            .iter()
            .map(|(&edge, &(weight, _))| (edge, weight))
            .collect();

        let weights = lexicographic_weights(&edges);

        assert_eq!(weights, raw, "overflow at {step}");
    }
}

#[test]
fn should_rank_descending_then_non_ascending_then_references() {
    let edges: BTreeMap<(usize, usize), (u64, EdgeTier)> = [
        ((0, 1), (1, EdgeTier::Descending)),
        ((1, 2), (1, EdgeTier::Lateral)),
        ((2, 0), (9, EdgeTier::Ascending)),
    ]
    .into_iter()
    .collect();

    let weights = lexicographic_weights(&edges);
    let weight = |edge| weights.get(&edge).copied().unwrap_or_default();

    assert!(weight((0, 1)) > weight((1, 2)) + weight((2, 0)));
    assert!(weight((1, 2)) > weight((2, 0)));
}

// --- cuts -------------------------------------------------------------------

#[test]
fn should_cut_a_heavier_back_edge_rather_than_a_light_descending_import() -> Result<(), String> {
    let entry = "src/a.ts";
    let child = "src/a/x.ts";
    let snapshot = priced_snapshot(
        &[entry, child],
        &[(entry, child, 1, 1), (child, entry, 1, 3)],
        vec![FileFact::folder_companion(entry, "src/a")],
    )?;

    let cycle = only_cycle(&snapshot)?;

    assert_eq!(cut_pairs(&cycle), vec![(child, entry)]);
    // the reported weight stays the raw reference count.
    assert_eq!(cycle.cut.first().map(|removed| removed.weight), Some(3));
    Ok(())
}

#[test]
fn should_cut_heavier_back_edges_rather_than_a_cheap_lateral_import() -> Result<(), String> {
    let entry = "src/p.ts";
    let sibling = "src/q.ts";
    let child = "src/p/child.ts";
    let snapshot = priced_snapshot(
        &[entry, sibling, child],
        &[
            (entry, sibling, 1, 1),
            (sibling, child, 1, 1),
            (child, entry, 1, 4),
        ],
        vec![FileFact::folder_companion(entry, "src/p")],
    )?;

    let cycle = only_cycle(&snapshot)?;

    assert_eq!(cut_pairs(&cycle), vec![(child, entry)]);
    Ok(())
}

const BLOCK: &str = "src/lexer/block.ts";
const LEAF: &str = "src/lexer/block/leaf.ts";
const MDCX: &str = "src/lexer/block/mdcx.ts";
const TABLE: &str = "src/lexer/block/table.ts";
const CONTAINER: &str = "src/lexer/container.ts";
const QUOTE: &str = "src/lexer/container/quote.ts";

/// The mdc lexer component with the reference counts the companion-folders
/// eval fixture measures: `block.ts` assembles `block/*` and `./container`,
/// whose barrel re-exports `container/quote.ts`, and the children import
/// `../block` back. `extra` adds that many more `block/*` children shaped
/// like `leaf.ts`.
fn mdc_lexer(extra: usize) -> Result<Snapshot, String> {
    let children: Vec<String> = (0..extra)
        .map(|child| format!("src/lexer/block/extra{child:02}.ts"))
        .collect();
    let mut files = vec![BLOCK, LEAF, MDCX, TABLE, CONTAINER, QUOTE];
    files.extend(children.iter().map(String::as_str));
    let mut imports: Vec<Priced<'_>> = vec![
        (BLOCK, LEAF, 3, 3),
        (BLOCK, MDCX, 5, 2),
        (BLOCK, TABLE, 9, 1),
        (BLOCK, CONTAINER, 10, 1),
        (LEAF, BLOCK, 1, 1),
        (MDCX, BLOCK, 3, 5),
        (TABLE, MDCX, 6, 1),
        (CONTAINER, QUOTE, 3, 1),
        (QUOTE, BLOCK, 1, 2),
    ];
    for child in &children {
        imports.push((BLOCK, child.as_str(), 11, 1));
        imports.push((child.as_str(), BLOCK, 1, 1));
    }
    priced_snapshot(
        &files,
        &imports,
        vec![
            FileFact::folder_companion(BLOCK, "src/lexer/block"),
            FileFact::folder_companion(CONTAINER, "src/lexer/container"),
        ],
    )
}

#[test]
fn should_cut_only_the_back_edges_to_block_in_the_mdc_lexer_cycle() -> Result<(), String> {
    let cycle = only_cycle(&mdc_lexer(0)?)?;

    assert!(cycle.exact);
    assert_eq!(
        cut_pairs(&cycle),
        vec![(LEAF, BLOCK), (MDCX, BLOCK), (QUOTE, BLOCK)]
    );
    Ok(())
}

#[test]
fn should_cut_only_back_edges_to_block_in_a_greedy_sized_mdc_lexer_cycle() -> Result<(), String> {
    let extra = EXACT_LIMIT - 6 + 2;
    let cycle = only_cycle(&mdc_lexer(extra)?)?;

    let cut = cut_pairs(&cycle);
    assert!(!cycle.exact);
    assert!(
        cut.iter().all(|&(_, imported)| imported == BLOCK),
        "{cut:?}"
    );
    assert_eq!(cut.len(), 3 + extra, "{cut:?}");
    Ok(())
}
