//! Cycle severity from the imports' load order (ADR-35): a cycle is a
//! violation only when one of its imports runs at load time.

use smol_str::SmolStr;
use unlace_ir::{FileImport, LoadOrder, Polarity, Snapshot};

use super::{analyzed, cycles, first, import, snapshot_with_imports};
use crate::analyze::test_support::{edge, node};
use crate::result::{ImportCycleStatus, NoLoadOrder, Severity, Violation};

const A: &str = "src/a.ts";
const B: &str = "src/b.ts";
const C: &str = "src/c.ts";

fn ordered(importer: &str, imported: &str, line: u32, order: LoadOrder) -> FileImport {
    FileImport {
        load_order: order,
        ..import(importer, imported, "./dep", line)
    }
}

/// A three-file cycle `a -> b -> c -> a` whose imports carry `orders` in turn.
fn triangle(orders: [LoadOrder; 3]) -> Result<Snapshot, String> {
    let [ab, bc, ca] = orders;
    snapshot_with_imports(
        &[A, B, C],
        Vec::new(),
        Vec::new(),
        vec![
            ordered(A, B, 1, ab),
            ordered(B, C, 1, bc),
            ordered(C, A, 1, ca),
        ],
        &[A, B, C],
    )
}

/// The only cycle finding of `snapshot`.
fn only_cycle(snapshot: &Snapshot) -> Result<Violation, String> {
    let result = analyzed(snapshot)?;
    let found = cycles(&result);
    assert_eq!(found.len(), 1, "{found:?}");
    first(&found).cloned()
}

#[test]
fn should_report_a_cycle_of_eager_imports_as_a_violation() -> Result<(), String> {
    let finding = only_cycle(&triangle([LoadOrder::Eager; 3])?)?;

    assert_eq!(finding.severity, Severity::Violation);
    assert_eq!(
        finding
            .import_cycle
            .as_ref()
            .and_then(|cycle| cycle.no_load_order),
        None
    );
    assert!(!finding.detail.contains("load"), "{}", finding.detail);
    Ok(())
}

#[test]
fn should_report_a_cycle_with_one_eager_import_as_a_violation() -> Result<(), String> {
    let finding = only_cycle(&triangle([
        LoadOrder::Deferred,
        LoadOrder::Eager,
        LoadOrder::Unordered,
    ])?)?;

    assert_eq!(finding.severity, Severity::Violation);
    Ok(())
}

#[test]
fn should_report_a_cycle_of_deferred_imports_as_borderline_and_say_why() -> Result<(), String> {
    let finding = only_cycle(&triangle([
        LoadOrder::Deferred,
        LoadOrder::Deferred,
        LoadOrder::Unordered,
    ])?)?;

    assert_eq!(finding.severity, Severity::Borderline);
    assert_eq!(
        finding
            .import_cycle
            .as_ref()
            .and_then(|cycle| cycle.no_load_order),
        Some(NoLoadOrder::Deferred)
    );
    assert!(
        finding
            .detail
            .ends_with("; no import in it runs at load time"),
        "{}",
        finding.detail
    );
    // still listed with its cut: borderline changes the gate, not the advice.
    assert!(
        finding
            .import_cycle
            .as_ref()
            .is_some_and(|cycle| !cycle.cut.is_empty())
    );
    Ok(())
}

#[test]
fn should_report_a_cycle_of_unordered_imports_as_borderline_and_say_why() -> Result<(), String> {
    let finding = only_cycle(&triangle([LoadOrder::Unordered; 3])?)?;

    assert_eq!(finding.severity, Severity::Borderline);
    assert_eq!(
        finding
            .import_cycle
            .as_ref()
            .and_then(|cycle| cycle.no_load_order),
        Some(NoLoadOrder::Unordered)
    );
    assert!(
        finding
            .detail
            .ends_with("; its language's imports carry no load order"),
        "{}",
        finding.detail
    );
    Ok(())
}

#[test]
fn should_read_an_eager_statement_behind_an_otherwise_deferred_edge() -> Result<(), String> {
    // two statements from `a` to `b`: one deferred, one a side-effect import.
    let snapshot = snapshot_with_imports(
        &[A, B],
        Vec::new(),
        Vec::new(),
        vec![
            ordered(A, B, 1, LoadOrder::Deferred),
            ordered(A, B, 2, LoadOrder::Eager),
            ordered(B, A, 1, LoadOrder::Deferred),
        ],
        &[A, B],
    )?;

    let finding = only_cycle(&snapshot)?;

    assert_eq!(finding.severity, Severity::Violation);
    Ok(())
}

/// A cross-file symbol cycle over two Rust files whose imports are not
/// stated; `unordered` lists the files whose language has no load order.
fn rust_witness(unordered: &[&str]) -> Result<Snapshot, String> {
    let lib = "src/lib.rs";
    let util = "src/util.rs";
    super::snapshot_with(
        &[lib, util],
        vec![
            node(0, "run", 0, Polarity::Production),
            node(1, "help", 1, Polarity::Production),
        ],
        vec![edge(0, 1), edge(1, 0)],
        Vec::new(),
        &[],
        |ir| ir.unordered_files = unordered.iter().copied().map(SmolStr::new).collect(),
    )
}

#[test]
fn should_report_an_unavailable_rust_witness_as_borderline_and_keep_it_unavailable()
-> Result<(), String> {
    let finding = only_cycle(&rust_witness(&["src/lib.rs", "src/util.rs"])?)?;

    let cycle = finding.import_cycle.as_ref().ok_or("a cycle finding")?;
    assert_eq!(cycle.status, ImportCycleStatus::Unavailable);
    assert_eq!(cycle.no_load_order, Some(NoLoadOrder::Unordered));
    assert_eq!(finding.severity, Severity::Borderline);
    assert!(
        finding.detail.contains("import facts are unavailable")
            && finding
                .detail
                .ends_with("; its language's imports carry no load order"),
        "{}",
        finding.detail
    );
    Ok(())
}

#[test]
fn should_keep_a_witness_a_violation_when_one_of_its_files_has_load_order() -> Result<(), String> {
    let finding = only_cycle(&rust_witness(&["src/lib.rs"])?)?;

    assert_eq!(finding.severity, Severity::Violation);
    Ok(())
}
