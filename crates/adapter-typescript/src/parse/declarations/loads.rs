//! Runtime module loads stated by top-level import and re-export statements.

use std::collections::BTreeSet;

use smol_str::SmolStr;
use swc_common::{BytePos, Span, Spanned};
use swc_ecma_ast::{ExportSpecifier, ImportSpecifier, ModuleDecl, TsModuleRef};
use unlace_ir::{ImportKind, LoadOrder};

use crate::parse::{ModuleLoad, specifier};

/// The runtime load `decl` states, or `None` when it loads nothing at runtime:
/// a declaration that is not an import or re-export, a statement written
/// `type`, or one whose every specifier is `type`-qualified.
///
/// `load_time` holds the names the module reads while it loads (see
/// `eager.rs`): an import binding one of them is [`LoadOrder::Eager`], and
/// one whose names are all read later is [`LoadOrder::Deferred`].
pub(super) fn module_load(
    decl: &ModuleDecl,
    load_time: &BTreeSet<SmolStr>,
    contents: &str,
    base: BytePos,
) -> Option<ModuleLoad> {
    let (source, kind, names, load_order) = match decl {
        ModuleDecl::Import(import) if !import.type_only => {
            let kind = import_kind(&import.specifiers)?;
            let names = bound_names(&import.specifiers);
            let read_at_load = names.iter().any(|name| load_time.contains(name));
            let load_order = if kind == ImportKind::SideEffect || read_at_load {
                LoadOrder::Eager
            } else {
                LoadOrder::Deferred
            };
            (specifier(&import.src), kind, names, load_order)
        }
        // a re-export makes an alias and reads nothing as the module loads
        ModuleDecl::ExportNamed(named) if !named.type_only => {
            let src = named.src.as_deref()?;
            let kind = re_export_kind(&named.specifiers)?;
            (specifier(src), kind, Vec::new(), LoadOrder::Deferred)
        }
        ModuleDecl::ExportAll(all) if !all.type_only => (
            specifier(&all.src),
            ImportKind::Value,
            Vec::new(),
            LoadOrder::Deferred,
        ),
        // `require` runs the module where the statement stands
        ModuleDecl::TsImportEquals(equals) if !equals.is_type_only => {
            let TsModuleRef::TsExternalModuleRef(external) = &equals.module_ref else {
                return None;
            };
            let names = vec![SmolStr::new(equals.id.sym.as_str())];
            (
                specifier(&external.expr),
                ImportKind::Value,
                names,
                LoadOrder::Eager,
            )
        }
        _ => return None,
    };
    Some(ModuleLoad {
        source,
        kind,
        line: line_of(decl.span(), contents, base),
        names,
        load_order,
    })
}

/// The runtime locals an import binds; a `type`-qualified specifier binds
/// none.
fn bound_names(specifiers: &[ImportSpecifier]) -> Vec<SmolStr> {
    specifiers
        .iter()
        .filter(
            |specifier| !matches!(specifier, ImportSpecifier::Named(named) if named.is_type_only),
        )
        .map(|specifier| SmolStr::new(specifier.local().sym.as_str()))
        .collect()
}

/// `import './x'` runs the module, `import { type A } from './x'` loads
/// nothing, and any other import binds a runtime name.
fn import_kind(specifiers: &[ImportSpecifier]) -> Option<ImportKind> {
    if specifiers.is_empty() {
        return Some(ImportKind::SideEffect);
    }
    specifiers
        .iter()
        .any(|specifier| !matches!(specifier, ImportSpecifier::Named(named) if named.is_type_only))
        .then_some(ImportKind::Value)
}

/// `export { type A } from './x'` loads nothing; any other re-export binds.
fn re_export_kind(specifiers: &[ExportSpecifier]) -> Option<ImportKind> {
    (specifiers.is_empty()
        || specifiers.iter().any(
            |specifier| !matches!(specifier, ExportSpecifier::Named(named) if named.is_type_only),
        ))
    .then_some(ImportKind::Value)
}

/// The 1-based line a span starts on.
fn line_of(span: Span, contents: &str, base: BytePos) -> u32 {
    let offset = (span.lo.0 - base.0) as usize;
    let newlines = contents
        .get(..offset)
        .map_or(0, |head| head.bytes().filter(|byte| *byte == b'\n').count());
    u32::try_from(newlines + 1).unwrap_or(u32::MAX)
}
