//! Scoped reference collection for parsed TypeScript declarations.

mod collector;
mod visit;

use std::collections::BTreeSet;

use smol_str::SmolStr;
use swc_ecma_ast::{Expr, Pat, Stmt};
use swc_ecma_visit::VisitWith;

use self::collector::ReferenceCollector;

/// References collected from one declaration-owned syntax subtree.
#[derive(Default)]
pub(super) struct References {
    pub(super) referenced: Vec<SmolStr>,
    pub(super) called: Vec<SmolStr>,
    pub(super) dynamic_imports: Vec<SmolStr>,
    pub(super) requires: Vec<SmolStr>,
    /// Whether an `import()` or `require()` takes a target that is not a string
    /// literal.
    pub(super) opaque_loads: bool,
    /// Runtime names read when the subtree runs at module load, outside every
    /// function, method and instance-property body and every type position.
    pub(super) eager: Vec<SmolStr>,
    /// Runtime names read anywhere in the subtree outside a type position.
    pub(super) runtime: Vec<SmolStr>,
    /// Whether a class body in the subtree reads `this` or `super` in code
    /// that runs at module load, so it may call its own deferred members then.
    pub(super) reads_this_at_load: bool,
}

/// Collects references from a class body.
pub(super) fn collect_class(class: &swc_ecma_ast::Class) -> References {
    collect(class)
}

/// Collects references from a function body and signature.
pub(super) fn collect_function(function: &swc_ecma_ast::Function) -> References {
    collect(function)
}

/// Collects references from a variable initializer.
pub(super) fn collect_expression(expression: &Expr) -> References {
    collect(expression)
}

/// Collects references from one top-level statement.
pub(super) fn collect_statement(statement: &Stmt) -> References {
    collect(statement)
}

/// Collects references from a namespace's body, leaving out its name.
pub(super) fn collect_namespace(namespace: &swc_ecma_ast::TsModuleDecl) -> References {
    collect(&namespace.body)
}

/// Collects references from top-level executable statements in source order.
pub(super) fn collect_statements(statements: &[&Stmt]) -> References {
    let mut collector = ReferenceCollector::default();
    for statement in statements {
        statement.visit_with(&mut collector);
    }
    collector.finish()
}

/// Collects scoped references from an interface's declaration members.
pub(super) fn collect_interface(interface: &swc_ecma_ast::TsInterfaceDecl) -> References {
    let mut collector = ReferenceCollector::for_declaration_members();
    collector.visit_type_parameter_scope(interface.type_params.as_deref(), |references| {
        interface.body.visit_with(references);
    });
    collector.finish()
}

/// Collects scoped references from a type alias body.
pub(super) fn collect_type_alias(alias: &swc_ecma_ast::TsTypeAliasDecl) -> References {
    let mut collector = ReferenceCollector::for_declaration_members();
    collector.visit_type_parameter_scope(alias.type_params.as_deref(), |references| {
        alias.type_ann.visit_with(references);
    });
    collector.finish()
}

/// Collects the unique type names appearing in a function signature.
pub(super) fn signature_type_names(function: &swc_ecma_ast::Function) -> BTreeSet<SmolStr> {
    let mut collector = ReferenceCollector::for_declaration_members();
    for parameter in &function.params {
        if let Some(type_ann) = pattern_type_annotation(&parameter.pat) {
            type_ann.type_ann.visit_with(&mut collector);
        }
    }
    if let Some(return_type) = &function.return_type {
        return_type.type_ann.visit_with(&mut collector);
    }
    collector.referenced.into_iter().collect()
}

fn collect<T>(node: &T) -> References
where
    T: VisitWith<ReferenceCollector>,
{
    let mut collector = ReferenceCollector::default();
    node.visit_with(&mut collector);
    collector.finish()
}

/// Returns only the annotation attached to a parameter pattern.
///
/// Assignment defaults and destructuring bodies are deliberately not visited:
/// companion evidence comes from the declared signature, never expressions.
fn pattern_type_annotation(pattern: &Pat) -> Option<&swc_ecma_ast::TsTypeAnn> {
    match pattern {
        Pat::Ident(binding) => binding.type_ann.as_deref(),
        Pat::Array(array) => array.type_ann.as_deref(),
        Pat::Object(object) => object.type_ann.as_deref(),
        Pat::Rest(rest) => rest.type_ann.as_deref(),
        // an assignment default keeps the annotation on its left side; invalid
        // and expression patterns never parse as parameters and carry none
        other => other
            .as_assign()
            .and_then(|assign| pattern_type_annotation(&assign.left)),
    }
}

#[cfg(test)]
mod tests {
    use swc_common::{FileName, SourceMap, sync::Lrc};
    use swc_ecma_ast::{Decl, ModuleItem};
    use swc_ecma_parser::{Parser, StringInput, Syntax, TsSyntax, lexer::Lexer};

    use super::*;

    /// The load-time reads of the first class declared in `source`, parsed
    /// with decorators enabled: the adapter's own parser keeps them disabled,
    /// so this is the only way to reach the dormant decorator handling.
    fn decorated_class_eager(source: &str) -> Result<Vec<SmolStr>, String> {
        let source_map: Lrc<SourceMap> = Lrc::default();
        let file = source_map.new_source_file(
            Lrc::new(FileName::Custom("decorated.ts".to_owned())),
            source.to_owned(),
        );
        let lexer = Lexer::new(
            Syntax::Typescript(TsSyntax {
                decorators: true,
                ..TsSyntax::default()
            }),
            swc_ecma_ast::EsVersion::default(),
            StringInput::from(&*file),
            None,
        );
        let module = Parser::new_from(lexer)
            .parse_module()
            .map_err(|error| format!("{error:?}"))?;
        let class = module
            .body
            .iter()
            .find_map(|item| match item {
                ModuleItem::Stmt(Stmt::Decl(Decl::Class(class))) => Some(&class.class),
                _ => None,
            })
            .ok_or_else(|| "expected a class declaration".to_owned())?;
        let mut eager = collect_class(class).eager;
        eager.sort();
        eager.dedup();
        Ok(eager)
    }

    #[test]
    fn should_read_a_member_decorator_where_its_class_definition_runs() -> Result<(), String> {
        let eager = decorated_class_eager(
            "class C {\n  @logged\n  run() {\n    return later();\n  }\n  constructor(@inject dep: number) {}\n}\n",
        )?;

        assert_eq!(eager, [SmolStr::new("inject"), SmolStr::new("logged")]);
        Ok(())
    }
}
