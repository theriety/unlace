//! Python file-import facts (decision E3): the adapter states which analysed
//! file each load-time import statement loads, and says when it listed them all.

use std::fs;
use std::path::Path;

use smol_str::SmolStr;
use unlace_adapter_python::PythonAdapter;
use unlace_ir::{Adapter, FileImport, ImportKind, IrFragment, LoadOrder, ScopeLevel, SourceFile};

/// Parses and binds `(path, contents)` pairs into one fragment.
fn fragment(files: &[(&str, &str)]) -> IrFragment {
    let adapter = PythonAdapter::default();
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|(path, contents)| SourceFile {
            path: SmolStr::new(*path),
            contents: (*contents).to_owned(),
        })
        .collect();
    let trees = adapter.parse(&sources).unwrap_or_default();
    adapter.bind(trees).unwrap_or_default()
}

/// The facts of `importer` as `(imported, specifier, line)` triples.
fn facts_of(fragment: &IrFragment, importer: &str) -> Vec<(String, String, u32)> {
    fragment
        .imports
        .iter()
        .filter(|import| import.importer == importer)
        .map(|import| {
            (
                import.imported.to_string(),
                import.specifier.to_string(),
                import.line,
            )
        })
        .collect()
}

/// One expected fact.
fn expect(imported: &str, specifier: &str, line: u32) -> (String, String, u32) {
    (imported.to_owned(), specifier.to_owned(), line)
}

const BARREL: (&str, &str) = ("pkg/__init__.py", "SETTING = 1\n");

#[test]
fn should_emit_a_value_import_for_every_module_form_with_its_line_and_specifier() {
    let app = "import pkg.b\nfrom pkg.b import thing\nfrom . import c\nfrom .d import helper\nfrom .sub import deep\nfrom pkg import b\nfrom pkg import SETTING\nimport pkg.b as alias\n";
    let fragment = fragment(&[
        BARREL,
        ("pkg/app.py", app),
        ("pkg/b.py", "thing = 1\n"),
        ("pkg/c.py", "value = 1\n"),
        ("pkg/d.py", "def helper():\n    pass\n"),
        ("pkg/sub/__init__.py", "deep = 1\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "pkg/app.py"),
        vec![
            expect("pkg/__init__.py", "pkg", 7),
            expect("pkg/b.py", "pkg", 6),
            expect("pkg/b.py", "pkg.b", 1),
            expect("pkg/b.py", "pkg.b", 2),
            expect("pkg/b.py", "pkg.b", 8),
            expect("pkg/c.py", ".", 3),
            expect("pkg/d.py", ".d", 4),
            expect("pkg/sub/__init__.py", ".sub", 5),
        ]
    );
    assert!(
        fragment
            .imports
            .iter()
            .all(|import| import.kind == ImportKind::Value)
    );
    assert!(fragment.states_imports);
}

#[test]
fn should_resolve_a_star_import_and_a_parent_relative_import() {
    let fragment = fragment(&[
        BARREL,
        (
            "pkg/sub/app.py",
            "from ..util import *\nfrom .. import helper\n",
        ),
        ("pkg/util.py", "X = 1\n"),
        ("pkg/helper.py", "Y = 1\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "pkg/sub/app.py"),
        vec![
            expect("pkg/helper.py", "..", 2),
            expect("pkg/util.py", "..util", 1)
        ]
    );
    assert!(fragment.states_imports);
}

#[test]
fn should_leave_out_a_type_checking_only_import() {
    let source = "from typing import TYPE_CHECKING\nimport typing\nfrom .real import R\nif TYPE_CHECKING:\n    from .only_types import T\n    import pkg.also_types\nelse:\n    from .fallback import F\nif typing.TYPE_CHECKING:\n    from .qualified import Q\nif not TYPE_CHECKING:\n    from .runtime import RT\nelse:\n    from .checker import C\n";
    let fragment = fragment(&[
        BARREL,
        ("pkg/app.py", source),
        ("pkg/real.py", "R = 1\n"),
        ("pkg/only_types.py", "T = 1\n"),
        ("pkg/also_types.py", "A = 1\n"),
        ("pkg/fallback.py", "F = 1\n"),
        ("pkg/qualified.py", "Q = 1\n"),
        ("pkg/runtime.py", "RT = 1\n"),
        ("pkg/checker.py", "C = 1\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "pkg/app.py"),
        vec![
            expect("pkg/fallback.py", ".fallback", 8),
            expect("pkg/real.py", ".real", 3),
            expect("pkg/runtime.py", ".runtime", 12),
        ]
    );
    assert!(fragment.states_imports);
}

#[test]
fn should_leave_out_a_lazy_import_inside_a_function_body() {
    let source = "from .top import T\n\n\ndef load():\n    from .lazy import L\n    import pkg.lazier\n    return L\n\n\nclass Holder:\n    def method(self):\n        from .lazy import L\n";
    let fragment = fragment(&[
        BARREL,
        ("pkg/app.py", source),
        ("pkg/top.py", "T = 1\n"),
        ("pkg/lazy.py", "L = 1\n"),
        ("pkg/lazier.py", "M = 1\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "pkg/app.py"),
        vec![expect("pkg/top.py", ".top", 1)]
    );
    assert!(fragment.states_imports);
}

#[test]
fn should_keep_imports_in_try_with_and_loop_blocks_that_run_at_load_time() {
    let source = "try:\n    from ._speedups import fast\nexcept ImportError:\n    from ._pure import fast\nwith guard:\n    from ._with import w\n";
    let fragment = fragment(&[
        BARREL,
        ("pkg/app.py", source),
        ("pkg/_speedups.py", "fast = 1\n"),
        ("pkg/_pure.py", "fast = 1\n"),
        ("pkg/_with.py", "w = 1\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "pkg/app.py"),
        vec![
            expect("pkg/_pure.py", "._pure", 4),
            expect("pkg/_speedups.py", "._speedups", 2),
            expect("pkg/_with.py", "._with", 6),
        ]
    );
}

#[test]
fn should_emit_nothing_for_an_external_package_and_still_state_imports() {
    let fragment = fragment(&[
        BARREL,
        (
            "pkg/app.py",
            "import os\nimport numpy.linalg\nfrom requests import get\nfrom .b import B\n",
        ),
        ("pkg/b.py", "B = 1\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "pkg/app.py"),
        vec![expect("pkg/b.py", ".b", 4)]
    );
    assert!(fragment.states_imports);
}

#[test]
fn should_resolve_a_submodule_of_a_namespace_package_but_not_the_package_itself() {
    let fragment = fragment(&[
        ("app.py", "import ns\nfrom ns import mod\nimport ns.mod\n"),
        ("ns/mod.py", "M = 1\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "app.py"),
        vec![
            expect("ns/mod.py", "ns", 2),
            expect("ns/mod.py", "ns.mod", 3)
        ]
    );
    assert!(fragment.states_imports);
}

#[test]
fn should_list_only_the_importer_as_a_gap_when_an_import_inside_the_analysed_set_resolves_to_no_file()
 {
    for source in [
        "from .ghost import x\n",
        "import pkg.ghost\n",
        "from pkg.ghost import x\n",
        "from ... import x\n",
    ] {
        let fragment = fragment(&[BARREL, ("pkg/app.py", source), ("pkg/b.py", "B = 1\n")]);

        // the other files still state their imports; only the importer is a gap.
        assert!(fragment.states_imports, "{source}");
        assert_eq!(
            fragment.files_without_imports,
            vec![SmolStr::new("pkg/app.py")],
            "{source}"
        );
    }
}

#[test]
fn should_resolve_src_layout_imports_by_their_installed_names_and_close_the_cycle() {
    let fragment = fragment(&[
        ("src/pkg/__init__.py", ""),
        ("src/pkg/a.py", "from pkg.b import b\nimport pkg\n"),
        ("src/pkg/b.py", "import pkg.a\nb = 1\n"),
        ("tests/test_a.py", "from pkg import a\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "src/pkg/a.py"),
        vec![
            expect("src/pkg/__init__.py", "pkg", 2),
            expect("src/pkg/b.py", "pkg.b", 1),
        ]
    );
    assert_eq!(
        facts_of(&fragment, "src/pkg/b.py"),
        vec![expect("src/pkg/a.py", "pkg.a", 1)]
    );
    assert_eq!(
        facts_of(&fragment, "tests/test_a.py"),
        vec![expect("src/pkg/a.py", "pkg", 1)]
    );
    assert!(fragment.files_without_imports.is_empty());
}

#[test]
fn should_list_a_src_layout_importer_as_a_gap_when_its_import_names_a_missing_module() {
    let fragment = fragment(&[
        ("src/pkg/__init__.py", ""),
        ("src/pkg/a.py", "from pkg.ghost import g\n"),
        ("src/pkg/b.py", "b = 1\n"),
    ]);

    assert_eq!(
        fragment.files_without_imports,
        vec![SmolStr::new("src/pkg/a.py")]
    );
}

#[test]
fn should_list_the_importer_as_a_gap_when_two_source_roots_claim_the_same_name() {
    // `pkg` is both `libs/one/src/pkg` and `libs/two/src/pkg`: an import of it
    // is first-party but names no single file, so it hides no cycle.
    let fragment = fragment(&[
        ("libs/one/src/pkg/__init__.py", ""),
        ("libs/one/src/pkg/a.py", "from pkg.b import b\n"),
        ("libs/one/src/pkg/b.py", "b = 1\n"),
        ("libs/two/src/pkg/__init__.py", ""),
        ("libs/two/src/pkg/b.py", "b = 2\n"),
    ]);

    assert!(facts_of(&fragment, "libs/one/src/pkg/a.py").is_empty());
    assert_eq!(
        fragment.files_without_imports,
        vec![SmolStr::new("libs/one/src/pkg/a.py")]
    );
}

#[test]
fn should_resolve_a_package_under_a_non_src_root_by_its_import_name_and_close_the_cycle() {
    // M2: `backend/` is not a package and holds the top-level package `app`,
    // so it is a source root as `src/` is.
    let fragment = fragment(&[
        ("backend/app/__init__.py", ""),
        ("backend/app/a.py", "from app import b\n"),
        ("backend/app/b.py", "import app.a\nimport settings\n"),
        ("backend/settings.py", "from app import b\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "backend/app/a.py"),
        vec![expect("backend/app/b.py", "app", 1)]
    );
    assert_eq!(
        facts_of(&fragment, "backend/app/b.py"),
        vec![
            expect("backend/app/a.py", "app.a", 1),
            expect("backend/settings.py", "settings", 2),
        ]
    );
    assert_eq!(
        facts_of(&fragment, "backend/settings.py"),
        vec![expect("backend/app/b.py", "app", 1)]
    );
    assert!(fragment.files_without_imports.is_empty());
}

#[test]
fn should_list_the_importer_as_a_gap_when_two_non_src_roots_hold_the_same_package() {
    let fragment = fragment(&[
        ("backend/app/__init__.py", ""),
        ("backend/app/a.py", "from app import b\n"),
        ("backend/app/b.py", "b = 1\n"),
        ("worker/app/__init__.py", ""),
        ("worker/app/b.py", "b = 2\n"),
    ]);

    assert!(facts_of(&fragment, "backend/app/a.py").is_empty());
    assert_eq!(
        fragment.files_without_imports,
        vec![SmolStr::new("backend/app/a.py")]
    );
}

#[test]
fn should_alias_no_package_nested_below_a_package() {
    // `pkg/ns/` sits inside the package `pkg`, so it is no source root:
    // `inner` stays an outside name.
    let fragment = fragment(&[
        ("pkg/__init__.py", ""),
        ("pkg/ns/inner/__init__.py", ""),
        ("pkg/ns/inner/x.py", "x = 1\n"),
        ("app.py", "import inner.x\nimport pkg.ns.inner.x\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "app.py"),
        vec![expect("pkg/ns/inner/x.py", "pkg.ns.inner.x", 2)]
    );
    assert!(fragment.files_without_imports.is_empty());
}

#[test]
fn should_keep_the_path_name_when_a_src_alias_spells_a_root_module() {
    // `src/` holds an `__init__.py`, so it is a package, not a source root:
    // `src/pkg/b.py` is `src.pkg.b` only, and `pkg.b` is the root `pkg/b.py`.
    let fragment = fragment(&[
        ("src/__init__.py", ""),
        ("src/pkg/__init__.py", ""),
        ("src/pkg/b.py", "b = 1\n"),
        ("pkg/__init__.py", ""),
        ("pkg/b.py", "b = 2\n"),
        (
            "app.py",
            "from pkg.b import b\nfrom src.pkg.b import b as c\n",
        ),
    ]);

    assert_eq!(
        facts_of(&fragment, "app.py"),
        vec![
            expect("pkg/b.py", "pkg.b", 1),
            expect("src/pkg/b.py", "src.pkg.b", 2),
        ]
    );
}

#[test]
fn should_still_emit_the_resolved_facts_when_another_import_is_unresolved() {
    let fragment = fragment(&[
        BARREL,
        ("pkg/app.py", "from .b import B\nfrom .ghost import G\n"),
        ("pkg/b.py", "B = 1\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "pkg/app.py"),
        vec![expect("pkg/b.py", ".b", 1)]
    );
    assert_eq!(
        fragment.files_without_imports,
        vec![SmolStr::new("pkg/app.py")]
    );
}

#[test]
fn should_emit_both_directions_of_an_import_cycle_in_a_deterministic_order() {
    let files = [
        BARREL,
        ("pkg/one.py", "from .two import b\na = 1\n"),
        ("pkg/two.py", "from .one import a\nb = 1\n"),
    ];

    let first = fragment(&files).imports;
    let second = fragment(&files).imports;

    assert_eq!(first, second);
    assert_eq!(
        first,
        vec![
            FileImport {
                importer: SmolStr::new("pkg/one.py"),
                imported: SmolStr::new("pkg/two.py"),
                kind: ImportKind::Value,
                specifier: SmolStr::new(".two"),
                line: 1,
                load_order: LoadOrder::Eager,
            },
            FileImport {
                importer: SmolStr::new("pkg/two.py"),
                imported: SmolStr::new("pkg/one.py"),
                kind: ImportKind::Value,
                specifier: SmolStr::new(".one"),
                line: 1,
                load_order: LoadOrder::Eager,
            },
        ]
    );
}

#[test]
fn should_spell_both_paths_exactly_like_the_file_containers() {
    let fragment = fragment(&[
        BARREL,
        ("pkg/app.py", "from .b import B\nfrom . import sub\n"),
        ("pkg/b.py", "B = 1\n"),
        ("pkg/sub/__init__.py", "S = 1\n"),
    ]);

    let files: Vec<&str> = fragment
        .containers
        .iter()
        .filter(|container| container.level == ScopeLevel::File)
        .map(|container| container.name.as_str())
        .collect();
    assert!(!fragment.imports.is_empty());
    for import in &fragment.imports {
        assert!(files.contains(&import.importer.as_str()), "{import:?}");
        assert!(files.contains(&import.imported.as_str()), "{import:?}");
    }
}

/// Collects every `.py` file below `dir` as `(path relative to base, contents)`.
fn python_files(base: &Path, dir: &Path, found: &mut Vec<(String, String)>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            python_files(base, &path, found);
        } else if path.extension().is_some_and(|extension| extension == "py")
            && let (Ok(relative), Ok(contents)) =
                (path.strip_prefix(base), fs::read_to_string(&path))
        {
            found.push((relative.to_string_lossy().replace('\\', "/"), contents));
        }
    }
}

/// A4: every non-external import of every Python eval fixture resolves.
#[test]
fn should_state_imports_for_every_python_eval_fixture() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../eval/fixtures");
    let mut checked = 0;
    let mut roots: Vec<_> = fs::read_dir(&fixtures)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    roots.sort();
    for root in roots.iter().filter(|root| root.is_dir()) {
        let mut found = Vec::new();
        python_files(root, root, &mut found);
        if found.is_empty() {
            continue;
        }
        let pairs: Vec<(&str, &str)> = found
            .iter()
            .map(|(path, contents)| (path.as_str(), contents.as_str()))
            .collect();
        let fragment = fragment(&pairs);

        assert!(fragment.states_imports, "{}", root.display());
        assert!(
            fragment.files_without_imports.is_empty(),
            "{}: {:?}",
            root.display(),
            fragment.files_without_imports
        );
        checked += 1;
    }
    assert!(checked > 0, "no Python eval fixture found");
}

#[test]
fn should_keep_an_import_under_a_negated_guard_other_than_type_checking() {
    let source = "DEBUG = False\nif not DEBUG:\n    from .runtime import RT\n";
    let fragment = fragment(&[
        BARREL,
        ("pkg/app.py", source),
        ("pkg/runtime.py", "RT = 1\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "pkg/app.py"),
        vec![expect("pkg/runtime.py", ".runtime", 3)]
    );
}

#[test]
fn should_state_no_file_and_no_gap_for_a_name_a_namespace_package_does_not_hold() {
    // a namespace package has no file of its own to load
    let fragment = fragment(&[
        ("app.py", "from ns import missing\nfrom ns import *\n"),
        ("ns/mod.py", "M = 1\n"),
    ]);

    assert_eq!(facts_of(&fragment, "app.py"), vec![]);
    assert!(fragment.states_imports);
    assert!(fragment.files_without_imports.is_empty());
}

#[test]
fn should_resolve_a_relative_import_from_the_unnamed_root_package_by_submodule_only() {
    // the analysed root holds no `__init__.py`: only a submodule is a file
    let fragment = fragment(&[
        (
            "app.py",
            "from . import other\nfrom . import missing\nfrom . import *\n",
        ),
        ("other.py", "O = 1\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "app.py"),
        vec![expect("other.py", ".", 1)]
    );
    assert!(fragment.files_without_imports.is_empty());
}

#[test]
fn should_keep_the_path_name_when_a_src_root_alias_spells_a_root_module() {
    // `src/` is a source root here, so `src/pkg/b.py` could be `pkg.b`, but
    // the root `pkg/b.py` already spells that name
    let fragment = fragment(&[
        ("src/pkg/__init__.py", ""),
        ("src/pkg/b.py", "b = 1\n"),
        ("pkg/__init__.py", ""),
        ("pkg/b.py", "b = 2\n"),
        ("app.py", "from pkg.b import b\n"),
    ]);

    assert_eq!(
        facts_of(&fragment, "app.py"),
        vec![expect("pkg/b.py", "pkg.b", 1)]
    );
    assert!(fragment.files_without_imports.is_empty());
}
