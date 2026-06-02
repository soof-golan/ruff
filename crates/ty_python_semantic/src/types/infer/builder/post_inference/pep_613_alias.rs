use crate::types::{
    ClassType, DynamicType, KnownInstanceType, SpecialFormType, Type, TypeCheckDiagnostics,
    infer::{InferenceFlags, TypeInferenceBuilder},
    known_instance::UnionTypeInstance,
};
use ruff_python_ast as ast;
use ty_python_core::definition::{
    AnnotatedAssignmentDefinitionKind, AssignmentDefinitionKind, Definition,
};

pub(crate) struct TypeAliasCheckResult<'db> {
    pub(crate) ty: Type<'db>,
    pub(crate) diagnostics: TypeCheckDiagnostics,
    contains_self_type_alias_error: bool,
}

impl TypeAliasCheckResult<'_> {
    pub(crate) fn contains_self_type_alias_error(&self) -> bool {
        self.contains_self_type_alias_error
    }
}

pub(crate) fn check_implicit_alias<'db>(
    assignment: &AssignmentDefinitionKind,
    definition: Definition<'db>,
    value_ty: Type<'db>,
    builder: &TypeInferenceBuilder<'db, '_>,
) -> Option<TypeAliasCheckResult<'db>> {
    let context = &builder.context;
    let value = assignment.value(context.module());

    if !alias_value_contains_self(value, value_ty, definition, builder)
        || !implicit_alias_expression_contains_self(value, builder)
    {
        return None;
    }

    let mut speculative = builder.speculate();
    speculative.typevar_binding_context = Some(definition);
    speculative.context.inference_flags |= InferenceFlags::IN_TYPE_ALIAS;
    let ty = speculative.infer_type_expression(value);
    let contains_self_type_alias_error = speculative.context.contains_self_type_alias_error();
    let diagnostics = speculative.context.finish();
    if !contains_self_type_alias_error {
        return None;
    }

    Some(TypeAliasCheckResult {
        ty: alias_value_from_type_expression(ty, builder),
        diagnostics,
        contains_self_type_alias_error,
    })
}

pub(crate) fn check_pep_613_alias<'db>(
    assignment: &AnnotatedAssignmentDefinitionKind,
    definition: Definition<'db>,
    builder: &TypeInferenceBuilder<'db, '_>,
) -> Option<TypeAliasCheckResult<'db>> {
    let context = &builder.context;

    let value = assignment.value(context.module())?;

    let annotation = assignment.annotation(context.module());
    if !builder
        .file_expression_type(annotation)
        .is_typealias_special_form()
    {
        return None;
    }

    let mut speculative = builder.speculate();

    speculative.typevar_binding_context = Some(definition);
    speculative.context.inference_flags |= InferenceFlags::IN_TYPE_ALIAS;
    let ty = speculative.infer_type_expression(value);
    let contains_self_type_alias_error = speculative.context.contains_self_type_alias_error();
    Some(TypeAliasCheckResult {
        ty: alias_value_from_type_expression(ty, builder),
        diagnostics: speculative.context.finish(),
        contains_self_type_alias_error,
    })
}

fn alias_value_contains_self<'db>(
    value: &ast::Expr,
    value_ty: Type<'db>,
    definition: Definition<'db>,
    builder: &TypeInferenceBuilder<'db, '_>,
) -> bool {
    match value_ty {
        Type::KnownInstance(KnownInstanceType::UnionType(union)) => {
            union_alias_value_contains_self(union, definition, builder)
        }
        Type::Dynamic(DynamicType::Todo(_)) => {
            unsupported_alias_value_contains_self(value, definition, builder)
        }
        Type::SpecialForm(SpecialFormType::TypingSelf) => true,
        _ if value_ty.contains_self(builder.context.db()) => {
            projected_alias_value_contains_self(value_ty, definition, builder)
        }
        _ => false,
    }
}

fn implicit_alias_expression_contains_self(
    value: &ast::Expr,
    builder: &TypeInferenceBuilder<'_, '_>,
) -> bool {
    implicit_alias_expression_shape_contains_self(value, builder, false)
        .is_some_and(|contains_self| contains_self)
}

fn implicit_alias_expression_shape_contains_self(
    value: &ast::Expr,
    builder: &TypeInferenceBuilder<'_, '_>,
    allow_argument_sequence: bool,
) -> Option<bool> {
    match value {
        ast::Expr::Name(_) | ast::Expr::Attribute(_) => {
            let ty = builder.expression_type(value);
            Some(
                matches!(ty, Type::SpecialForm(SpecialFormType::TypingSelf))
                    || ty.contains_self(builder.context.db()),
            )
        }
        ast::Expr::BinOp(binary) if binary.op == ast::Operator::BitOr => {
            let left_contains_self =
                implicit_alias_expression_shape_contains_self(&binary.left, builder, false)?;
            let right_contains_self =
                implicit_alias_expression_shape_contains_self(&binary.right, builder, false)?;
            Some(left_contains_self || right_contains_self)
        }
        ast::Expr::Subscript(subscript) => {
            let value_contains_self =
                implicit_alias_expression_shape_contains_self(&subscript.value, builder, false)?;
            let slice_contains_self = if matches!(
                builder.expression_type(&subscript.value),
                Type::SpecialForm(SpecialFormType::Annotated)
            ) {
                match &*subscript.slice {
                    ast::Expr::Tuple(tuple) => tuple.elts.first().map_or(Some(false), |first| {
                        implicit_alias_expression_shape_contains_self(first, builder, false)
                    })?,
                    slice => implicit_alias_expression_shape_contains_self(slice, builder, false)?,
                }
            } else if matches!(
                builder.expression_type(&subscript.value),
                Type::SpecialForm(SpecialFormType::Literal)
            ) {
                false
            } else {
                implicit_alias_expression_shape_contains_self(&subscript.slice, builder, true)?
            };
            Some(value_contains_self || slice_contains_self)
        }
        ast::Expr::Tuple(tuple) if allow_argument_sequence => {
            tuple.elts.iter().try_fold(false, |contains_self, element| {
                let element_contains_self =
                    implicit_alias_expression_shape_contains_self(element, builder, true)?;
                Some(contains_self || element_contains_self)
            })
        }
        ast::Expr::List(list) if allow_argument_sequence => {
            list.elts.iter().try_fold(false, |contains_self, element| {
                let element_contains_self =
                    implicit_alias_expression_shape_contains_self(element, builder, true)?;
                Some(contains_self || element_contains_self)
            })
        }
        ast::Expr::Starred(starred) if allow_argument_sequence => {
            implicit_alias_expression_shape_contains_self(&starred.value, builder, false)
        }
        ast::Expr::StringLiteral(_) => {
            let ty = builder.expression_type(value);
            Some(
                matches!(ty, Type::SpecialForm(SpecialFormType::TypingSelf))
                    || ty.contains_self(builder.context.db()),
            )
        }
        ast::Expr::NoneLiteral(_) | ast::Expr::EllipsisLiteral(_) => Some(false),
        _ => None,
    }
}

fn union_alias_value_contains_self<'db>(
    union: UnionTypeInstance<'db>,
    definition: Definition<'db>,
    builder: &TypeInferenceBuilder<'db, '_>,
) -> bool {
    let db = builder.context.db();
    let value_expression_types_contain_self =
        union
            .value_expression_types(db)
            .is_ok_and(|value_expression_types| {
                value_expression_types.into_iter().any(|element| {
                    projected_alias_value_contains_self(element, definition, builder)
                })
            });
    let union_type_contains_self = match union.union_type(db).as_ref() {
        Ok(union_type) => union_type.contains_self(db),
        Err(error) => error.contains_typing_self_in_type_alias(),
    };

    value_expression_types_contain_self || union_type_contains_self
}

fn projected_alias_value_contains_self<'db>(
    value_ty: Type<'db>,
    definition: Definition<'db>,
    builder: &TypeInferenceBuilder<'db, '_>,
) -> bool {
    match value_ty {
        Type::KnownInstance(KnownInstanceType::UnionType(union)) => {
            return union_alias_value_contains_self(union, definition, builder);
        }
        Type::SpecialForm(SpecialFormType::TypingSelf) => return true,
        _ => {}
    }

    value_ty
        .in_type_expression(
            builder.context.db(),
            builder.scope(),
            Some(definition),
            builder.inference_flags() | InferenceFlags::IN_TYPE_ALIAS,
        )
        .map_or_else(
            |error| error.contains_typing_self_in_type_alias(),
            |ty| ty.contains_self(builder.context.db()),
        )
}

fn unsupported_alias_value_contains_self<'db>(
    value: &ast::Expr,
    definition: Definition<'db>,
    builder: &TypeInferenceBuilder<'db, '_>,
) -> bool {
    value.as_subscript_expr().is_some_and(|subscript| {
        matches!(
            builder.expression_type(&subscript.value),
            Type::SpecialForm(SpecialFormType::TypingSelf)
        ) || alias_value_contains_self(
            &subscript.slice,
            builder.expression_type(&subscript.slice),
            definition,
            builder,
        )
    })
}

fn alias_value_from_type_expression<'db>(
    ty: Type<'db>,
    builder: &TypeInferenceBuilder<'db, '_>,
) -> Type<'db> {
    let db = builder.context.db();

    match ty {
        Type::Union(_) => Type::KnownInstance(KnownInstanceType::UnionType(
            UnionTypeInstance::new(db, None, Ok(ty)),
        )),
        Type::NominalInstance(instance) => match instance.class(db) {
            ClassType::Generic(alias) => Type::GenericAlias(alias),
            ClassType::NonGeneric(class) => Type::ClassLiteral(class),
        },
        Type::Callable(callable) => Type::KnownInstance(KnownInstanceType::Callable(callable)),
        Type::Dynamic(_) | Type::Divergent(_) | Type::Never => ty,
        _ => ty.to_meta_type(db),
    }
}
