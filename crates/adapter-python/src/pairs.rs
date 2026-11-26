//! Test pairs and entry files for Python.
//!
//! A source `src/{dir}/{stem}.py` is paired with `test_{stem}.py` or
//! `{stem}_test.py`, either beside it or under `{test,tests}/{dir}/`, but only
//! when that test file is present and no other source claims it (`source`
//! stands in for `src`). `conftest.py` is test support, not a test of anything,
//! so it is neither paired nor an entry.
//!
//! Entry files are `__main__.py` (what `python -m pkg` runs) and a top-level
//! `setup.py`. Everything else is not an entry: when unsure, it is not one.
//!
//! A package initializer `pkg/__init__.py` is the module entry of its folder,
//! stated as `entry_of = "pkg"`, which only the import-cycle cut reads. A root
//! `__init__.py` has no folder above it, so it states none.

use std::collections::{BTreeMap, BTreeSet};

use smol_str::SmolStr;
use unlace_ir::{FileFact, unique_test_pairs};

use crate::parse::ParsedModule;

const SOURCE_ROOTS: [&str; 2] = ["src", "source"];
const TEST_ROOTS: [&str; 2] = ["test", "tests"];

/// The pair and entry facts for `modules`, sorted by file.
pub(super) fn file_facts(modules: &[ParsedModule]) -> Vec<FileFact> {
    let present: BTreeSet<&str> = modules.iter().map(|module| module.path.as_str()).collect();
    let candidates = present.iter().filter_map(|path| {
        let (root, dir, stem) = split_source(path)?;
        Some((SmolStr::new(path), test_paths(root, dir, stem)))
    });
    let mut facts: BTreeMap<SmolStr, FileFact> = unique_test_pairs(candidates, &present)
        .into_iter()
        .map(|fact| (fact.file.clone(), fact))
        .collect();
    for path in present.iter().filter(|path| is_entry(path)) {
        facts
            .entry(SmolStr::new(path))
            .or_insert_with(|| FileFact::entry_file(*path))
            .entry = true;
    }
    for (path, folder) in present
        .iter()
        .filter_map(|path| package_folder(path).map(|folder| (*path, folder)))
    {
        facts
            .entry(SmolStr::new(path))
            .or_insert_with(|| FileFact::entry_of_folder(path, folder))
            .entry_of = Some(SmolStr::new(folder));
    }
    facts.into_values().collect()
}

/// The folder `pkg` whose initializer `pkg/__init__.py` is, or `None` for any
/// other file and for a root `__init__.py`.
fn package_folder(path: &str) -> Option<&str> {
    path.strip_suffix("/__init__.py")
}

fn is_entry(path: &str) -> bool {
    path == "setup.py" || path == "__main__.py" || path.ends_with("/__main__.py")
}

/// Splits `{root}/{dir}/{stem}.py` into root, directory and stem; `None` when
/// the path is not a production-looking source there.
fn split_source(path: &str) -> Option<(&str, &str, &str)> {
    let (root, rest) = path.split_once('/')?;
    if !SOURCE_ROOTS.contains(&root) {
        return None;
    }
    let (dir, file) = rest.rsplit_once('/').unwrap_or(("", rest));
    let stem = file.strip_suffix(".py")?;
    let is_test_name = stem.starts_with("test_") || stem.ends_with("_test");
    (!stem.is_empty() && !is_test_name && stem != "conftest").then_some((root, dir, stem))
}

/// Every path a test of this source could have, in convention order.
fn test_paths(root: &str, dir: &str, stem: &str) -> Vec<String> {
    let join = |base: &str, file: String| {
        if dir.is_empty() {
            format!("{base}/{file}")
        } else {
            format!("{base}/{dir}/{file}")
        }
    };
    let files = || [format!("test_{stem}.py"), format!("{stem}_test.py")];
    let mut paths = Vec::new();
    for test_root in TEST_ROOTS {
        paths.extend(files().map(|file| join(test_root, file)));
    }
    paths.extend(files().map(|file| join(root, file)));
    paths
}
