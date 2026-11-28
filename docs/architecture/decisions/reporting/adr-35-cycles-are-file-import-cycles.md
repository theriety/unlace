# ADR-35: Cycle findings are file-import cycles

📌

A cycle finding reports a strongly connected component of the file-import graph that the adapters state. Its severity says whether the cycle can fail at load time: a violation when any of its imports runs at module load, otherwise borderline. It names the imports whose removal breaks the cycle, taking a child's import of its parent entry before the entry's own imports into its folder. If a file on the cycle states no imports, it says the cut cannot be named. It never offers only symbol edges, and never an edge inside one file.

- Status: `Accepted`
- Date: `2026-11-26`
- Resolves: findings C1 and H4 of the mdc run; discovery rows disc-24, disc-29b, disc-35 and disc-36 (advice-false-moves); direction-review rework B1, N3 and the coverage notes (REV02)
- Builds on: user decisions D5 (cycle findings move to file grain), E3 (adapters state imports; the engine never projects them), and the REV02 choices D1 (Rust imports carry no load order), D4 (Rust import facts from macro bodies), D5 (opt-in coverage gate), D6 (src-layout coverage) and D8 (cut direction at a companion entry; function-only TypeScript cycles are borderline)

## 🎯 Motivation

Unlace 1.0.0 ran a minimum feedback arc set (MFAS) over every multi-member strongly connected component of the hard symbol graph. It reported the component unless every member sat in one file, and suggested the MFAS break set. On mdc, every suggestion was a same-file cut, such as `applyAdapters -> recognizeAdapter` inside `parser/adapters.ts`, on a cycle that spans `parser/adapters.ts ⇄ parser/build.ts` (C1). Cutting a call inside one file cannot remove an import cycle between files, and several suggested cuts severed recursive-descent dispatch. The chooser preferred intra-file edges because they were cheapest (H4). The detail text "unlace keeps a dependency cycle in one file; to split it, break …" did not say which files or which imports were involved. The cut was also not deterministic: `applyAdapters -> recognizeAdapter` and `buildAst -> applyAdapters` tied at weight 1.0, and debug and release builds picked different edges (disc-24).

The first file-grain draft fixed the grain but not the judgement. On unlace's own tree it reported 21 cycle violations, every one a Rust parent facade and its children (`pub use child::f` in the parent, `use super::X` in the child). Rust `use` has no module evaluation order, so none can fail. On mdc, both cycles are reached only from function bodies, yet they gated `--fail-on cycle`. The cheapest cut on the lexer cycle removed `block.ts → ./container`, the import that assembles the module, instead of the children's `../block` back references.

## ✅ Decision

1. **Graph.** The graph's vertices are files and its edges are the IR's `FileImport` facts (`crates/engine/src/analyze/findings/import_cycles/graph.rs`). Value and side-effect imports both count, because both load the target; module declarations do not (§7). Parallel statements between one file pair collapse into one edge that keeps every statement (specifier, line and load order). Self-imports are left out. A component of two or more files is a cross-file cycle and becomes one `cycle` finding, located by its files in path order.
2. **Load order and severity.** Each `FileImport` carries a `load_order` the adapter states (IR schema 4, omitted when `eager`):
   - **`eager`**: the import runs when its file is first loaded and needs the target's bindings then.
   - **`deferred`**: the binding is read only inside function bodies, or the statement only re-exports.
   - **`unordered`**: the language has no module evaluation order.

   A cycle is a `violation` when any statement on any of its edges is eager. Every edge of a strongly connected component lies on a cycle, so one eager statement is a load-time hazard for whichever entry point closes the cycle. Otherwise the cycle is `borderline`, and its detail ends with the reason: "no import in it runs at load time" (`ImportCycle.noLoadOrder = deferred`) or "its language's imports carry no load order" (`unordered`). Borderline cycles are still listed, because they are real file dependencies, but `--fail-on cycle` gates only violations. The engine reads only the fact; which statement is eager is each adapter's decision:
   - **Rust** stamps every import `unordered`. A cycle through Rust files alone is therefore borderline, whether cut or unavailable.
   - **Python** stamps every import `eager`, because a module-level import executes when the module loads.
   - **TypeScript** stamps:
     - side-effect imports and `import x = require()` `eager`;
     - re-exports `deferred`;
     - a static import `eager` exactly when a value binding it introduces is read at load time.

     Load time covers top-level statements and initialisers (destructuring defaults and computed keys included), class `extends`, static properties and blocks, computed keys, enum initialisers, namespace bodies, the default-export expression, immediately invoked functions, and functions passed straight to a load-time call or `new`. Function, arrow, method and instance-property bodies are deferred, and type positions never count. A local declaration read at load time carries its own runtime reads into load time, iterated to a fixpoint, so `const y = f();` makes every import `f` reads eager. A class whose load-time code reads `this` or `super`, as `static { this.init(); }` does, counts as read at load, so its methods' reads join load time too.
3. **Weight.** Removing the import `A → B` costs the number of distinct hard symbol edges from symbols in `A` to symbols in `B`, counted by node identity so two symbols that share a display name count twice, and at least 1, the cost of deleting the statement itself. These are the references a reader must relocate or invert before the import can go. Weights are integers, so ties are exact.
4. **Direction.** An entry file stands for its folder (`FileFact.entry_of`, falling back to `folder_companion`; `import_cycles/direction.rs`):
   - **Rust**: `lib.rs`, `main.rs` and `mod.rs` for their directory, and `foo.rs` with children in `foo/`;
   - **TypeScript**: the companion `x.ts` beside `x/`, and the single `x/index.*`;
   - **Python**: `pkg/__init__.py`.

   The entry's imports into its own folder assemble the module, so removing one tears the module apart; a child's import back up to the entry is the dependency to undo. Each import `A → B` is classed by physical path:
   - **Descending**: `A` is the entry of a folder that holds `B`.
   - **Ascending**: `B` is an entry, and `A` lies inside `B`'s folder or in a subdirectory of `B`'s own directory, as `lexer/container/quote.ts → lexer/block.ts` does.
   - **Lateral**: every other import.

   A cut is priced lexicographically by:
   1. the number of descending imports removed;
   2. the number of removed imports that are not ascending;
   3. the symbol references they carry;
   4. the number of imports removed.

   So a cut of back edges beats one that removes a cheaper sideways import. The first three criteria are folded into one additive integer per edge (`descending·B2 + nonAscending·B1 + references`, with `B1` above any cut's total references and `B2` above any cut's other terms). The cut solver stays unaware of the tiers. If the encoding would overflow, the raw weights are used. Only the cycle cut reads `entry_of`. Placement, capacity and pins keep reading `folder_companion` (ADR-31). `ImportBreak.weight` reports the raw reference count.
5. **Cut.** The cut is a minimum feedback arc set of the component under the encoded weights (`import_cycles/cut.rs`). A component of up to 12 files is solved exactly by dynamic programming over file subsets: a cut is the backward-edge set of some linear order of the files, and the cheapest order is built one file at a time. Larger components take an Eades–Lin–Smyth order, then each cut edge that no longer closes a cycle is restored, and the cut is marked heuristic (`exact: false`). The order keeps every file in a bucket keyed by its outgoing-minus-incoming weight and updates only the placed file's neighbours, on adjacency lists. Each bucket is an ordered set, so a choice still falls to the lowest index, at O((V+E)·log V). The restore pass walks the kept edges once per cut edge, for O(c·(V+E)) with a cut of c imports.

   There is no time budget and no ILP, so the same input always yields the same cut. Among cuts of equal weight the solver prefers fewer imports. Both criteria are additive, so the exact cut is optimal on both. A remaining tie is settled by comparing the sorted `(importer, imported)` lists per file subset as the cut is built. That choice is deterministic, but it is not guaranteed to be the globally smallest list. Files are numbered by path and never by node or container id. This fixes disc-24.
6. **Finding.** `Violation.importCycle` carries these fields:
   - `status`: `cut` or `unavailable`;
   - `cut`: one `ImportBreak` per removed edge, holding `importer`, `imported`, every `statements` entry as `{line, specifier}`, the carried `references` as `{source, target}` symbol names, and `weight`;
   - `exact`;
   - `filesWithoutImports`;
   - `noLoadOrder`.

   The detail line names the file count, the first import to remove with its specifier, line and carried references, the method, how many more imports follow, and the borderline reason. For example: "2-file import cycle; to break it, remove `./build` (line 7) from `src/parser/adapters.ts`, which carries 1 symbol reference (`parseFragment` -> `buildAst`) (exact); no import in it runs at load time". The report lists each further import on its own "Also remove …" line. `breakSuggestions` is no longer set on a cycle finding, but stays readable in saved results.
7. **Unavailable, per cycle.** The import graph cannot see a cycle through a file whose adapter stated no imports (`Snapshot::imports_available_for` is false). Each cross-file component of the hard symbol graph that touches such a file is therefore reported as a cycle finding with `status: unavailable` and the files that lack facts. It is never reported as an empty cut and never dropped.
   - **Where the graph is authoritative.** A symbol component whose files all state their imports is left to the import graph, which is authoritative for those files. A type-only back reference forms a symbol cycle but no import cycle, so no finding is reported.
   - **What a file's coverage claims.** That authority stops where the adapter's facts stop, and coverage is stated per file. A file states its imports unless its adapter lists it in `IrFragment.files_without_imports`:
     - The Rust adapter lists a file with an unresolved `use` leaf.
     - The Python adapter lists an importer with an unresolved first-party import.
     - The rest of the crate or package stays covered.
   - **Rust macro bodies.** The Rust adapter reads paths inside macro bodies that are comma-separated expression lists (`println!("{}", crate::a::f())`, `assert!`, `vec!`, nested ones included). It cannot read a `use` inside a macro, items a `macro_rules!` produces, bodies that are not expression lists, or proc macros, because the loader runs without the proc-macro server. Those files still count as covered, so a cycle closed only through such a body is neither cut nor reported (D-52).
   - **What an unavailable finding lists.** It reports only what no cut finding already reports:
     - It is left out when cut findings list every file of the symbol component, or when every file states its imports.
     - Otherwise its location is the files no cut finding lists, plus the files without facts, and `filesWithoutImports` names the files without facts.
     - A fact-less file that a cut finding also lists is still named when the component reaches an unlisted file through it, so the cycle is never dropped.
     - So a symbol component over `a`, `b` and a fact-less `c`, beside an import cycle `a ⇄ b`, yields the cut over `a, b` and an unavailable finding over `c` alone, never the same files twice.
     - Its detail counts the files listed ("dependency cycle through 1 file"), not the cycle's size.
   - **Severity of an unavailable finding.** With no facts to read, it is borderline only when every file it runs through belongs to a language without load order (`IrFragment.unordered_imports`); otherwise it is a violation.
8. **Coverage per run and its gate.** `current.cycleCoverage` records `status` (`complete`, `partial` or `unavailable`) and `filesWithoutImports`. When no file states its imports (a schema 2 or 3 IR, or every adapter reported `states_imports = false`), the status is `unavailable`, so zero cycle findings never reads as zero cycles. The report says so under the shared findings, and `violations` ends its table with the coverage note when coverage is not complete. Coverage does not change the exit code by default. `violations --fail-on cycle-coverage` (also `cycleCoverage`) exits 2 when coverage is `partial` or `unavailable`. It takes no severity qualifier.
9. **Rust module declarations (disc-36).** A `mod child;` declaration states ownership, not a use, so it never forms or closes a cycle. Otherwise every parent `mod child;` plus the child's `use super::*` would form a 2-cycle. The adapter says which facts are declarations: the Rust adapter emits `mod child;` as `ImportKind::ModuleDeclaration`, and the graph leaves out every declaration fact.
   - **Pairs with uses.** A file pair whose facts are all declarations has no edge. A pair that also states a `Value` or `SideEffect` use keeps an edge whose statements are just those uses, so a cut never names a `mod` line.
   - **The parent's uses.** The parent's uses of the child's items are separate `Value` facts, for `use child::x` and for qualified paths such as `child::run()`, `crate::a::f()` or `super::T` that resolve to another file. So a parent that calls `child::run()` while the child uses `super::Config` is a real 2-cycle and is reported, as a borderline finding.
   - **No guessing from spelling.** The engine no longer guesses declarations from the specifier's spelling. An earlier draft treated a bare identifier from a file with `::` specifiers as a declaration, on the premise that every `use` leaf is spelled with `::`. That premise is false: `use b;` and a qualified path with one module segment both spell a bare name, and the guess dropped real uses.
   - **Test and cfg children.** A `#[cfg(test)] mod tests;` child follows the same rule: its declaration is left out, and its `use super::*` back-edge has no forward use to close a cycle. A `cfg`-disabled `mod` is still emitted as a declaration and left out like every declaration.
10. **Python (disc-35).** A parent package's `__init__.py` is not emitted as an implicit load of each submodule import, and `from pkg import sub` targets `pkg/sub.py`. This is acceptable for cycle findings. Python runs `pkg/__init__.py` first, but a cycle that closes only through an `__init__` the importer never names is a package-initialisation order issue, not an import the reader can remove. An `__init__.py` that itself imports its submodules states those imports and joins the graph normally.
    - **Source roots.** The resolver registers each module under a source root under its import name. A source root is a directory that is not a package, whose parent is not one, and that is named `src` or directly holds a package; the repository root is the default root. So `pkg.x` resolves to `src/pkg/x.py`, and `app.x` to `backend/app/x.py`.
    - **Collisions and shared names.** A real path name wins a collision. A name that two source roots share is first-party but unresolved, so its importer is listed in `filesWithoutImports` and coverage reads `partial`.
11. **Symbol-grain machinery.** Cycle findings no longer read the symbol MFAS. The unavailable findings above use the hard-edge components as witnesses and read only their membership, so they take it from the condensation alone and solve nothing. `solve_cycles` still solves every multi-member hard-edge SCC with the HiGHS ILP or heuristic, for one consumer: `conditional_splits`, once per profile result. Splitting an over-cap component into several files cuts symbol edges, so its `preconditions` stay `EdgeBreak`s. `hard_capacity_breaks` counts capacity findings only and never read cycle solutions. Intra-file recursion stays unreported: a symbol component inside one file container yields no import edge and no unavailable finding.

## 🔀 Alternatives considered

- **Project file edges from symbol edges.** Rejected by E3. A projected edge cannot be located by line or specifier, sees type-only references as loads, and misses side-effect imports. The adapter that owns the language states its imports.
- **Keep symbol grain and prefer file-crossing cuts.** Rejected by D5. A symbol cut that crosses files still leaves the import in place when other references use it, so it does not name the change that breaks the cycle. The 1.0.0 output also showed that cheaper same-file edges win any symbol-grain tie.
- **Reuse the core ILP for the file graph.** Rejected. Its time budget makes the exact-versus-heuristic outcome machine-dependent, and its tie-breaking follows dense ids. File components are small, so the subset DP is exact and deterministic without a solver.
- **Report an unavailable cycle per run only.** Rejected. A partially covered run would hide which cycles could not be cut. An example is a TypeScript package beside a Rust crate whose adapter could not resolve every `use`. A per-cycle status keeps every symbol-visible cycle in the report, and the per-run coverage covers the cycles nothing can see.
- **Drop every Rust parent/child edge.** Rejected. A parent re-exporting a child while the child uses the parent is a real two-way dependency, the same shape as the companion-folder cycle C1 asks unlace to report. It is listed as borderline instead.
- **Keep every cycle a violation.** Rejected by D1. It gated 21 idiomatic Rust facades on unlace itself and both function-only mdc cycles, none of which can fail at load time.
- **Grade Rust cycles by a symbol witness.** Rejected by D1. This covers two variants: borderline only when no hard symbol cycle spans the files, or a violation only when one does. A symbol cycle in Rust still carries no load order, so the witness would grade a hazard that does not exist. Load order as an adapter fact keeps the engine language-neutral (ADR-6).
- **Grade TypeScript cycles by import kind alone.** Rejected by D8. A plain value import read only inside functions is the common mdc shape and cannot fail at load time. Only the place the binding is read tells the two apart.
- **Strict ascending (the importer inside the entry's own folder only).** Rejected. A sibling folder's file importing the entry, as `lexer/container/quote.ts → lexer/block.ts` does, would stay lateral. The cut would then still remove `block.ts → ./container`, which assembles the module.
- **Mark every file with an unreadable macro body as partial.** Rejected. Almost every Rust crate has a DSL or proc macro somewhere, so coverage would read `partial` everywhere and stop telling runs apart. The residual is recorded instead (D-52).
- **Gate incomplete coverage by default.** Rejected by D5. It would change the exit code of existing CI jobs on any crate with one unresolved `use`.

## ⚖️ Consequences

- **Eval pin.** A cross-file cycle finding names imports between two of its files, with statement lines. The eval kind `cycle_import_cut` pins this on `companion-folders` and fails on 1.0.0's same-file-only output. Its optional `expect` pins the import the cut must name:
  - `parser/adapters.ts → parser/build.ts` line 7;
  - `lexer/block/mdcx.ts → lexer/block.ts` line 3, a child's back edge. The entry's own `block.ts → block/mdcx.ts` is never named.
- **Schema.** Result schema 12 gains `Violation.importCycle` (with `noLoadOrder`) and `CurrentTree.cycleCoverage`. IR schema 4 gains `FileImport.load_order`, `FileFact.entry_of`, `IrFragment.files_without_imports` and `IrFragment.unordered_imports`. Neither version has shipped, so no bump. Every new field defaults when absent:
  - A saved schema-11 or early schema-12 result still reads, with coverage `complete` and the old `breakSuggestions` rendered as before.
  - An IR without `load_order` reads as eager.
- **Goldens.**
  - Goldens with cycle findings (`cyclic`, `cyclic-oversized`, `dupflat`) name an import to remove instead of a symbol edge.
  - `cyclic-oversized` is borderline, because no import in it runs at load time.
  - No Rust golden gains a finding.
  - Every golden's snapshot hash changes with the new IR facts.
- **Disappearing findings.** A symbol cycle whose back edge is type-only is no longer reported. Unlace 1.0.0 reported it, because a type reference is a hard symbol edge, but a type-only import loads nothing at runtime, so the files form no import cycle. A reader upgrading from 1.0.0 sees such findings disappear. A cycle through a file without import facts is reported as unavailable, with no cut.
- **Exit codes and Rust crates.** `--fail-on cycle` now fails only on cycles that can fail at load time:
  - Real Rust crates list their `use` cycles between modules, often a parent facade and its children, as borderline. They no longer gate.
  - A TypeScript cycle reached only from function bodies is borderline.
  - A Python cycle is a violation.
- **TypeScript residuals.**
  - A load-time call through dynamic dispatch, such as a method on an imported object's property read at load time, is classed by where the binding is read, not where the call lands.
  - A callback passed to a top-level `setTimeout` counts as load time.
  - Decorators are not parsed, so a file that uses one fails the whole run with a parse error, as in 1.0.0.
  - These are recorded in the deviations.
- **Coverage gate.** `--fail-on cycle-coverage` makes an incomplete view fail CI. Rust and Python mark only the affected files, so one unresolved `use` makes a run `partial`, not `unavailable`. Even so, the gate fails on any such file, which is why it is opt-in.
- **Cost.** Findings no longer solve the symbol MFAS: the unavailable witnesses come from the condensation, O(V+E) on the symbol graph. The MFAS still runs once per profile result, for conditional splits. The file-graph cut adds O(2^n · n²) for a component of up to 12 files, and O((V+E)·log V + c·(V+E)) above that, for V files, E imports and a cut of c. Classing each edge by direction is O(E) path comparisons. A single 3000-file component with 15000 imports cuts in under a second in a debug build.
