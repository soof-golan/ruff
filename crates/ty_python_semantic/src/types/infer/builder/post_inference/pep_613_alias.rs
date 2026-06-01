use crate::types::{
    SpecialFormType, Type, TypeCheckDiagnostics,
    infer::{InferenceFlags, TypeInferenceBuilder},
};
use ty_python_core::definition::{
    AnnotatedAssignmentDefinitionKind, AssignmentDefinitionKind, Definition,
};

pub(crate) fn check_implicit_alias<'db>(
    assignment: &AssignmentDefinitionKind,
    definition: Definition<'db>,
    value_ty: Type<'db>,
    builder: &TypeInferenceBuilder<'db, '_>,
) -> Option<TypeCheckDiagnostics> {
    let context = &builder.context;
    let value = assignment.value(context.module());

    let contains_self = value_ty.contains_self(context.db())
        || matches!(value_ty, Type::SpecialForm(SpecialFormType::TypingSelf));
    if !contains_self
        || value_ty
            .in_type_expression(
                context.db(),
                builder.scope(),
                Some(definition),
                InferenceFlags::empty(),
            )
            .is_err()
    {
        return None;
    }

    let mut speculative = builder.speculate();
    speculative.typevar_binding_context = Some(definition);
    speculative.infer_type_expression(value);
    if !speculative.context.finish().is_empty() {
        return None;
    }

    let mut speculative = builder.speculate();
    speculative.typevar_binding_context = Some(definition);
    speculative.context.inference_flags |= InferenceFlags::IN_TYPE_ALIAS;
    speculative.infer_type_expression(value);
    Some(speculative.context.finish())
}

pub(crate) fn check_pep_613_alias<'db>(
    assignment: &AnnotatedAssignmentDefinitionKind,
    definition: Definition<'db>,
    builder: &TypeInferenceBuilder<'db, '_>,
) -> Option<TypeCheckDiagnostics> {
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
    speculative.infer_type_expression(value);
    Some(speculative.context.finish())
}
