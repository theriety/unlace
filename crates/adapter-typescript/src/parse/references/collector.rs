//! Reference collector state and scope handling for declaration subtrees.

use smol_str::SmolStr;
use swc_ecma_visit::VisitWith;

use super::References;

/// A [`Visit`] that gathers referenced identifiers, invoked identifiers, and
/// dynamic-import literals reachable from a single top-level declaration's body.
///
/// Beside the flat lists it tells apart when each runtime name is read: at
/// module load (outside every function, method and instance-property body) or
/// only once called. Names in type positions are left out of both, because
/// TypeScript erases them.
#[derive(Default)]
pub(super) struct ReferenceCollector {
    /// Whether declaration-member keys and binders should be excluded.
    pub(super) is_declaration_member_scope: bool,
    /// Type-level names bound by the declaration scopes currently being visited.
    pub(super) bound_type_names: Vec<SmolStr>,
    /// Identifiers referenced within the visited subtree.
    pub(super) referenced: Vec<SmolStr>,
    /// Identifiers invoked as a call or `new` target within the subtree.
    pub(super) called: Vec<SmolStr>,
    /// Literal specifiers of `import('...')` calls within the subtree.
    pub(super) dynamic_imports: Vec<SmolStr>,
    /// Literal specifiers of `require('...')` calls within the subtree.
    pub(super) requires: Vec<SmolStr>,
    /// Whether an `import()` or `require()` within the subtree has a computed
    /// target.
    pub(super) opaque_loads: bool,
    /// How many function, method or instance-property bodies enclose the
    /// current node: their code runs when called, not when the module loads.
    pub(super) deferred_depth: u32,
    /// The deferred depth at which the innermost enclosing class definition
    /// runs, where its decorators would run too.
    pub(super) class_depth: u32,
    /// How many class definitions enclose the current node.
    pub(super) class_nesting: u32,
    /// How many type positions enclose the current node: TypeScript erases
    /// them, so nothing in them runs at all.
    pub(super) erased_depth: u32,
    /// Runtime names read when the module loads: outside every deferred body
    /// and type position.
    pub(super) eager: Vec<SmolStr>,
    /// Runtime names read anywhere outside a type position, including inside
    /// deferred bodies.
    pub(super) runtime: Vec<SmolStr>,
    /// Whether code in a class body that runs at module load (a static block,
    /// a static property value, a computed key) reads `this` or `super`.
    pub(super) reads_this_at_load: bool,
}

impl ReferenceCollector {
    /// A collector for type-level declaration members, which are erased whole.
    pub(super) fn for_declaration_members() -> Self {
        Self {
            is_declaration_member_scope: true,
            erased_depth: 1,
            ..Self::default()
        }
    }

    /// Records a runtime name read at the current position.
    pub(super) fn record_read(&mut self, name: &SmolStr) {
        if self.erased_depth > 0 {
            return;
        }
        if self.deferred_depth == 0 {
            self.eager.push(name.clone());
        }
        self.runtime.push(name.clone());
    }

    /// Records a `this` or `super` read at the current position: in a class
    /// body at load time it may call any of the class's own members.
    pub(super) fn record_this(&mut self) {
        if self.class_nesting > 0 && self.deferred_depth == 0 && self.erased_depth == 0 {
            self.reads_this_at_load = true;
        }
    }

    /// Visits code that runs only when called, such as a function body.
    pub(super) fn deferred(&mut self, visit: impl FnOnce(&mut Self)) {
        self.deferred_depth += 1;
        visit(self);
        self.deferred_depth -= 1;
    }

    /// Visits code that runs only when called unless `runs_at_load` holds,
    /// as for a class property value that is static or not.
    pub(super) fn deferred_unless(&mut self, runs_at_load: bool, visit: impl FnOnce(&mut Self)) {
        if runs_at_load {
            visit(self);
        } else {
            self.deferred(visit);
        }
    }

    /// Visits a type position, which TypeScript erases.
    pub(super) fn erased(&mut self, visit: impl FnOnce(&mut Self)) {
        self.erased_depth += 1;
        visit(self);
        self.erased_depth -= 1;
    }

    pub(super) fn finish(self) -> References {
        References {
            referenced: self.referenced,
            called: self.called,
            dynamic_imports: self.dynamic_imports,
            requires: self.requires,
            opaque_loads: self.opaque_loads,
            eager: self.eager,
            runtime: self.runtime,
            reads_this_at_load: self.reads_this_at_load,
        }
    }

    pub(super) fn visit_type_parameter_scope(
        &mut self,
        params: Option<&swc_ecma_ast::TsTypeParamDecl>,
        visit: impl FnOnce(&mut Self),
    ) {
        let Some(params) = params else {
            visit(self);
            return;
        };
        let names = params
            .params
            .iter()
            .map(|param| SmolStr::new(param.name.sym.as_str()));
        self.visit_bound_type_scope(names, |references| {
            for param in &params.params {
                param.constraint.visit_with(references);
                param.default.visit_with(references);
            }
            visit(references);
        });
    }

    pub(super) fn visit_bound_type_scope(
        &mut self,
        names: impl IntoIterator<Item = SmolStr>,
        visit: impl FnOnce(&mut Self),
    ) {
        let mut scoped = Self {
            is_declaration_member_scope: self.is_declaration_member_scope,
            bound_type_names: self.bound_type_names.clone(),
            deferred_depth: self.deferred_depth,
            class_depth: self.class_depth,
            class_nesting: self.class_nesting,
            erased_depth: self.erased_depth,
            ..Self::default()
        };
        scoped.bound_type_names.extend(names);
        visit(&mut scoped);
        self.referenced.extend(scoped.referenced);
        self.called.extend(scoped.called);
        self.dynamic_imports.extend(scoped.dynamic_imports);
        self.requires.extend(scoped.requires);
        self.opaque_loads |= scoped.opaque_loads;
        self.eager.extend(scoped.eager);
        self.runtime.extend(scoped.runtime);
        self.reads_this_at_load |= scoped.reads_this_at_load;
    }
}

#[derive(Default)]
pub(super) struct InferBindingCollector {
    pub(super) names: Vec<SmolStr>,
}
