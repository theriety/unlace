//! Visitor implementation that records references, calls, and dynamic imports.

use smol_str::SmolStr;
use swc_ecma_ast::{ArrowExpr, CallExpr, Callee, Expr, FnExpr, Lit, NewExpr};
use swc_ecma_visit::{Visit, VisitWith};

use super::super::specifier;
use super::collector::{InferBindingCollector, ReferenceCollector};

impl Visit for ReferenceCollector {
    fn visit_ident(&mut self, ident: &swc_ecma_ast::Ident) {
        if self.is_declaration_member_scope
            && self
                .bound_type_names
                .iter()
                .any(|name| name == ident.sym.as_str())
        {
            return;
        }
        let name = SmolStr::new(ident.sym.as_str());
        self.record_read(&name);
        self.referenced.push(name);
    }

    // Code that runs only once called: function, method, constructor and
    // accessor bodies, and instance property values.

    fn visit_function(&mut self, function: &swc_ecma_ast::Function) {
        self.deferred(|references| function.visit_children_with(references));
    }

    fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
        self.deferred(|references| arrow.visit_children_with(references));
    }

    fn visit_constructor(&mut self, constructor: &swc_ecma_ast::Constructor) {
        self.deferred(|references| constructor.visit_children_with(references));
    }

    fn visit_getter_prop(&mut self, getter: &swc_ecma_ast::GetterProp) {
        getter.key.visit_with(self);
        getter.type_ann.visit_with(self);
        self.deferred(|references| getter.body.visit_with(references));
    }

    fn visit_setter_prop(&mut self, setter: &swc_ecma_ast::SetterProp) {
        setter.key.visit_with(self);
        self.deferred(|references| {
            setter.this_param.visit_with(references);
            setter.param.visit_with(references);
            setter.body.visit_with(references);
        });
    }

    fn visit_class_prop(&mut self, property: &swc_ecma_ast::ClassProp) {
        property.key.visit_with(self);
        self.deferred_unless(property.is_static, |references| {
            property.value.visit_with(references);
        });
        property.type_ann.visit_with(self);
        property.decorators.visit_with(self);
    }

    fn visit_private_prop(&mut self, property: &swc_ecma_ast::PrivateProp) {
        property.key.visit_with(self);
        self.deferred_unless(property.is_static, |references| {
            property.value.visit_with(references);
        });
        property.type_ann.visit_with(self);
        property.decorators.visit_with(self);
    }

    fn visit_auto_accessor(&mut self, accessor: &swc_ecma_ast::AutoAccessor) {
        accessor.key.visit_with(self);
        self.deferred_unless(accessor.is_static, |references| {
            accessor.value.visit_with(references);
        });
        accessor.type_ann.visit_with(self);
        accessor.decorators.visit_with(self);
    }

    // A class definition runs its `extends`, computed keys and static members
    // where it stands. A `this` or `super` read there may call any member of
    // the class, deferred bodies included.

    fn visit_class(&mut self, class: &swc_ecma_ast::Class) {
        let enclosing = std::mem::replace(&mut self.class_depth, self.deferred_depth);
        self.class_nesting += 1;
        class.visit_children_with(self);
        self.class_nesting -= 1;
        self.class_depth = enclosing;
    }

    fn visit_this_expr(&mut self, _this: &swc_ecma_ast::ThisExpr) {
        self.record_this();
    }

    fn visit_super(&mut self, _super: &swc_ecma_ast::Super) {
        self.record_this();
    }

    // Dormant: the parser does not enable decorators, so a decorated file
    // fails to parse and the run stops before load-order classification; no
    // decorator reaches this handler today. It is kept, with `class_depth`,
    // so that enabling decorators classifies them correctly at once: a
    // decorator runs where its class definition runs, even when it sits on a
    // method or a constructor parameter inside a deferred body.

    fn visit_decorator(&mut self, decorator: &swc_ecma_ast::Decorator) {
        let depth = std::mem::replace(&mut self.deferred_depth, self.class_depth);
        decorator.visit_children_with(self);
        self.deferred_depth = depth;
    }

    // Type positions, which TypeScript erases: nothing in them runs.

    fn visit_ts_type(&mut self, ty: &swc_ecma_ast::TsType) {
        self.erased(|references| ty.visit_children_with(references));
    }

    fn visit_ts_type_ann(&mut self, annotation: &swc_ecma_ast::TsTypeAnn) {
        self.erased(|references| annotation.visit_children_with(references));
    }

    fn visit_ts_type_param_decl(&mut self, params: &swc_ecma_ast::TsTypeParamDecl) {
        self.erased(|references| params.visit_children_with(references));
    }

    fn visit_ts_type_param_instantiation(
        &mut self,
        arguments: &swc_ecma_ast::TsTypeParamInstantiation,
    ) {
        self.erased(|references| arguments.visit_children_with(references));
    }

    fn visit_ts_expr_with_type_args(&mut self, clause: &swc_ecma_ast::TsExprWithTypeArgs) {
        self.erased(|references| clause.visit_children_with(references));
    }

    fn visit_ts_index_signature(&mut self, signature: &swc_ecma_ast::TsIndexSignature) {
        self.erased(|references| signature.visit_children_with(references));
    }

    fn visit_ts_interface_decl(&mut self, interface: &swc_ecma_ast::TsInterfaceDecl) {
        self.erased(|references| interface.visit_children_with(references));
    }

    fn visit_ts_type_alias_decl(&mut self, alias: &swc_ecma_ast::TsTypeAliasDecl) {
        self.erased(|references| alias.visit_children_with(references));
    }

    fn visit_ts_module_decl(&mut self, module: &swc_ecma_ast::TsModuleDecl) {
        if module.declare {
            self.erased(|references| module.visit_children_with(references));
        } else {
            module.visit_children_with(self);
        }
    }

    fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
        if self.is_declaration_member_scope {
            binding.type_ann.visit_with(self);
        } else {
            binding.visit_children_with(self);
        }
    }

    fn visit_ts_type_param(&mut self, param: &swc_ecma_ast::TsTypeParam) {
        if self.is_declaration_member_scope {
            param.constraint.visit_with(self);
            param.default.visit_with(self);
        } else {
            param.visit_children_with(self);
        }
    }

    fn visit_ts_property_signature(&mut self, property: &swc_ecma_ast::TsPropertySignature) {
        if property.computed || !self.is_declaration_member_scope {
            property.key.visit_with(self);
        }
        property.type_ann.visit_with(self);
    }

    fn visit_ts_getter_signature(&mut self, getter: &swc_ecma_ast::TsGetterSignature) {
        if getter.computed || !self.is_declaration_member_scope {
            getter.key.visit_with(self);
        }
        getter.type_ann.visit_with(self);
    }

    fn visit_ts_setter_signature(&mut self, setter: &swc_ecma_ast::TsSetterSignature) {
        if setter.computed || !self.is_declaration_member_scope {
            setter.key.visit_with(self);
        }
        setter.param.visit_with(self);
    }

    fn visit_ts_method_signature(&mut self, method: &swc_ecma_ast::TsMethodSignature) {
        if !self.is_declaration_member_scope {
            method.visit_children_with(self);
            return;
        }
        if method.computed {
            method.key.visit_with(self);
        }
        self.visit_type_parameter_scope(method.type_params.as_deref(), |references| {
            method.params.visit_with(references);
            method.type_ann.visit_with(references);
        });
    }

    fn visit_ts_call_signature_decl(&mut self, call: &swc_ecma_ast::TsCallSignatureDecl) {
        if !self.is_declaration_member_scope {
            call.visit_children_with(self);
            return;
        }
        self.visit_type_parameter_scope(call.type_params.as_deref(), |references| {
            call.params.visit_with(references);
            call.type_ann.visit_with(references);
        });
    }

    fn visit_ts_construct_signature_decl(
        &mut self,
        constructor: &swc_ecma_ast::TsConstructSignatureDecl,
    ) {
        if !self.is_declaration_member_scope {
            constructor.visit_children_with(self);
            return;
        }
        self.visit_type_parameter_scope(constructor.type_params.as_deref(), |references| {
            constructor.params.visit_with(references);
            constructor.type_ann.visit_with(references);
        });
    }

    fn visit_ts_fn_type(&mut self, function: &swc_ecma_ast::TsFnType) {
        if !self.is_declaration_member_scope {
            function.visit_children_with(self);
            return;
        }
        self.visit_type_parameter_scope(function.type_params.as_deref(), |references| {
            function.params.visit_with(references);
            function.type_ann.visit_with(references);
        });
    }

    fn visit_ts_constructor_type(&mut self, constructor: &swc_ecma_ast::TsConstructorType) {
        if !self.is_declaration_member_scope {
            constructor.visit_children_with(self);
            return;
        }
        self.visit_type_parameter_scope(constructor.type_params.as_deref(), |references| {
            constructor.params.visit_with(references);
            constructor.type_ann.visit_with(references);
        });
    }

    fn visit_ts_tuple_element(&mut self, element: &swc_ecma_ast::TsTupleElement) {
        if self.is_declaration_member_scope {
            element.ty.visit_with(self);
        } else {
            element.visit_children_with(self);
        }
    }

    fn visit_ts_type_predicate(&mut self, predicate: &swc_ecma_ast::TsTypePredicate) {
        if self.is_declaration_member_scope {
            predicate.type_ann.visit_with(self);
        } else {
            predicate.visit_children_with(self);
        }
    }

    fn visit_ts_mapped_type(&mut self, mapped: &swc_ecma_ast::TsMappedType) {
        if !self.is_declaration_member_scope {
            mapped.visit_children_with(self);
            return;
        }
        mapped.type_param.constraint.visit_with(self);
        mapped.type_param.default.visit_with(self);
        let name = SmolStr::new(mapped.type_param.name.sym.as_str());
        self.visit_bound_type_scope([name], |references| {
            mapped.name_type.visit_with(references);
            mapped.type_ann.visit_with(references);
        });
    }

    fn visit_ts_conditional_type(&mut self, conditional: &swc_ecma_ast::TsConditionalType) {
        if !self.is_declaration_member_scope {
            conditional.visit_children_with(self);
            return;
        }
        conditional.check_type.visit_with(self);
        let mut bindings = InferBindingCollector::default();
        conditional.extends_type.visit_with(&mut bindings);
        self.visit_bound_type_scope(bindings.names, |references| {
            conditional.extends_type.visit_with(references);
            conditional.true_type.visit_with(references);
        });
        conditional.false_type.visit_with(self);
    }

    fn visit_ts_qualified_name(&mut self, name: &swc_ecma_ast::TsQualifiedName) {
        if self.is_declaration_member_scope {
            name.left.visit_with(self);
        } else {
            name.visit_children_with(self);
        }
    }

    fn visit_object_pat_prop(&mut self, property: &swc_ecma_ast::ObjectPatProp) {
        if !self.is_declaration_member_scope {
            property.visit_children_with(self);
            return;
        }
        match property {
            swc_ecma_ast::ObjectPatProp::KeyValue(property) => {
                if let swc_ecma_ast::PropName::Computed(key) = &property.key {
                    key.expr.visit_with(self);
                }
                property.value.visit_with(self);
            }
            swc_ecma_ast::ObjectPatProp::Assign(property) => {
                property.key.type_ann.visit_with(self);
                property.value.visit_with(self);
            }
            swc_ecma_ast::ObjectPatProp::Rest(rest) => rest.visit_with(self),
        }
    }

    fn visit_call_expr(&mut self, call: &CallExpr) {
        if let Callee::Expr(callee) = &call.callee
            && let Some(function) = function_expr(callee)
        {
            // an immediately invoked function runs its body where it stands
            self.visit_run_where_called(function);
            self.visit_arguments(&call.args);
            call.type_args.visit_with(self);
            return;
        }
        match &call.callee {
            Callee::Import(_) => {
                if let Some(first) = call.args.first()
                    && let Expr::Lit(Lit::Str(literal)) = first.expr.as_ref()
                {
                    self.dynamic_imports.push(specifier(literal));
                } else {
                    self.opaque_loads = true;
                }
            }
            Callee::Expr(expr) => {
                if let Expr::Ident(ident) = expr.as_ref() {
                    self.called.push(SmolStr::new(ident.sym.as_str()));
                    // `require('./a')` only suppresses advice; it adds no edge
                    if ident.sym.as_str() == "require" {
                        if let Some(first) = call.args.first()
                            && let Expr::Lit(Lit::Str(literal)) = first.expr.as_ref()
                        {
                            self.requires.push(specifier(literal));
                        } else {
                            self.opaque_loads = true;
                        }
                    }
                }
            }
            Callee::Super(_) => {}
        }
        call.callee.visit_with(self);
        self.visit_arguments(&call.args);
        call.type_args.visit_with(self);
    }

    fn visit_new_expr(&mut self, new: &NewExpr) {
        if let Expr::Ident(ident) = new.callee.as_ref() {
            self.called.push(SmolStr::new(ident.sym.as_str()));
        }
        new.callee.visit_with(self);
        if let Some(args) = &new.args {
            self.visit_arguments(args);
        }
        new.type_args.visit_with(self);
    }
}

impl ReferenceCollector {
    /// Visits call arguments. A function or arrow passed straight in is read
    /// where the call runs: `list.map(x => g(x))` at the top level calls `g`
    /// as the module loads. A callee that stores the callback for later
    /// (`setTimeout`, an event listener) is over-reported as load time, which
    /// keeps the conservative reading.
    fn visit_arguments(&mut self, args: &[swc_ecma_ast::ExprOrSpread]) {
        for arg in args {
            match function_expr(&arg.expr) {
                Some(function) => self.visit_run_where_called(function),
                None => arg.visit_with(self),
            }
        }
    }

    /// Visits a function or arrow expression as code that runs at the current
    /// depth rather than once called later.
    fn visit_run_where_called(&mut self, function: FunctionExpr<'_>) {
        match function {
            FunctionExpr::Arrow(arrow) => arrow.visit_children_with(self),
            FunctionExpr::Function(function) => {
                function.ident.visit_with(self);
                function.function.visit_children_with(self);
            }
        }
    }
}

/// A function or arrow expression, the two kinds of code that can run where
/// a call stands.
#[derive(Clone, Copy)]
enum FunctionExpr<'a> {
    Arrow(&'a ArrowExpr),
    Function(&'a FnExpr),
}

/// The function or arrow `expr` is, through any parentheses: the callee of
/// an immediately invoked function, or a callback passed straight to a call.
fn function_expr(expr: &Expr) -> Option<FunctionExpr<'_>> {
    let mut expr = expr;
    while let Expr::Paren(paren) = expr {
        expr = paren.expr.as_ref();
    }
    match expr {
        Expr::Arrow(arrow) => Some(FunctionExpr::Arrow(arrow)),
        Expr::Fn(function) => Some(FunctionExpr::Function(function)),
        _ => None,
    }
}

impl Visit for InferBindingCollector {
    fn visit_ts_infer_type(&mut self, infer: &swc_ecma_ast::TsInferType) {
        self.names
            .push(SmolStr::new(infer.type_param.name.sym.as_str()));
        infer.type_param.constraint.visit_with(self);
        infer.type_param.default.visit_with(self);
    }
}
