//! File-import facts: the adapter, not the engine, states which statements make
//! one analysed file load another (decision E3).

use std::fs;
use std::path::{Path, PathBuf};

use smol_str::SmolStr;
use unlace_adapter_typescript::TypeScriptAdapter;
use unlace_ir::{Adapter, FileImport, ImportKind, IrFragment, LoadOrder, ScopeLevel, SourceFile};

/// Parses and binds `files` (path, contents) with `adapter`.
fn fragment_with(adapter: &TypeScriptAdapter, files: &[(&str, &str)]) -> IrFragment {
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

/// The imports emitted for `files`, with the default adapter.
fn imports(files: &[(&str, &str)]) -> Vec<FileImport> {
    fragment_with(&TypeScriptAdapter::default(), files).imports
}

/// The imports of `files` as `(importer, imported, kind, specifier, line)`.
fn rows(files: &[(&str, &str)]) -> Vec<(String, String, ImportKind, String, u32)> {
    imports(files)
        .into_iter()
        .map(|import| {
            (
                import.importer.to_string(),
                import.imported.to_string(),
                import.kind,
                import.specifier.to_string(),
                import.line,
            )
        })
        .collect()
}

fn row(
    importer: &str,
    imported: &str,
    kind: ImportKind,
    specifier: &str,
    line: u32,
) -> (String, String, ImportKind, String, u32) {
    (
        importer.to_owned(),
        imported.to_owned(),
        kind,
        specifier.to_owned(),
        line,
    )
}

const TARGET: (&str, &str) = ("src/x.ts", "export const a = 1;\nexport type T = number;\n");

#[test]
fn should_state_imports_once_the_adapter_emits_them() {
    let fragment = fragment_with(&TypeScriptAdapter::default(), &[TARGET]);

    assert!(fragment.states_imports);
}

#[test]
fn should_emit_no_fact_for_a_type_import_and_one_for_a_side_effect_import() {
    // A1: a type-only import creates no fact, a side-effect import does.
    let typed = (
        "src/typed.ts",
        "import type { T } from './x';\nexport const t = 1;\n",
    );
    let effect = ("src/effect.ts", "import './x';\nexport const e = 1;\n");

    assert_eq!(rows(&[TARGET, typed]), Vec::new());
    assert_eq!(
        rows(&[TARGET, effect]),
        vec![row(
            "src/effect.ts",
            "src/x.ts",
            ImportKind::SideEffect,
            "./x",
            1
        )]
    );
}

#[test]
fn should_emit_a_value_import_for_every_binding_form() {
    let consumer = (
        "src/consumer.ts",
        "import { a } from './x';\nimport x from './x';\nimport * as ns from './x';\n",
    );

    assert_eq!(
        rows(&[TARGET, consumer]),
        vec![
            row("src/consumer.ts", "src/x.ts", ImportKind::Value, "./x", 1),
            row("src/consumer.ts", "src/x.ts", ImportKind::Value, "./x", 2),
            row("src/consumer.ts", "src/x.ts", ImportKind::Value, "./x", 3),
        ]
    );
}

#[test]
fn should_emit_a_value_import_for_a_re_export() {
    let barrel = (
        "src/barrel.ts",
        "\nexport { a } from './x';\nexport * from './x';\nexport * as all from './x';\n",
    );

    assert_eq!(
        rows(&[TARGET, barrel]),
        vec![
            row("src/barrel.ts", "src/x.ts", ImportKind::Value, "./x", 2),
            row("src/barrel.ts", "src/x.ts", ImportKind::Value, "./x", 3),
            row("src/barrel.ts", "src/x.ts", ImportKind::Value, "./x", 4),
        ]
    );
}

#[test]
fn should_emit_a_value_import_for_an_import_equals_require() {
    let consumer = (
        "src/consumer.ts",
        "import x = require('./x');\nexport import y = require('./x');\n",
    );

    assert_eq!(
        rows(&[TARGET, consumer]),
        vec![
            row("src/consumer.ts", "src/x.ts", ImportKind::Value, "./x", 1),
            row("src/consumer.ts", "src/x.ts", ImportKind::Value, "./x", 2),
        ]
    );
}

#[test]
fn should_leave_out_every_type_only_form() {
    let consumer = (
        "src/consumer.ts",
        "import type { T } from './x';\n\
         import type * as ns from './x';\n\
         import type d from './x';\n\
         import { type T as A, type T as B } from './x';\n\
         export type { T } from './x';\n\
         export type * from './x';\n\
         export { type T } from './x';\n\
         import type e = require('./x');\n",
    );

    assert_eq!(rows(&[TARGET, consumer]), Vec::new());
}

#[test]
fn should_emit_a_value_import_for_a_mixed_type_and_value_import() {
    let consumer = (
        "src/consumer.ts",
        "import { type T, a } from './x';\nexport { type T, a } from './x';\n",
    );

    assert_eq!(
        rows(&[TARGET, consumer]),
        vec![
            row("src/consumer.ts", "src/x.ts", ImportKind::Value, "./x", 1),
            row("src/consumer.ts", "src/x.ts", ImportKind::Value, "./x", 2),
        ]
    );
}

#[test]
fn should_not_emit_a_dynamic_import_or_a_require_call() {
    let consumer = (
        "src/consumer.ts",
        "export async function load() {\n  await import('./x');\n  return require('./x');\n}\n",
    );

    assert_eq!(rows(&[TARGET, consumer]), Vec::new());
}

#[test]
fn should_leave_out_packages_and_unresolved_specifiers() {
    let consumer = (
        "src/consumer.ts",
        "import fs from 'node:fs';\nimport React from 'react';\nimport './missing';\nimport { z } from '../outside/z';\n",
    );

    assert_eq!(rows(&[TARGET, consumer]), Vec::new());
}

#[test]
fn should_leave_out_a_file_importing_itself() {
    let own = ("src/own.ts", "import './own';\nexport const o = 1;\n");

    assert_eq!(rows(&[own]), Vec::new());
}

#[test]
fn should_resolve_a_js_specifier_to_its_typescript_source() {
    let consumer = ("src/consumer.ts", "import { a } from './x.js';\n");

    assert_eq!(
        rows(&[TARGET, consumer]),
        vec![row(
            "src/consumer.ts",
            "src/x.ts",
            ImportKind::Value,
            "./x.js",
            1
        )]
    );
}

#[test]
fn should_resolve_a_directory_specifier_to_its_index_file() {
    let consumer = ("src/consumer.ts", "import { a } from './geometry';\n");
    let index = ("src/geometry/index.ts", "export const a = 1;\n");

    assert_eq!(
        rows(&[index, consumer]),
        vec![row(
            "src/consumer.ts",
            "src/geometry/index.ts",
            ImportKind::Value,
            "./geometry",
            1
        )]
    );
}

#[test]
fn should_record_the_line_a_multi_line_statement_starts_on() {
    let consumer = (
        "src/consumer.ts",
        "// header\n\n/* note */\nimport {\n  a,\n  b,\n} from './x';\n",
    );

    assert_eq!(
        rows(&[TARGET, consumer]),
        vec![row(
            "src/consumer.ts",
            "src/x.ts",
            ImportKind::Value,
            "./x",
            4
        )]
    );
}

#[test]
fn should_emit_in_a_deterministic_order_whatever_the_input_order() {
    let a = ("src/a.ts", "import './c';\nimport './b';\n");
    let b = ("src/b.ts", "export const b = 1;\n");
    let c = ("src/c.ts", "import './b';\n");

    let forward = imports(&[a, b, c]);
    let reverse = imports(&[c, b, a]);

    assert_eq!(forward, reverse);
    assert_eq!(forward.len(), 3);
}

#[test]
fn should_spell_both_paths_like_the_file_containers() {
    let consumer = ("src/consumer.ts", "import './x';\n");
    let fragment = fragment_with(&TypeScriptAdapter::default(), &[TARGET, consumer]);
    let files: Vec<&str> = fragment
        .containers
        .iter()
        .filter(|container| container.level == ScopeLevel::File)
        .map(|container| container.name.as_str())
        .collect();

    assert_eq!(fragment.imports.len(), 1);
    for import in &fragment.imports {
        assert!(files.contains(&import.importer.as_str()), "{files:?}");
        assert!(files.contains(&import.imported.as_str()), "{files:?}");
    }
}

/// A module exporting one name of each runtime shape the load-order cases read.
const PROVIDER: (&str, &str) = (
    "src/x.ts",
    "export const a = 1;\n\
     export function g() { return 1; }\n\
     export class Base {}\n",
);

/// The load order of each import `src/c.ts` (holding `code`) states for the
/// provider, in line order.
fn load_orders(code: &str) -> Vec<LoadOrder> {
    imports(&[PROVIDER, ("src/c.ts", code)])
        .into_iter()
        .filter(|import| import.importer == "src/c.ts")
        .map(|import| import.load_order)
        .collect()
}

#[test]
fn should_mark_an_import_called_at_the_top_level_eager() {
    assert_eq!(
        load_orders("import { g } from './x';\ng();\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_a_class_extends_eager() {
    assert_eq!(
        load_orders("import { Base } from './x';\nexport class C extends Base {}\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_read_by_a_static_property_eager() {
    assert_eq!(
        load_orders("import { a } from './x';\nexport class C {\n  static value = a;\n}\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_read_in_a_static_block_eager() {
    assert_eq!(
        load_orders("import { g } from './x';\nexport class C {\n  static {\n    g();\n  }\n}\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_a_computed_method_key_reads_eager() {
    assert_eq!(
        load_orders("import { a } from './x';\nexport class C {\n  [a]() {}\n}\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_a_static_block_reaches_through_this_eager() {
    // M1: the static block calls `init` through `this` as the module loads
    let code = "import { g } from './x';\n\
                export class Reg {\n  static {\n    this.init();\n  }\n  \
                static init() {\n    g();\n  }\n}\n";

    assert_eq!(load_orders(code), vec![LoadOrder::Eager]);
}

#[test]
fn should_mark_an_import_a_static_property_reaches_through_this_eager() {
    let code = "import { g } from './x';\n\
                export class Reg {\n  static x = this.create();\n  \
                static create() {\n    return g();\n  }\n}\n";

    assert_eq!(load_orders(code), vec![LoadOrder::Eager]);
}

#[test]
fn should_mark_an_import_a_static_block_reaches_through_super_eager() {
    // `super.boot()` runs `this.hook()`, which is `Reg.hook`, as the module
    // loads: only `Reg` itself reads `g`
    let code = "import { g } from './x';\n\
                class Base {\n  static boot() {\n    this.hook();\n  }\n  static hook() {}\n}\n\
                export class Reg extends Base {\n  static {\n    super.boot();\n  }\n  \
                static hook() {\n    g();\n  }\n}\n";

    assert_eq!(load_orders(code), vec![LoadOrder::Eager]);
}

#[test]
fn should_mark_an_import_a_class_expression_reaches_through_this_eager() {
    let code = "import { g } from './x';\n\
                export const Reg = class {\n  static {\n    this.init();\n  }\n  \
                static init() {\n    g();\n  }\n};\n";

    assert_eq!(load_orders(code), vec![LoadOrder::Eager]);
}

#[test]
fn should_keep_an_import_instance_members_read_through_this_deferred() {
    let code = "import { g } from './x';\n\
                export class Reg {\n  value = this.make();\n  \
                make() {\n    return this.run();\n  }\n  run() {\n    return g();\n  }\n  \
                static helper = () => this.make();\n}\n";

    assert_eq!(load_orders(code), vec![LoadOrder::Deferred]);
}

#[test]
fn should_mark_an_import_a_destructuring_default_reads_eager() {
    // m1: a default runs as the declaration binds
    assert_eq!(
        load_orders("import { g } from './x';\nexport const { v = g() } = {} as { v?: number };\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_a_destructuring_computed_key_reads_eager() {
    assert_eq!(
        load_orders("import { a } from './x';\nconst { [a]: v } = { 1: 2 };\nexport { v };\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_a_nested_or_array_destructuring_default_reads_eager() {
    assert_eq!(
        load_orders(
            "import { g } from './x';\nexport const { o: { v = g() } } = { o: {} as { v?: number } };\n"
        ),
        vec![LoadOrder::Eager]
    );
    assert_eq!(
        load_orders("import { g } from './x';\nexport const [v = g()] = [] as number[];\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_keep_an_import_a_destructuring_default_arrow_reads_deferred() {
    assert_eq!(
        load_orders(
            "import { g } from './x';\nexport const { v = () => g() } = {} as { v?: () => number };\n"
        ),
        vec![LoadOrder::Deferred]
    );
}

#[test]
fn should_mark_an_import_read_only_by_an_instance_property_deferred() {
    assert_eq!(
        load_orders("import { a } from './x';\nexport class C {\n  value = a;\n}\n"),
        vec![LoadOrder::Deferred]
    );
}

#[test]
fn should_mark_an_import_read_only_inside_methods_deferred() {
    assert_eq!(
        load_orders(
            "import { g } from './x';\nexport class C {\n  run() {\n    return g();\n  }\n  constructor() {\n    g();\n  }\n}\n"
        ),
        vec![LoadOrder::Deferred]
    );
}

#[test]
fn should_mark_an_import_read_only_inside_arrows_deferred() {
    assert_eq!(
        load_orders(
            "import { g } from './x';\nexport const handlers = { run: () => g(), stop() { g(); } };\n"
        ),
        vec![LoadOrder::Deferred]
    );
}

#[test]
fn should_mark_an_import_an_arrow_constant_reads_deferred() {
    assert_eq!(
        load_orders("import { a } from './x';\nexport const f = () => a;\n"),
        vec![LoadOrder::Deferred]
    );
}

#[test]
fn should_mark_an_import_read_through_a_local_function_called_at_load_eager() {
    // `local` reads `g` only in its body, but the top level calls `local`.
    let code = "import { g } from './x';\n\
                function local() {\n  return g();\n}\n\
                export const y = local();\n";

    assert_eq!(load_orders(code), vec![LoadOrder::Eager]);
}

#[test]
fn should_mark_an_import_read_through_a_local_function_never_used_at_load_deferred() {
    let code = "import { g } from './x';\n\
                function local() {\n  return g();\n}\n\
                export function y() {\n  return local();\n}\n";

    assert_eq!(load_orders(code), vec![LoadOrder::Deferred]);
}

#[test]
fn should_mark_an_import_an_immediately_invoked_function_reads_eager() {
    assert_eq!(
        load_orders("import { g } from './x';\nexport const v = (() => g())();\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_a_callback_reads_at_load_eager() {
    // the callback runs where the top-level call runs, chained or not
    assert_eq!(
        load_orders("import { g } from './x';\nexport const v = [1].map((n) => g() + n);\n"),
        vec![LoadOrder::Eager]
    );
    assert_eq!(
        load_orders(
            "import { g } from './x';\nexport const v = [1].map((n) => n).filter(() => g());\n"
        ),
        vec![LoadOrder::Eager]
    );
    assert_eq!(
        load_orders(
            "import { g } from './x';\nexport const p = new Promise(function () {\n  g();\n});\n"
        ),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_a_callback_reads_inside_a_function_deferred() {
    assert_eq!(
        load_orders(
            "import { g } from './x';\nexport function f() {\n  return [1].map((n) => g() + n);\n}\n"
        ),
        vec![LoadOrder::Deferred]
    );
}

#[test]
fn should_over_report_a_callback_a_top_level_call_stores_for_later_as_eager() {
    // `setTimeout` runs its callback later, but a callback argument is read as
    // load time: the conservative reading keeps the cycle a violation.
    assert_eq!(
        load_orders("import { g } from './x';\nsetTimeout(() => g());\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_close_over_a_local_function_a_load_time_callback_calls() {
    let code = "import { g } from './x';\n\
                function local() {\n  return g();\n}\n\
                export const v = [1].map(() => local());\n";

    assert_eq!(load_orders(code), vec![LoadOrder::Eager]);
}

#[test]
fn should_mark_a_side_effect_import_and_an_import_equals_require_eager() {
    assert_eq!(
        load_orders(
            "import './x';\nimport x = require('./x');\nexport function f() { return x; }\n"
        ),
        vec![LoadOrder::Eager, LoadOrder::Eager]
    );
}

#[test]
fn should_mark_every_re_export_deferred() {
    assert_eq!(
        load_orders(
            "export { a } from './x';\nexport * from './x';\nexport * as all from './x';\n"
        ),
        vec![LoadOrder::Deferred; 3]
    );
}

#[test]
fn should_mark_an_import_the_default_export_expression_reads_eager() {
    assert_eq!(
        load_orders("import { g } from './x';\nexport default g();\n"),
        vec![LoadOrder::Eager]
    );
    assert_eq!(
        load_orders("import { a } from './x';\nexport default a;\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_an_enum_initialiser_reads_eager() {
    assert_eq!(
        load_orders("import { a } from './x';\nexport enum E {\n  A = a,\n}\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_a_namespace_body_reads_eager() {
    assert_eq!(
        load_orders("import { g } from './x';\nexport namespace N {\n  export const v = g();\n}\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_a_value_import_used_only_as_a_type_deferred() {
    let code = "import { Base } from './x';\n\
                export let current: Base | undefined;\n\
                export function make(base: Base): Base {\n  return base;\n}\n\
                export class C implements Base {}\n";

    assert_eq!(load_orders(code), vec![LoadOrder::Deferred]);
}

#[test]
fn should_mark_a_namespace_import_by_where_its_members_are_read() {
    assert_eq!(
        load_orders("import * as ns from './x';\nexport const v = ns.a;\n"),
        vec![LoadOrder::Eager]
    );
    assert_eq!(
        load_orders("import * as ns from './x';\nexport function v() {\n  return ns.a;\n}\n"),
        vec![LoadOrder::Deferred]
    );
}

#[test]
fn should_order_each_import_of_one_file_on_its_own() {
    let code = "import { a } from './x';\nimport { g } from './x';\nexport const v = a;\nexport function f() {\n  return g();\n}\n";

    assert_eq!(
        load_orders(code),
        vec![LoadOrder::Eager, LoadOrder::Deferred]
    );
}

/// A throwaway repository root holding `files`, removed on drop.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str, files: &[(&str, &str)]) -> Result<Self, String> {
        let root = std::env::temp_dir().join(format!("unlace-ts-imports-{name}"));
        let _ = fs::remove_dir_all(&root);
        for (path, contents) in files {
            let full = root.join(path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::write(&full, contents).map_err(|error| error.to_string())?;
        }
        Ok(Self(root))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn should_resolve_a_tsconfig_alias_and_a_package_subpath_import() -> Result<(), String> {
    let root = TempRoot::new(
        "config",
        &[
            (
                "tsconfig.json",
                r#"{"compilerOptions":{"paths":{"@app/*":["src/*"]}}}"#,
            ),
            ("package.json", r##"{"imports":{"#x":"./src/x.ts"}}"##),
        ],
    )?;
    let adapter = TypeScriptAdapter::new(root.path());
    let consumer = (
        "src/consumer.ts",
        "import { a } from '@app/x';\nimport '#x';\nimport 'lodash';\n",
    );

    let fragment = fragment_with(&adapter, &[TARGET, consumer]);

    let found: Vec<(&str, &str, u32)> = fragment
        .imports
        .iter()
        .map(|import| {
            (
                import.specifier.as_str(),
                import.imported.as_str(),
                import.line,
            )
        })
        .collect();
    assert_eq!(
        found,
        vec![("@app/x", "src/x.ts", 1), ("#x", "src/x.ts", 2)]
    );
    Ok(())
}

#[test]
fn should_emit_both_directions_of_the_companion_folders_pair() -> Result<(), String> {
    // C1: `parser/adapters.ts` and `parser/build.ts` import each other.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../eval/fixtures/companion-folders");
    let mut sources = Vec::new();
    collect_ts(&root, &root, &mut sources)?;
    let adapter = TypeScriptAdapter::new(&root);
    let trees = adapter.parse(&sources).map_err(|error| error.to_string())?;
    let fragment = adapter.bind(trees).map_err(|error| error.to_string())?;

    let between = |from: &str, to: &str| -> Vec<(ImportKind, String, u32)> {
        fragment
            .imports
            .iter()
            .filter(|import| import.importer == from && import.imported == to)
            .map(|import| (import.kind, import.specifier.to_string(), import.line))
            .collect()
    };
    assert_eq!(
        between("src/parser/adapters.ts", "src/parser/build.ts"),
        vec![(ImportKind::Value, "./build".to_owned(), 7)]
    );
    assert_eq!(
        between("src/parser/build.ts", "src/parser/adapters.ts"),
        vec![(ImportKind::Value, "./adapters".to_owned(), 14)]
    );
    Ok(())
}

/// Reads every `.ts` file under `dir` as a repo-relative [`SourceFile`].
fn collect_ts(dir: &Path, root: &Path, out: &mut Vec<SourceFile>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|error| error.to_string())?;
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() {
            collect_ts(&path, root, out)?;
        } else if path.extension().is_some_and(|extension| extension == "ts") {
            let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
            out.push(SourceFile {
                path: SmolStr::new(relative.to_string_lossy().replace('\\', "/")),
                contents: fs::read_to_string(&path).map_err(|error| error.to_string())?,
            });
        }
    }
    Ok(())
}

#[test]
fn should_mark_an_import_read_only_inside_object_accessor_bodies_deferred() {
    assert_eq!(
        load_orders(
            "import { g } from './x';\nexport const o = {\n  get v() {\n    return g();\n  },\n  set v(n: number) {\n    g();\n  },\n};\n"
        ),
        vec![LoadOrder::Deferred]
    );
}

#[test]
fn should_mark_an_import_an_object_accessor_computed_key_reads_eager() {
    assert_eq!(
        load_orders(
            "import { a } from './x';\nexport const o = {\n  get [a]() {\n    return 1;\n  },\n};\n"
        ),
        vec![LoadOrder::Eager]
    );
    assert_eq!(
        load_orders("import { a } from './x';\nexport const o = {\n  set [a](n: number) {},\n};\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_by_whether_its_auto_accessor_is_static() {
    assert_eq!(
        load_orders("import { g } from './x';\nexport class C {\n  static accessor v = g();\n}\n"),
        vec![LoadOrder::Eager]
    );
    assert_eq!(
        load_orders("import { g } from './x';\nexport class C {\n  accessor v = g();\n}\n"),
        vec![LoadOrder::Deferred]
    );
}

#[test]
fn should_mark_an_import_constructed_without_arguments_at_the_top_level_eager() {
    assert_eq!(
        load_orders("import { Base } from './x';\nexport const b = new Base;\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_a_nested_namespace_body_reads_eager() {
    assert_eq!(
        load_orders(
            "import { g } from './x';\nexport namespace N {\n  export namespace M {\n    export const v = g();\n  }\n}\n"
        ),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_keep_an_import_read_only_by_types_inside_a_namespace_deferred() {
    let code = "import { a } from './x';\n\
                export namespace N {\n  \
                export interface I {\n    [a]: number;\n  }\n  \
                export type T = { [a]: number };\n  \
                export declare namespace M {\n    const v: typeof a;\n  }\n  \
                export const w = 1;\n}\n";

    assert_eq!(load_orders(code), vec![LoadOrder::Deferred]);
}

#[test]
fn should_mark_an_import_a_using_declaration_initialiser_reads_eager() {
    assert_eq!(
        load_orders("import { g } from './x';\nusing r = g();\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_a_quoted_module_body_reads_eager() {
    assert_eq!(
        load_orders("import { g } from './x';\nmodule 'ext' {\n  g();\n}\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_a_rest_binding_reaches_at_load_eager() {
    // `rest` and `all` hold the arrows that read `g`, and the top level calls them
    assert_eq!(
        load_orders(
            "import { g } from './x';\nconst { ...rest } = { run: () => g() };\nrest.run();\n"
        ),
        vec![LoadOrder::Eager]
    );
    assert_eq!(
        load_orders("import { g } from './x';\nconst [...all] = [() => g()];\nall[0]();\n"),
        vec![LoadOrder::Eager]
    );
}

#[test]
fn should_mark_an_import_a_default_inside_a_rest_pattern_reads_eager() {
    assert_eq!(
        load_orders("import { g } from './x';\nexport const [...[v = g()]] = [] as number[];\n"),
        vec![LoadOrder::Eager]
    );
}
