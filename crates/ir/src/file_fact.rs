//! Per-file facts an adapter states about its own language's conventions.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// What an adapter knows about one source file beyond its declarations.
///
/// Test pairs, entry files and folder companions are language conventions, so
/// the adapter that owns the language states them and the engine only reads
/// them. A file with no pair, no entry role and no companion folder has no
/// fact. An adapter states a pair only when it
/// can prove it (a module tree, a mirrored path whose source exists); when
/// unsure it records nothing, so a missing fact means "no pair" and "not an
/// entry".
///
/// Paths are repo-relative with `/` separators, the same spelling the file
/// containers use.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FileFact {
    /// The file the fact describes.
    pub file: SmolStr,
    /// The production source file this test file mirrors, when it is a test
    /// whose source the adapter found. The pair is one-directional: `file` is
    /// the test, `test_of` the source. An inline test module shares its
    /// source's file, so it records nothing here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_of: Option<SmolStr>,
    /// Whether the file is a program entry point (a crate root, a binary, a
    /// package entry). An entry file never moves.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub entry: bool,
    /// The name of the program's start function declared at this entry file's
    /// root (Rust `fn main` of a binary, example or build script). The symbol
    /// pass never moves it out of the file; other symbols in the file may
    /// still move. Only an adapter that can name a start symbol sets it (the
    /// TypeScript and Python adapters have none), so absent means "none".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<SmolStr>,
    /// The repo-relative directory `x` this file `x.ts` is the entry of: the
    /// file sits beside `x/` and stands in for `x/index.ts` (decisions D1/E1).
    /// The adapter records nothing, so the file stays an ordinary sibling, when
    /// `x/index.ts` also exists, when both `x.ts` and `x.tsx` sit beside `x/`,
    /// or when `x/` holds no analysed module. A test paired with such a file
    /// gets the mirrored companion of its own (`spec/lexer/block.spec.ts`
    /// gets `spec/lexer/block`), so the pair stays mirrored. Added to schema
    /// 4; absent means "no companion folder".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_companion: Option<SmolStr>,
    /// The repo-relative directory whose module entry this file is, when the
    /// language makes it one without a companion: Rust `lib.rs`, `main.rs`
    /// and `mod.rs` for their own directory, `foo.rs` for the `foo/` holding
    /// its child modules, the single TypeScript `x/index.*` for `x`, and a
    /// Python `pkg/__init__.py` for `pkg`. The engine reads `entry_of`, falling back to
    /// `folder_companion`, only to price an import-cycle cut, where an import
    /// from an entry into its own folder is the last one to remove (ADR-35).
    /// It is not a placement fact: clustering, capacity, depth and the move
    /// pins never read it. Added to schema 4; absent means "no entry folder
    /// beyond the companion".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_of: Option<SmolStr>,
}

impl FileFact {
    /// A fact stating that `file` is the test of `source`.
    #[must_use]
    pub fn test_pair(file: impl Into<SmolStr>, source: impl Into<SmolStr>) -> Self {
        Self {
            file: file.into(),
            test_of: Some(source.into()),
            entry: false,
            start: None,
            folder_companion: None,
            entry_of: None,
        }
    }

    /// A fact stating that `file` is a program entry point.
    #[must_use]
    pub fn entry_file(file: impl Into<SmolStr>) -> Self {
        Self {
            file: file.into(),
            test_of: None,
            entry: true,
            start: None,
            folder_companion: None,
            entry_of: None,
        }
    }

    /// A fact stating that `file` is a program entry point whose start function
    /// is `start`.
    #[must_use]
    pub fn entry_with_start(file: impl Into<SmolStr>, start: impl Into<SmolStr>) -> Self {
        Self {
            file: file.into(),
            test_of: None,
            entry: true,
            start: Some(start.into()),
            folder_companion: None,
            entry_of: None,
        }
    }

    /// A fact stating that `file` is the entry of the companion directory
    /// `folder` beside it.
    #[must_use]
    pub fn folder_companion(file: impl Into<SmolStr>, folder: impl Into<SmolStr>) -> Self {
        Self {
            file: file.into(),
            test_of: None,
            entry: false,
            start: None,
            folder_companion: Some(folder.into()),
            entry_of: None,
        }
    }

    /// A fact stating that `file` is the module entry of `folder` (see
    /// [`FileFact::entry_of`]).
    #[must_use]
    pub fn entry_of_folder(file: impl Into<SmolStr>, folder: impl Into<SmolStr>) -> Self {
        Self {
            file: file.into(),
            test_of: None,
            entry: false,
            start: None,
            folder_companion: None,
            entry_of: Some(folder.into()),
        }
    }
}

/// Pairs each candidate test path that exists with the one source that names it.
///
/// `candidates` lists, per production source, the test paths its language
/// convention would give it. A test path that exists in `present` becomes a
/// [`FileFact::test_pair`] only when exactly one source names it: a test two
/// sources claim is ambiguous, so it records no pair (when unsure: no pair).
/// The result is sorted by test path.
#[must_use]
pub fn unique_test_pairs(
    candidates: impl IntoIterator<Item = (SmolStr, Vec<String>)>,
    present: &BTreeSet<&str>,
) -> Vec<FileFact> {
    let mut claims: BTreeMap<String, Vec<SmolStr>> = BTreeMap::new();
    for (source, tests) in candidates {
        for test in tests {
            if present.contains(test.as_str()) {
                let sources = claims.entry(test).or_default();
                if !sources.contains(&source) {
                    sources.push(source.clone());
                }
            }
        }
    }
    claims
        .into_iter()
        .filter_map(|(test, sources)| match sources.as_slice() {
            [only] => Some(FileFact::test_pair(test, only.clone())),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_pair_a_present_test_with_its_only_source() {
        let present: BTreeSet<&str> = ["a.ts", "a.spec.ts"].into_iter().collect();
        let pairs = unique_test_pairs(
            [(
                SmolStr::new("a.ts"),
                vec!["a.spec.ts".to_owned(), "gone.ts".to_owned()],
            )],
            &present,
        );
        assert_eq!(pairs, vec![FileFact::test_pair("a.spec.ts", "a.ts")]);
    }

    #[test]
    fn should_record_no_pair_for_a_test_two_sources_claim() {
        let present: BTreeSet<&str> = ["t.py"].into_iter().collect();
        let pairs = unique_test_pairs(
            [
                (SmolStr::new("src/x.py"), vec!["t.py".to_owned()]),
                (SmolStr::new("source/x.py"), vec!["t.py".to_owned()]),
            ],
            &present,
        );
        assert_eq!(pairs, Vec::<FileFact>::new());
    }

    #[test]
    fn should_pair_a_test_its_only_source_names_twice() {
        let present: BTreeSet<&str> = ["a_test.py"].into_iter().collect();
        let pairs = unique_test_pairs(
            [
                (SmolStr::new("a.py"), vec!["a_test.py".to_owned()]),
                (SmolStr::new("a.py"), vec!["a_test.py".to_owned()]),
            ],
            &present,
        );
        assert_eq!(pairs, vec![FileFact::test_pair("a_test.py", "a.py")]);
    }

    #[test]
    fn should_state_an_entry_file_with_no_pair_and_no_start() -> Result<(), String> {
        let fact = FileFact::entry_file("src/lib.rs");

        let json = serde_json::to_string(&fact).or(Err("a fact must serialize"))?;

        assert_eq!(json, r#"{"file":"src/lib.rs","entry":true}"#);
        Ok(())
    }

    #[test]
    fn should_state_an_entry_file_with_its_start_function() -> Result<(), String> {
        let fact = FileFact::entry_with_start("src/main.rs", "main");

        let json = serde_json::to_string(&fact).or(Err("a fact must serialize"))?;
        let restored: FileFact = serde_json::from_str(&json).or(Err("a fact must deserialize"))?;

        assert_eq!(
            json,
            r#"{"file":"src/main.rs","entry":true,"start":"main"}"#
        );
        assert_eq!(restored, fact);
        Ok(())
    }

    #[test]
    fn should_round_trip_an_entry_folder_beside_an_entry_role() -> Result<(), String> {
        let fact = FileFact {
            entry_of: Some(SmolStr::new("src")),
            ..FileFact::entry_file("src/lib.rs")
        };

        let json = serde_json::to_string(&fact).or(Err("a fact must serialize"))?;
        let restored: FileFact = serde_json::from_str(&json).or(Err("a fact must deserialize"))?;

        assert_eq!(
            json,
            r#"{"file":"src/lib.rs","entry":true,"entry_of":"src"}"#
        );
        assert_eq!(restored, fact);
        Ok(())
    }
}
