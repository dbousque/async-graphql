use crate::parser::ast::{
    Document, FragmentDefinition, FragmentSpread, OperationDefinition, Type, VariableDefinition,
};
use crate::registry::TypeName;
use crate::validation::utils::{operation_name, Scope};
use crate::validation::visitor::{Visitor, VisitorContext};
use crate::{Pos, Positioned, Value};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct VariableInAllowedPosition<'a> {
    spreads: HashMap<Scope<'a>, HashSet<&'a str>>,
    variable_usages: HashMap<Scope<'a>, Vec<(&'a str, Pos, TypeName<'a>)>>,
    variable_defs: HashMap<Scope<'a>, Vec<&'a Positioned<VariableDefinition>>>,
    current_scope: Option<Scope<'a>>,
}

impl<'a> VariableInAllowedPosition<'a> {
    fn collect_incorrect_usages(
        &self,
        from: &Scope<'a>,
        var_defs: &[&'a Positioned<VariableDefinition>],
        ctx: &mut VisitorContext<'a>,
        visited: &mut HashSet<Scope<'a>>,
    ) {
        if visited.contains(from) {
            return;
        }

        visited.insert(from.clone());

        if let Some(usages) = self.variable_usages.get(from) {
            for (var_name, usage_pos, var_type) in usages {
                if let Some(def) = var_defs.iter().find(|def| def.name.node == *var_name) {
                    let expected_type = match (&def.default_value, &def.var_type.node) {
                        (Some(_), Type::List(_)) => def.var_type.to_string() + "!",
                        (Some(_), Type::Named(_)) => def.var_type.to_string() + "!",
                        (_, _) => def.var_type.to_string(),
                    };

                    if !var_type.is_subtype(&TypeName::create(&expected_type)) {
                        ctx.report_error(
                            vec![def.position(), *usage_pos],
                            format!(
                            "Variable \"{}\" of type \"{}\" used in position expecting type \"{}\"",
                            var_name, var_type, expected_type
                        ),
                        );
                    }
                }
            }
        }

        if let Some(spreads) = self.spreads.get(from) {
            for spread in spreads {
                self.collect_incorrect_usages(&Scope::Fragment(spread), var_defs, ctx, visited);
            }
        }
    }
}

impl<'a> Visitor<'a> for VariableInAllowedPosition<'a> {
    fn exit_document(&mut self, ctx: &mut VisitorContext<'a>, _doc: &'a Document) {
        for (op_scope, var_defs) in &self.variable_defs {
            self.collect_incorrect_usages(op_scope, var_defs, ctx, &mut HashSet::new());
        }
    }

    fn enter_operation_definition(
        &mut self,
        _ctx: &mut VisitorContext<'a>,
        operation_definition: &'a Positioned<OperationDefinition>,
    ) {
        let (op_name, _) = operation_name(operation_definition);
        self.current_scope = Some(Scope::Operation(op_name));
    }

    fn enter_fragment_definition(
        &mut self,
        _ctx: &mut VisitorContext<'a>,
        fragment_definition: &'a Positioned<FragmentDefinition>,
    ) {
        self.current_scope = Some(Scope::Fragment(fragment_definition.name.node));
    }

    fn enter_variable_definition(
        &mut self,
        _ctx: &mut VisitorContext<'a>,
        variable_definition: &'a Positioned<VariableDefinition>,
    ) {
        if let Some(ref scope) = self.current_scope {
            self.variable_defs
                .entry(scope.clone())
                .or_insert_with(Vec::new)
                .push(variable_definition);
        }
    }

    fn enter_fragment_spread(
        &mut self,
        _ctx: &mut VisitorContext<'a>,
        fragment_spread: &'a Positioned<FragmentSpread>,
    ) {
        if let Some(ref scope) = self.current_scope {
            self.spreads
                .entry(scope.clone())
                .or_insert_with(HashSet::new)
                .insert(fragment_spread.fragment_name.node);
        }
    }

    fn enter_input_value(
        &mut self,
        _ctx: &mut VisitorContext<'a>,
        pos: Pos,
        expected_type: &Option<TypeName<'a>>,
        value: &'a Value,
    ) {
        if let Value::Variable(name) = value {
            if let Some(expected_type) = expected_type {
                if let Some(scope) = &self.current_scope {
                    self.variable_usages
                        .entry(scope.clone())
                        .or_insert_with(Vec::new)
                        .push((name, pos, *expected_type));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::test_harness::{expect_fails_rule, expect_passes_rule};

    pub fn factory<'a>() -> VariableInAllowedPosition<'a> {
        VariableInAllowedPosition::default()
    }

    #[test]
    fn boolean_into_boolean() {
        expect_passes_rule(
            factory,
            r#"
          query Query($booleanArg: Boolean)
          {
            complicatedArgs {
              booleanArgField(booleanArg: $booleanArg)
            }
          }
        "#,
        );
    }

    #[test]
    fn boolean_into_boolean_within_fragment() {
        expect_passes_rule(
            factory,
            r#"
          fragment booleanArgFrag on ComplicatedArgs {
            booleanArgField(booleanArg: $booleanArg)
          }
          query Query($booleanArg: Boolean)
          {
            complicatedArgs {
              ...booleanArgFrag
            }
          }
        "#,
        );

        expect_passes_rule(
            factory,
            r#"
          query Query($booleanArg: Boolean)
          {
            complicatedArgs {
              ...booleanArgFrag
            }
          }
          fragment booleanArgFrag on ComplicatedArgs {
            booleanArgField(booleanArg: $booleanArg)
          }
        "#,
        );
    }

    #[test]
    fn non_null_boolean_into_boolean() {
        expect_passes_rule(
            factory,
            r#"
          query Query($nonNullBooleanArg: Boolean!)
          {
            complicatedArgs {
              booleanArgField(booleanArg: $nonNullBooleanArg)
            }
          }
        "#,
        );
    }

    #[test]
    fn non_null_boolean_into_boolean_within_fragment() {
        expect_passes_rule(
            factory,
            r#"
          fragment booleanArgFrag on ComplicatedArgs {
            booleanArgField(booleanArg: $nonNullBooleanArg)
          }
          query Query($nonNullBooleanArg: Boolean!)
          {
            complicatedArgs {
              ...booleanArgFrag
            }
          }
        "#,
        );
    }

    #[test]
    fn int_into_non_null_int_with_default() {
        expect_passes_rule(
            factory,
            r#"
          query Query($intArg: Int = 1)
          {
            complicatedArgs {
              nonNullIntArgField(nonNullIntArg: $intArg)
            }
          }
        "#,
        );
    }

    #[test]
    fn string_list_into_string_list() {
        expect_passes_rule(
            factory,
            r#"
          query Query($stringListVar: [String])
          {
            complicatedArgs {
              stringListArgField(stringListArg: $stringListVar)
            }
          }
        "#,
        );
    }

    #[test]
    fn non_null_string_list_into_string_list() {
        expect_passes_rule(
            factory,
            r#"
          query Query($stringListVar: [String!])
          {
            complicatedArgs {
              stringListArgField(stringListArg: $stringListVar)
            }
          }
        "#,
        );
    }

    #[test]
    fn string_into_string_list_in_item_position() {
        expect_passes_rule(
            factory,
            r#"
          query Query($stringVar: String)
          {
            complicatedArgs {
              stringListArgField(stringListArg: [$stringVar])
            }
          }
        "#,
        );
    }

    #[test]
    fn non_null_string_into_string_list_in_item_position() {
        expect_passes_rule(
            factory,
            r#"
          query Query($stringVar: String!)
          {
            complicatedArgs {
              stringListArgField(stringListArg: [$stringVar])
            }
          }
        "#,
        );
    }

    #[test]
    fn complex_input_into_complex_input() {
        expect_passes_rule(
            factory,
            r#"
          query Query($complexVar: ComplexInput)
          {
            complicatedArgs {
              complexArgField(complexArg: $complexVar)
            }
          }
        "#,
        );
    }

    #[test]
    fn complex_input_into_complex_input_in_field_position() {
        expect_passes_rule(
            factory,
            r#"
          query Query($boolVar: Boolean = false)
          {
            complicatedArgs {
              complexArgField(complexArg: {requiredArg: $boolVar})
            }
          }
        "#,
        );
    }

    #[test]
    fn non_null_boolean_into_non_null_boolean_in_directive() {
        expect_passes_rule(
            factory,
            r#"
          query Query($boolVar: Boolean!)
          {
            dog @include(if: $boolVar)
          }
        "#,
        );
    }

    #[test]
    fn boolean_in_non_null_in_directive_with_default() {
        expect_passes_rule(
            factory,
            r#"
          query Query($boolVar: Boolean = false)
          {
            dog @include(if: $boolVar)
          }
        "#,
        );
    }

    #[test]
    fn int_into_non_null_int() {
        expect_fails_rule(
            factory,
            r#"
          query Query($intArg: Int) {
            complicatedArgs {
              nonNullIntArgField(nonNullIntArg: $intArg)
            }
          }
        "#,
        );
    }

    #[test]
    fn int_into_non_null_int_within_fragment() {
        expect_fails_rule(
            factory,
            r#"
          fragment nonNullIntArgFieldFrag on ComplicatedArgs {
            nonNullIntArgField(nonNullIntArg: $intArg)
          }
          query Query($intArg: Int) {
            complicatedArgs {
              ...nonNullIntArgFieldFrag
            }
          }
        "#,
        );
    }

    #[test]
    fn int_into_non_null_int_within_nested_fragment() {
        expect_fails_rule(
            factory,
            r#"
          fragment outerFrag on ComplicatedArgs {
            ...nonNullIntArgFieldFrag
          }
          fragment nonNullIntArgFieldFrag on ComplicatedArgs {
            nonNullIntArgField(nonNullIntArg: $intArg)
          }
          query Query($intArg: Int) {
            complicatedArgs {
              ...outerFrag
            }
          }
        "#,
        );
    }

    #[test]
    fn string_over_boolean() {
        expect_fails_rule(
            factory,
            r#"
          query Query($stringVar: String) {
            complicatedArgs {
              booleanArgField(booleanArg: $stringVar)
            }
          }
        "#,
        );
    }

    #[test]
    fn string_into_string_list() {
        expect_fails_rule(
            factory,
            r#"
          query Query($stringVar: String) {
            complicatedArgs {
              stringListArgField(stringListArg: $stringVar)
            }
          }
        "#,
        );
    }

    #[test]
    fn boolean_into_non_null_boolean_in_directive() {
        expect_fails_rule(
            factory,
            r#"
          query Query($boolVar: Boolean) {
            dog @include(if: $boolVar)
          }
        "#,
        );
    }

    #[test]
    fn string_into_non_null_boolean_in_directive() {
        expect_fails_rule(
            factory,
            r#"
          query Query($stringVar: String) {
            dog @include(if: $stringVar)
          }
        "#,
        );
    }
}
