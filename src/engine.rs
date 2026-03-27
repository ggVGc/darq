use std::collections::{HashMap, HashSet};

use crate::error::DarqError;
use crate::ir::{FieldConstraint, QueryPattern, QueryPlan, Subject, Value};
use crate::rdf::{Iri, Term, RDF_TYPE};
use crate::resource_store::ResourceStore;
use crate::schema::Schema;
use crate::sparql::ast::*;
use crate::sparql::{self, parser};

/// One solution row: a mapping from variable names to bound terms.
pub type Binding = HashMap<String, Term>;

/// The result of a SELECT query.
#[derive(Debug)]
pub struct QueryResult {
    pub variables: Vec<String>,
    pub rows: Vec<Vec<Option<Term>>>,
}

/// Execute a SPARQL SELECT query against a resource store with a known schema.
pub fn execute(
    query_str: &str,
    schema: &Schema,
    store: &ResourceStore,
) -> Result<QueryResult, DarqError> {
    // 1. Parse
    let mut query = parser::parse(query_str)?;

    // 2. Expand prefixes
    sparql::expand_prefixes(&mut query)?;

    // 3. Validate predicates against schema
    validate_predicates(&query, schema)?;

    // 4. Lower to resource-level IR
    let plan = crate::lower::lower(&query, schema)?;

    // 5. Evaluate plan
    let bindings = evaluate_plan(&plan, store, schema);

    // 6. Apply solution modifiers
    let bindings = apply_modifiers(bindings, &plan.modifier);

    // 7. Project to selected variables
    project(bindings, &plan)
}

/// Validate that all concrete predicates in the query are known to the schema.
fn validate_predicates(query: &SelectQuery, schema: &Schema) -> Result<(), DarqError> {
    for pattern in &query.where_pattern.patterns {
        if let TermOrVariable::Iri(iri) = &pattern.predicate {
            if !schema.is_known_predicate(iri) {
                return Err(DarqError::UnknownPredicate(iri.clone()));
            }
        }
    }
    Ok(())
}

/// Evaluate a resource-level query plan using nested-loop join.
fn evaluate_plan(
    plan: &QueryPlan,
    store: &ResourceStore,
    schema: &Schema,
) -> Vec<Binding> {
    let mut solutions: Vec<Binding> = vec![HashMap::new()];

    for pattern in &plan.patterns {
        let mut next_solutions = Vec::new();

        for existing in &solutions {
            match pattern {
                QueryPattern::Resource {
                    subject,
                    type_iri,
                    constraints,
                    type_variable,
                } => {
                    let instances: Vec<_> = match type_iri {
                        Some(ti) => store.instances_of(ti).iter().collect(),
                        None => store.all_instances().collect(),
                    };

                    for instance in instances {
                        if let Some(binding) = match_resource(
                            subject,
                            constraints,
                            type_variable.as_deref(),
                            instance,
                            existing,
                        ) {
                            let mut merged = existing.clone();
                            merged.extend(binding);
                            next_solutions.push(merged);
                        }
                    }
                }
                QueryPattern::FieldScan {
                    subject,
                    predicate_var,
                    object,
                    type_iri,
                } => {
                    let instances: Vec<_> = match type_iri {
                        Some(ti) => store.instances_of(ti).iter().collect(),
                        None => store.all_instances().collect(),
                    };

                    for instance in instances {
                        let subject_binding =
                            match check_subject(subject, &instance.subject, existing) {
                                Some(b) => b,
                                None => continue,
                            };

                        let fields_for_type =
                            schema.fields_for_type(&instance.type_iri).unwrap_or(&[]);

                        // Synthetic rdf:type field
                        {
                            let pred_term = Term::Iri(Iri::new(RDF_TYPE));
                            let obj_term = Term::Iri(instance.type_iri.clone());
                            if let Some(obj_binding) =
                                check_value(object, &obj_term, existing, &subject_binding)
                            {
                                let mut merged = existing.clone();
                                merged.extend(subject_binding.clone());
                                if check_and_bind_var(
                                    predicate_var,
                                    &pred_term,
                                    existing,
                                    &mut merged,
                                ) {
                                    merged.extend(obj_binding);
                                    next_solutions.push(merged);
                                }
                            }
                        }

                        // Real fields
                        for fd in fields_for_type {
                            if let Some(field_value) = instance.fields.get(fd.name) {
                                let pred_term = Term::Iri(fd.predicate.clone());
                                if let Some(obj_binding) =
                                    check_value(object, field_value, existing, &subject_binding)
                                {
                                    let mut merged = existing.clone();
                                    merged.extend(subject_binding.clone());
                                    if check_and_bind_var(
                                        predicate_var,
                                        &pred_term,
                                        existing,
                                        &mut merged,
                                    ) {
                                        merged.extend(obj_binding);
                                        next_solutions.push(merged);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        solutions = next_solutions;
    }

    solutions
}

/// Try to match a resource instance against a Resource pattern.
fn match_resource(
    subject: &Subject,
    constraints: &[FieldConstraint],
    type_variable: Option<&str>,
    instance: &crate::resource_store::ResourceInstance,
    existing: &Binding,
) -> Option<Binding> {
    let mut new_bindings = Binding::new();

    // Check/bind subject
    match subject {
        Subject::Variable(name) => {
            let term = Term::Iri(instance.subject.clone());
            if let Some(existing_val) = existing.get(name) {
                if *existing_val != term {
                    return None;
                }
            } else if let Some(already) = new_bindings.get(name) {
                if *already != term {
                    return None;
                }
            } else {
                new_bindings.insert(name.clone(), term);
            }
        }
        Subject::Bound(iri) => {
            if *iri != instance.subject {
                return None;
            }
        }
    }

    // Bind type variable if requested
    if let Some(tv) = type_variable {
        let term = Term::Iri(instance.type_iri.clone());
        if let Some(existing_val) = existing.get(tv) {
            if *existing_val != term {
                return None;
            }
        } else if let Some(already) = new_bindings.get(tv) {
            if *already != term {
                return None;
            }
        } else {
            new_bindings.insert(tv.to_string(), term);
        }
    }

    // Check all field constraints
    for constraint in constraints {
        let field_value = instance.fields.get(&constraint.field_name)?;

        match &constraint.value {
            Value::Bound(expected) => {
                if field_value != expected {
                    return None;
                }
            }
            Value::Variable(name) => {
                if let Some(existing_val) = existing.get(name) {
                    if existing_val != field_value {
                        return None;
                    }
                } else if let Some(already) = new_bindings.get(name) {
                    if already != field_value {
                        return None;
                    }
                } else {
                    new_bindings.insert(name.clone(), field_value.clone());
                }
            }
        }
    }

    Some(new_bindings)
}

/// Check if a subject pattern matches an instance subject.
fn check_subject(
    subject: &Subject,
    actual: &Iri,
    existing: &Binding,
) -> Option<Binding> {
    let mut binding = Binding::new();
    match subject {
        Subject::Variable(name) => {
            let term = Term::Iri(actual.clone());
            if let Some(existing_val) = existing.get(name) {
                if *existing_val != term {
                    return None;
                }
            } else {
                binding.insert(name.clone(), term);
            }
        }
        Subject::Bound(iri) => {
            if *iri != *actual {
                return None;
            }
        }
    }
    Some(binding)
}

/// Check/bind a variable against a term. Returns true if consistent.
fn check_and_bind_var(
    var_name: &str,
    term: &Term,
    existing: &Binding,
    merged: &mut Binding,
) -> bool {
    if let Some(existing_val) = existing.get(var_name) {
        if *existing_val != *term {
            return false;
        }
    } else if let Some(already) = merged.get(var_name) {
        if *already != *term {
            return false;
        }
    } else {
        merged.insert(var_name.to_string(), term.clone());
    }
    true
}

/// Check a Value constraint against an actual term.
fn check_value(
    value: &Value,
    actual: &Term,
    existing: &Binding,
    subject_binding: &Binding,
) -> Option<Binding> {
    let mut binding = Binding::new();
    match value {
        Value::Bound(expected) => {
            if expected != actual {
                return None;
            }
        }
        Value::Variable(name) => {
            if let Some(existing_val) = existing.get(name) {
                if *existing_val != *actual {
                    return None;
                }
            } else if let Some(subj_val) = subject_binding.get(name) {
                if *subj_val != *actual {
                    return None;
                }
            } else {
                binding.insert(name.clone(), actual.clone());
            }
        }
    }
    Some(binding)
}

/// Project bindings to the requested variables.
fn project(
    bindings: Vec<Binding>,
    plan: &QueryPlan,
) -> Result<QueryResult, DarqError> {
    let variables = match &plan.select {
        SelectClause::Variables(vars) => vars.iter().map(|v| v.0.clone()).collect(),
        SelectClause::Star => plan.collect_variables(),
    };

    let rows = bindings
        .into_iter()
        .map(|binding| {
            variables
                .iter()
                .map(|var| binding.get(var).cloned())
                .collect()
        })
        .collect();

    Ok(QueryResult { variables, rows })
}

/// Apply DISTINCT, ORDER BY, OFFSET, LIMIT.
fn apply_modifiers(mut bindings: Vec<Binding>, modifier: &SolutionModifier) -> Vec<Binding> {
    if modifier.distinct {
        let mut seen = HashSet::new();
        bindings.retain(|b| {
            let mut sorted: Vec<_> = b.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);
            seen.insert(format!("{:?}", sorted))
        });
    }

    if !modifier.order_by.is_empty() {
        bindings.sort_by(|a, b| {
            for cond in &modifier.order_by {
                let va = a.get(&cond.variable.0);
                let vb = b.get(&cond.variable.0);
                let ord = va.cmp(&vb);
                let ord = match cond.direction {
                    OrderDirection::Ascending => ord,
                    OrderDirection::Descending => ord.reverse(),
                };
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            std::cmp::Ordering::Equal
        });
    }

    if let Some(offset) = modifier.offset {
        if offset < bindings.len() {
            bindings = bindings.into_iter().skip(offset).collect();
        } else {
            bindings.clear();
        }
    }

    if let Some(limit) = modifier.limit {
        bindings.truncate(limit);
    }

    bindings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::{Iri, Literal, Term};
    use crate::resource_store::ResourceStore;
    use crate::schema::{FieldDescriptor, FieldType, Resource, Schema};

    struct Person {
        id: String,
        name: String,
        age: i64,
    }

    impl Resource for Person {
        fn rdf_type() -> Iri {
            Iri::new("http://example.org/Person")
        }

        fn subject_iri(&self) -> Iri {
            Iri::new(format!("http://example.org/person/{}", self.id))
        }

        fn field_descriptors() -> Vec<FieldDescriptor> {
            vec![
                FieldDescriptor {
                    predicate: Iri::new("http://example.org/name"),
                    name: "name",
                    field_type: FieldType::Literal,
                },
                FieldDescriptor {
                    predicate: Iri::new("http://example.org/age"),
                    name: "age",
                    field_type: FieldType::Literal,
                },
            ]
        }

        fn field_values(&self) -> Vec<Term> {
            vec![self.name.clone().into(), self.age.into()]
        }
    }

    fn setup() -> (Schema, ResourceStore) {
        let mut schema = Schema::new();
        schema.register::<Person>();

        let mut store = ResourceStore::new();
        store.load(&Person { id: "alice".into(), name: "Alice".into(), age: 30 });
        store.load(&Person { id: "bob".into(), name: "Bob".into(), age: 25 });

        (schema, store)
    }

    #[test]
    fn test_basic_select_star() {
        let (schema, store) = setup();
        let result = execute(
            "PREFIX ex: <http://example.org/> SELECT * WHERE { ?p a ex:Person . ?p ex:name ?name }",
            &schema,
            &store,
        ).unwrap();

        assert_eq!(result.rows.len(), 2);
        assert!(result.variables.contains(&"name".to_string()));
    }

    #[test]
    fn test_select_specific_vars() {
        let (schema, store) = setup();
        let result = execute(
            "PREFIX ex: <http://example.org/> SELECT ?name ?age WHERE { ?p a ex:Person . ?p ex:name ?name . ?p ex:age ?age }",
            &schema,
            &store,
        ).unwrap();

        assert_eq!(result.variables, vec!["name", "age"]);
        assert_eq!(result.rows.len(), 2);
    }

    #[test]
    fn test_order_by() {
        let (schema, store) = setup();
        let result = execute(
            "PREFIX ex: <http://example.org/> SELECT ?name WHERE { ?p ex:name ?name } ORDER BY ?name",
            &schema,
            &store,
        ).unwrap();

        assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Alice".into()))));
        assert_eq!(result.rows[1][0], Some(Term::Literal(Literal::String("Bob".into()))));
    }

    #[test]
    fn test_order_by_desc() {
        let (schema, store) = setup();
        let result = execute(
            "PREFIX ex: <http://example.org/> SELECT ?name WHERE { ?p ex:name ?name } ORDER BY DESC(?name)",
            &schema,
            &store,
        ).unwrap();

        assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Bob".into()))));
        assert_eq!(result.rows[1][0], Some(Term::Literal(Literal::String("Alice".into()))));
    }

    #[test]
    fn test_limit() {
        let (schema, store) = setup();
        let result = execute(
            "PREFIX ex: <http://example.org/> SELECT ?name WHERE { ?p ex:name ?name } ORDER BY ?name LIMIT 1",
            &schema,
            &store,
        ).unwrap();

        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Alice".into()))));
    }

    #[test]
    fn test_offset() {
        let (schema, store) = setup();
        let result = execute(
            "PREFIX ex: <http://example.org/> SELECT ?name WHERE { ?p ex:name ?name } ORDER BY ?name OFFSET 1",
            &schema,
            &store,
        ).unwrap();

        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Bob".into()))));
    }

    #[test]
    fn test_unknown_predicate_errors() {
        let (schema, store) = setup();
        let result = execute(
            "PREFIX ex: <http://example.org/> SELECT ?x WHERE { ?p ex:email ?x }",
            &schema,
            &store,
        );

        assert!(matches!(result, Err(DarqError::UnknownPredicate(_))));
    }

    #[test]
    fn test_join_across_patterns() {
        let (schema, store) = setup();
        let result = execute(
            "PREFIX ex: <http://example.org/> SELECT ?name ?age WHERE { ?p ex:name ?name . ?p ex:age ?age } ORDER BY ?name",
            &schema,
            &store,
        ).unwrap();

        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Alice".into()))));
        assert_eq!(result.rows[0][1], Some(Term::Literal(Literal::Integer(30))));
        assert_eq!(result.rows[1][0], Some(Term::Literal(Literal::String("Bob".into()))));
        assert_eq!(result.rows[1][1], Some(Term::Literal(Literal::Integer(25))));
    }
}
