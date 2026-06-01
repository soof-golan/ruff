use crate::types::{
    SpecialFormType, Type, TypeCheckDiagnostics, TypeContext,
    infer::{InferenceFlags, TypeInferenceBuilder},
};
use ty_python_core::definition::{
    AnnotatedAssignmentDefinitionKind, AssignmentDefinitionKind, Definition,
};

pub(crate) struct TypeAliasCheckResult<'db> {
    pub(crate) ty: Type<'db>,
    pub(crate) diagnostics: TypeCheckDiagnostics,
}

pub(crate) fn check_implicit_alias<'db>(
    assignment: &AssignmentDefinitionKind,
    definition: Definition<'db>,
    builder: &TypeInferenceBuilder<'db, '_>,
) -> Option<TypeAliasCheckResult<'db>> {
    let context = &builder.context;
    let value = assignment.value(context.module());

    let mut speculative = builder.speculate();
    speculative.typevar_binding_context = Some(definition);
    let ty = speculative.infer_type_expression(value);
    let contains_self = ty.contains_self(context.db())
        || matches!(ty, Type::SpecialForm(SpecialFormType::TypingSelf));
    let is_implicit_self_alias = (speculative.context.finish().is_empty() && contains_self)
        || is_bare_typing_self(value, builder);
    if is_implicit_self_alias && !ty.is_todo() {
        let mut speculative = builder.speculate();
        speculative.typevar_binding_context = Some(definition);
        speculative.context.inference_flags |= InferenceFlags::IN_TYPE_ALIAS;
        let ty = speculative.infer_type_expression(value);
        Some(TypeAliasCheckResult {
            ty,
            diagnostics: speculative.context.finish(),
        })
    } else {
        None
    }
}

fn is_bare_typing_self<'db>(
    value: &ruff_python_ast::Expr,
    builder: &TypeInferenceBuilder<'db, '_>,
) -> bool {
    if !value.is_name_expr() {
        return false;
    }

    let mut speculative = builder.speculate();
    matches!(
        speculative.infer_expression(value, TypeContext::default()),
        Type::SpecialForm(SpecialFormType::TypingSelf)
    )
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
    Some(TypeAliasCheckResult {
        ty,
        diagnostics: speculative.context.finish(),
    })
}
