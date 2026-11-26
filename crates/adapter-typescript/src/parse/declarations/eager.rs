//! The names a module reads while it loads, closed over its own declarations.
//!
//! An import runs at load time when the importer reads one of its bindings
//! before every module of a cycle has finished loading: in a top-level
//! statement or initialiser, a class `extends`, decorator, static member or
//! computed key, an enum initialiser, a namespace body or the default-export
//! expression. A read inside a function, method or instance-property body
//! waits until that code is called, and a read in a type position is erased.
//!
//! The closure makes this sound across local declarations: a local declaration
//! read at load time may run its deferred code then (`const y = f();` calls the
//! local `f`), so every name it reads anywhere counts as read at load time too,
//! until nothing new is added. A class whose load-time code (a static block,
//! a static property value, a computed key) reads `this` or `super` counts as
//! read at load time, so every member it may call that way joins in. Other
//! calls through dynamic dispatch (`obj[key]()`, a callback a library runs
//! while the module loads) are not seen.

use std::collections::{BTreeMap, BTreeSet};

use smol_str::SmolStr;
use swc_ecma_ast::{
    Decl, DefaultDecl, Expr, Module, ModuleDecl, ModuleItem, ObjectPatProp, Pat, PropName, Stmt,
    TsEntityName, TsModuleName, TsModuleRef,
};

use super::super::references::{
    References, collect_class, collect_expression, collect_function, collect_namespace,
    collect_statement,
};

/// Every name `module` reads while it loads, after the intra-file closure.
pub(super) fn load_time_names(module: &Module) -> BTreeSet<SmolStr> {
    let mut reads = LoadReads::default();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(decl)) => reads.declaration(decl),
            ModuleItem::Stmt(statement) => reads.at_load(collect_statement(statement)),
            ModuleItem::ModuleDecl(decl) => reads.module_declaration(decl),
        }
    }
    reads.close()
}

/// The reads of one module, split into those its load runs and those each
/// local declaration runs once it is used.
#[derive(Default)]
struct LoadReads {
    /// Names read while the module loads, before the closure.
    load_time: BTreeSet<SmolStr>,
    /// Every runtime name each local declaration reads, deferred or not.
    by_local: BTreeMap<SmolStr, Vec<SmolStr>>,
}

impl LoadReads {
    /// Records code that runs at load time and binds no local. A class in it
    /// that reads `this` at load may run any of its members, so all of its
    /// runtime reads count.
    fn at_load(&mut self, references: References) {
        self.load_time.extend(references.eager);
        if references.reads_this_at_load {
            self.load_time.extend(references.runtime);
        }
    }

    /// Records a local declaration: its load-time reads run now, the rest
    /// once the local is used. A class whose load-time code reads `this` or
    /// `super` uses itself as it loads.
    fn local(&mut self, name: SmolStr, references: References) {
        self.load_time.extend(references.eager);
        if references.reads_this_at_load {
            self.load_time.insert(name.clone());
        }
        self.by_local
            .entry(name)
            .or_default()
            .extend(references.runtime);
    }

    /// Records a top-level declaration; an ambient (`declare`) one is erased.
    fn declaration(&mut self, decl: &Decl) {
        match decl {
            Decl::Class(class) if !class.declare => {
                self.local(name_of(&class.ident), collect_class(&class.class));
            }
            Decl::Fn(function) if !function.declare => {
                self.local(
                    name_of(&function.ident),
                    collect_function(&function.function),
                );
            }
            Decl::Var(var) if !var.declare => {
                for declarator in &var.decls {
                    let Some(init) = declarator.init.as_deref() else {
                        continue;
                    };
                    let mut names = Vec::new();
                    bound_names(&declarator.name, &mut names);
                    let mut references = collect_expression(init);
                    // destructuring defaults and computed keys run as the
                    // declaration does
                    let mut pattern_code = Vec::new();
                    pattern_expressions(&declarator.name, &mut pattern_code);
                    for expression in pattern_code {
                        let part = collect_expression(expression);
                        references.eager.extend(part.eager);
                        references.runtime.extend(part.runtime);
                        references.reads_this_at_load |= part.reads_this_at_load;
                    }
                    for name in names {
                        self.by_local
                            .entry(name)
                            .or_default()
                            .extend(references.runtime.iter().cloned());
                    }
                    self.at_load(references);
                }
            }
            Decl::Using(using) => {
                for init in using
                    .decls
                    .iter()
                    .filter_map(|declarator| declarator.init.as_deref())
                {
                    self.at_load(collect_expression(init));
                }
            }
            Decl::TsEnum(ts_enum) if !ts_enum.declare => {
                let mut references = References::default();
                for init in ts_enum
                    .members
                    .iter()
                    .filter_map(|member| member.init.as_deref())
                {
                    let member = collect_expression(init);
                    references.eager.extend(member.eager);
                    references.runtime.extend(member.runtime);
                }
                self.local(name_of(&ts_enum.id), references);
            }
            Decl::TsModule(namespace) if !namespace.declare => {
                let references = collect_namespace(namespace);
                match &namespace.id {
                    TsModuleName::Ident(ident) => self.local(name_of(ident), references),
                    TsModuleName::Str(_) => self.at_load(references),
                }
            }
            _ => {}
        }
    }

    /// Records an export or `import x = ...` statement; imports and
    /// re-exports read nothing.
    fn module_declaration(&mut self, decl: &ModuleDecl) {
        match decl {
            ModuleDecl::ExportDecl(export) => self.declaration(&export.decl),
            ModuleDecl::ExportDefaultDecl(default) => match &default.decl {
                DefaultDecl::Class(class) => {
                    let name = class.ident.as_ref().map_or_else(default_name, name_of);
                    self.local(name, collect_class(&class.class));
                }
                DefaultDecl::Fn(function) => {
                    let name = function.ident.as_ref().map_or_else(default_name, name_of);
                    self.local(name, collect_function(&function.function));
                }
                DefaultDecl::TsInterfaceDecl(_) => {}
            },
            // `export default expr` and `export = expr` evaluate `expr` as the
            // module loads
            ModuleDecl::ExportDefaultExpr(default) => {
                self.at_load(collect_expression(&default.expr));
            }
            ModuleDecl::TsExportAssignment(assignment) => {
                self.at_load(collect_expression(&assignment.expr));
            }
            // `import x = NS.y` reads `NS` as the module loads
            ModuleDecl::TsImportEquals(equals) if !equals.is_type_only => {
                if let TsModuleRef::TsEntityName(entity) = &equals.module_ref {
                    let root = entity_root(entity);
                    self.load_time.insert(root.clone());
                    self.by_local
                        .entry(name_of(&equals.id))
                        .or_default()
                        .push(root);
                }
            }
            _ => {}
        }
    }

    /// The load-time names closed over the local declarations they use.
    fn close(self) -> BTreeSet<SmolStr> {
        let mut names = self.load_time;
        let mut pending: Vec<SmolStr> = names.iter().cloned().collect();
        while let Some(name) = pending.pop() {
            for read in self.by_local.get(&name).into_iter().flatten() {
                if names.insert(read.clone()) {
                    pending.push(read.clone());
                }
            }
        }
        names
    }
}

fn name_of(ident: &swc_ecma_ast::Ident) -> SmolStr {
    SmolStr::new(ident.sym.as_str())
}

fn default_name() -> SmolStr {
    SmolStr::new("default")
}

/// The leftmost identifier of `A.B.C`.
fn entity_root(entity: &TsEntityName) -> SmolStr {
    match entity {
        TsEntityName::Ident(ident) => name_of(ident),
        TsEntityName::TsQualifiedName(qualified) => entity_root(&qualified.left),
    }
}

/// The locals a binding pattern declares: an identifier names itself, and
/// every other pattern names what its sub-patterns name.
fn bound_names(pattern: &Pat, out: &mut Vec<SmolStr>) {
    if let Pat::Ident(binding) = pattern {
        out.push(name_of(&binding.id));
    }
    match pattern {
        Pat::Array(array) => {
            for element in array.elems.iter().flatten() {
                bound_names(element, out);
            }
        }
        Pat::Object(object) => {
            for property in &object.props {
                match property {
                    ObjectPatProp::KeyValue(property) => bound_names(&property.value, out),
                    ObjectPatProp::Assign(property) => out.push(name_of(&property.key.id)),
                    ObjectPatProp::Rest(rest) => bound_names(&rest.arg, out),
                }
            }
        }
        Pat::Rest(rest) => bound_names(&rest.arg, out),
        Pat::Assign(assign) => bound_names(&assign.left, out),
        // leaves: an identifier was named above, and neither an assignment
        // target nor an invalid pattern binds a local.
        Pat::Ident(_) | Pat::Invalid(_) | Pat::Expr(_) => {}
    }
}

/// The expressions a binding pattern evaluates as it binds: default values
/// and computed keys, nested patterns included.
fn pattern_expressions<'a>(pattern: &'a Pat, out: &mut Vec<&'a Expr>) {
    match pattern {
        Pat::Array(array) => {
            for element in array.elems.iter().flatten() {
                pattern_expressions(element, out);
            }
        }
        Pat::Object(object) => {
            for property in &object.props {
                match property {
                    ObjectPatProp::KeyValue(property) => {
                        if let PropName::Computed(key) = &property.key {
                            out.push(&key.expr);
                        }
                        pattern_expressions(&property.value, out);
                    }
                    ObjectPatProp::Assign(property) => out.extend(property.value.as_deref()),
                    ObjectPatProp::Rest(rest) => pattern_expressions(&rest.arg, out),
                }
            }
        }
        Pat::Rest(rest) => pattern_expressions(&rest.arg, out),
        Pat::Assign(assign) => {
            out.push(&assign.right);
            pattern_expressions(&assign.left, out);
        }
        Pat::Ident(_) | Pat::Invalid(_) | Pat::Expr(_) => {}
    }
}
