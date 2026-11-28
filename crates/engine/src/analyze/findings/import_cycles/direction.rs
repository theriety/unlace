//! Which way an import runs through the module-entry folders, and the cut
//! price that makes a child's import of its parent entry the first to go
//! (ADR-35).
//!
//! An entry file stands for its folder: `lib.rs`, `mod.rs` and `foo.rs` with
//! `foo/` in Rust, a companion `x.ts` beside `x/`. Its import into its own
//! folder is how the folder is assembled, so removing it tears the module
//! apart; a child's import back up to the entry is the dependency to undo.
//! The cut therefore prices a component's edges lexicographically:
//!
//! 1. the number of [`EdgeTier::Descending`] imports removed,
//! 2. the number of imports removed that are not [`EdgeTier::Ascending`],
//! 3. the symbol references they carry,
//! 4. the number of imports removed (the cut's own tie-break).
//!
//! So a cut of back edges beats one that removes a cheaper sideways import:
//! the child-to-parent imports are the ones to undo, whatever they carry.
//! The first three are folded into one additive integer per edge
//! ([`lexicographic_weights`]), so the exact solve is exactly lexicographic and
//! the greedy one approximates it without either knowing the tiers.

use std::collections::BTreeMap;

use unlace_ir::FileFact;

/// The folder each entry file stands for, read from the adapters' facts:
/// [`FileFact::entry_of`], falling back to [`FileFact::folder_companion`].
///
/// Only the cycle cut reads this; placement keeps reading the companions.
pub(super) struct EntryFolders<'a> {
    /// Repo-relative entry file → the repo-relative folder it is the entry of.
    by_file: BTreeMap<&'a str, &'a str>,
}

impl<'a> EntryFolders<'a> {
    /// Collects every fact that names a folder. A file stated twice keeps its
    /// last folder in fact order, which the snapshot sorts.
    pub(super) fn from_facts(facts: &'a [FileFact]) -> Self {
        let by_file = facts
            .iter()
            .filter_map(|fact| {
                let folder = fact.entry_of.as_ref().or(fact.folder_companion.as_ref())?;
                Some((fact.file.as_str(), folder.as_str()))
            })
            .collect();
        Self { by_file }
    }

    /// The folder `file` is the entry of, when it is an entry.
    fn folder_of(&self, file: &str) -> Option<&'a str> {
        self.by_file.get(file).copied()
    }
}

/// How an import `importer -> imported` runs relative to the entry folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EdgeTier {
    /// The importer is the entry of a folder that holds the imported file: the
    /// import assembles the module and is the last one to remove.
    Descending,
    /// The imported file is an entry, and the importer lies inside the folder
    /// it is the entry of, or in a subdirectory of the entry's own directory
    /// (`lexer/container/quote.ts -> lexer/block.ts`): a child reaching back
    /// up, the first one to remove.
    Ascending,
    /// Any other import.
    Lateral,
}

impl EdgeTier {
    /// Classifies `importer -> imported` by physical paths.
    pub(super) fn of(importer: &str, imported: &str, entries: &EntryFolders<'_>) -> Self {
        if importer == imported {
            return Self::Lateral;
        }
        if entries
            .folder_of(importer)
            .is_some_and(|folder| is_inside(imported, folder))
        {
            return Self::Descending;
        }
        let ascending = entries.folder_of(imported).is_some_and(|folder| {
            is_inside(importer, folder) || is_strictly_below(importer, parent_directory(imported))
        });
        if ascending {
            Self::Ascending
        } else {
            Self::Lateral
        }
    }
}

/// Folds each edge's tier into its raw weight so the sum over a cut orders
/// cuts lexicographically by (descending count, non-ascending count, raw
/// weight).
///
/// With `E` edges, `B1 = Σraw + 1` and `B2 = E·B1 + Σraw + 1`, an edge weighs
/// `descending·B2 + non_ascending·B1 + raw`: the raw weight of any cut stays
/// below `B1`, and the rest of any cut stays below `B2`. When the encoding
/// would overflow, the raw weights are returned unchanged.
pub(super) fn lexicographic_weights(
    edges: &BTreeMap<(usize, usize), (u64, EdgeTier)>,
) -> BTreeMap<(usize, usize), u64> {
    encoded(edges).unwrap_or_else(|| edges.iter().map(|(&edge, &(raw, _))| (edge, raw)).collect())
}

/// The encoded weights, or `None` on overflow.
fn encoded(
    edges: &BTreeMap<(usize, usize), (u64, EdgeTier)>,
) -> Option<BTreeMap<(usize, usize), u64>> {
    // lossless: every supported target is at most 64-bit.
    let count = edges.len() as u64;
    let raw_total = edges
        .values()
        .try_fold(0_u64, |total, &(raw, _)| total.checked_add(raw))?;
    let low = raw_total.checked_add(1)?;
    let high = count.checked_mul(low)?.checked_add(low)?;
    edges
        .iter()
        .map(|(&edge, &(raw, tier))| {
            let descending = if tier == EdgeTier::Descending {
                high
            } else {
                0
            };
            let non_ascending = if tier == EdgeTier::Ascending { 0 } else { low };
            let weight = descending.checked_add(non_ascending)?.checked_add(raw)?;
            Some((edge, weight))
        })
        .collect()
}

/// Whether `path` lies inside `folder` at any depth; every path lies inside
/// the root folder `""`.
fn is_inside(path: &str, folder: &str) -> bool {
    folder.is_empty()
        || path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Whether `path` lies in a proper subdirectory of `directory`.
fn is_strictly_below(path: &str, directory: &str) -> bool {
    parent_directory(path) != directory && is_inside(path, directory)
}

/// The physical parent directory of `path`, empty for a root-level file.
fn parent_directory(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(directory, _)| directory)
}
