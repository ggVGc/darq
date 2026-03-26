use std::collections::{HashMap, HashSet};

use crate::error::DarqError;
use crate::rdf::{Iri, Literal, Term, RDF_TYPE};
use crate::schema::Schema;
use crate::sparql::ast::*;
use crate::sparql::{self, parser};
use crate::store::{self, Binding, IriPattern, TermPattern, TripleStore};

/// The result of a SELECT query.
#[derive(Debug)]
pub struct QueryResult {
    pub variables: Vec<String>,
    pub rows: Vec<Vec<Option<Term>>>,
}

/// Execute a SPARQL SELECT query against a store with a known schema.
pub fn execute(
    query_str: &str,
    schema: &Schema,
    store: &TripleStore,
) -> Result<QueryResult, DarqError> {
    // 1. Parse
    let mut query = parser::parse(query_str)?;

    // 2. Expand prefixes
    sparql::expand_prefixes(&mut query)?;

    // 3. Validate predicates against schema
    validate_predicates(&query, schema)?;

    // 4. Evaluate basic graph pattern
    let bindings = evaluate_bgp(&query.where_pattern, store, schema);

    // 5. Apply solution modifiers
    let bindings = apply_modifiers(bindings, &query.modifier);

    // 6. Project to selected variables
    project(bindings, &query.select, &query.where_pattern)
}

/// Validate that all concrete predicates in the query are known to the schema.
fn validate_predicates(query: &SelectQuery, schema: &Schema) -> Result<(), DarqError> {
    for pattern in &query.where_pattern.patterns {
        if let Some(iri) = as_concrete_iri(&pattern.predicate) {
            if !schema.is_known_predicate(iri) {
                return Err(DarqError::UnknownPredicate(iri.clone()));
            }
        }
    }
    Ok(())
}

/// Extract a concrete IRI from a TermOrVariable, if it is one.
fn as_concrete_iri(tov: &TermOrVariable) -> Option<&Iri> {
    match tov {
        TermOrVariable::Iri(iri) => Some(iri),
        TermOrVariable::RdfType => None, // rdf:type is always valid
        _ => None,
    }
}

/// Convert an AST node in subject/predicate position to an IriPattern.
fn to_iri_pattern(tov: &TermOrVariable, binding: &Binding) -> IriPattern {
    match tov {
        TermOrVariable::Variable(Variable(name)) => {
            if let Some(term) = binding.get(name) {
                match term {
                    Term::Iri(iri) => IriPattern::Bound(iri.clone()),
                    _ => IriPattern::Variable(name.clone()), // shouldn't happen
                }
            } else {
                IriPattern::Variable(name.clone())
            }
        }
        TermOrVariable::Iri(iri) => IriPattern::Bound(iri.clone()),
        TermOrVariable::RdfType => IriPattern::Bound(Iri::new(RDF_TYPE)),
        TermOrVariable::Literal(_) => {
            unreachable!("literal cannot appear in subject/predicate position")
        }
        TermOrVariable::PrefixedName { .. } => {
            unreachable!("prefixed names should be expanded before evaluation")
        }
    }
}

/// Convert an AST node in object position to a TermPattern.
fn to_term_pattern(tov: &TermOrVariable, binding: &Binding) -> TermPattern {
    match tov {
        TermOrVariable::Variable(Variable(name)) => {
            if let Some(term) = binding.get(name) {
                TermPattern::Bound(term.clone())
            } else {
                TermPattern::Variable(name.clone())
            }
        }
        TermOrVariable::Iri(iri) => TermPattern::Bound(Term::Iri(iri.clone())),
        TermOrVariable::RdfType => TermPattern::Bound(Term::Iri(Iri::new(RDF_TYPE))),
        TermOrVariable::Literal(lit) => TermPattern::Bound(ast_lit_to_term(lit)),
        TermOrVariable::PrefixedName { .. } => {
            unreachable!("prefixed names should be expanded before evaluation")
        }
    }
}

fn ast_lit_to_term(lit: &AstLiteral) -> Term {
    match lit {
        AstLiteral::String(s) => Term::Literal(Literal::String(s.clone())),
        AstLiteral::Integer(n) => Term::Literal(Literal::Integer(*n)),
        AstLiteral::Boolean(b) => Term::Literal(Literal::Boolean(*b)),
    }
}

/// If the predicate is an unbound variable, return its name.
fn unbound_predicate_var(tov: &TermOrVariable, binding: &Binding) -> Option<String> {
    if let TermOrVariable::Variable(Variable(name)) = tov {
        if !binding.contains_key(name) {
            return Some(name.clone());
        }
    }
    None
}

/// Evaluate a basic graph pattern using nested-loop join.
/// Variable predicates are expanded over all known predicates in the schema.
fn evaluate_bgp(
    pattern: &GroupGraphPattern,
    store: &TripleStore,
    schema: &Schema,
) -> Vec<Binding> {
    let mut solutions: Vec<Binding> = vec![HashMap::new()];

    for triple_pattern in &pattern.patterns {
        let mut next_solutions = Vec::new();

        for existing in &solutions {
            if let Some(var_name) = unbound_predicate_var(&triple_pattern.predicate, existing) {
                // Variable predicate: expand over all known predicates, union results
                for pred_iri in schema.known_predicates() {
                    let store_pattern = store::TriplePattern {
                        subject: to_iri_pattern(&triple_pattern.subject, existing),
                        predicate: IriPattern::Bound(pred_iri.clone()),
                        object: to_term_pattern(&triple_pattern.object, existing),
                    };

                    for mut new_binding in store.match_pattern(&store_pattern) {
                        new_binding.insert(var_name.clone(), Term::Iri(pred_iri.clone()));
                        let mut merged = existing.clone();
                        merged.extend(new_binding);
                        next_solutions.push(merged);
                    }
                }
            } else {
                // Concrete or already-bound predicate: normal path
                let store_pattern = store::TriplePattern {
                    subject: to_iri_pattern(&triple_pattern.subject, existing),
                    predicate: to_iri_pattern(&triple_pattern.predicate, existing),
                    object: to_term_pattern(&triple_pattern.object, existing),
                };

                for new_binding in store.match_pattern(&store_pattern) {
                    let mut merged = existing.clone();
                    merged.extend(new_binding);
                    next_solutions.push(merged);
                }
            }
        }

        solutions = next_solutions;
    }

    solutions
}

/// Apply DISTINCT, ORDER BY, OFFSET, LIMIT.
fn apply_modifiers(mut bindings: Vec<Binding>, modifier: &SolutionModifier) -> Vec<Binding> {
    // DISTINCT
    if modifier.distinct {
        let mut seen = HashSet::new();
        bindings.retain(|b| {
            let mut sorted: Vec<_> = b.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);
            seen.insert(format!("{:?}", sorted))
        });
    }

    // ORDER BY
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

    // OFFSET
    if let Some(offset) = modifier.offset {
        if offset < bindings.len() {
            bindings = bindings.into_iter().skip(offset).collect();
        } else {
            bindings.clear();
        }
    }

    // LIMIT
    if let Some(limit) = modifier.limit {
        bindings.truncate(limit);
    }

    bindings
}

/// Project bindings to the requested variables.
fn project(
    bindings: Vec<Binding>,
    select: &SelectClause,
    pattern: &GroupGraphPattern,
) -> Result<QueryResult, DarqError> {
    let variables = match select {
        SelectClause::Variables(vars) => {
            vars.iter().map(|v| v.0.clone()).collect()
        }
        SelectClause::Star => {
            // Collect all variables mentioned in the pattern
            let mut vars = Vec::new();
            let mut seen = HashSet::new();
            for tp in &pattern.patterns {
                collect_variables(&tp.subject, &mut vars, &mut seen);
                collect_variables(&tp.predicate, &mut vars, &mut seen);
                collect_variables(&tp.object, &mut vars, &mut seen);
            }
            vars
        }
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

fn collect_variables(tov: &TermOrVariable, vars: &mut Vec<String>, seen: &mut HashSet<String>) {
    if let TermOrVariable::Variable(Variable(name)) = tov {
        if seen.insert(name.clone()) {
            vars.push(name.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::{Iri, Literal, Term};
    use crate::schema::{FieldDescriptor, Resource, Schema};
    use crate::store::TripleStore;

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
                },
                FieldDescriptor {
                    predicate: Iri::new("http://example.org/age"),
                    name: "age",
                },
            ]
        }

        fn field_values(&self) -> Vec<Term> {
            vec![self.name.clone().into(), self.age.into()]
        }
    }

    fn setup() -> (Schema, TripleStore) {
        let mut schema = Schema::new();
        schema.register::<Person>();

        let mut store = TripleStore::new();
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
        // Alice, 30
        assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Alice".into()))));
        assert_eq!(result.rows[0][1], Some(Term::Literal(Literal::Integer(30))));
        // Bob, 25
        assert_eq!(result.rows[1][0], Some(Term::Literal(Literal::String("Bob".into()))));
        assert_eq!(result.rows[1][1], Some(Term::Literal(Literal::Integer(25))));
    }
}
