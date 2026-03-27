pub mod memory;
pub mod sql;

use std::collections::{HashMap, HashSet};

use crate::error::DarqError;
use crate::ir::QueryPlan;
use crate::rdf::Term;
use crate::schema::Schema;
use crate::sparql::ast::*;
use crate::sparql::{self, parser};

pub use memory::InMemoryEngine;

/// One solution row: a mapping from variable names to bound terms.
pub type Binding = HashMap<String, Term>;

/// The result of a SELECT query.
#[derive(Debug)]
pub struct QueryResult {
    pub variables: Vec<String>,
    pub rows: Vec<Vec<Option<Term>>>,
}

/// Trait for query plan evaluation backends.
pub trait Engine {
    fn evaluate_plan(
        &self,
        plan: &QueryPlan,
        schema: &Schema,
    ) -> Result<Vec<Binding>, DarqError>;
}

/// Execute a SPARQL SELECT query using the given engine and schema.
pub fn execute(
    query_str: &str,
    schema: &Schema,
    engine: &dyn Engine,
) -> Result<QueryResult, DarqError> {
    // 1. Parse
    let mut query = parser::parse(query_str)?;

    // 2. Expand prefixes
    sparql::expand_prefixes(&mut query)?;

    // 3. Validate SELECT variables are bound
    sparql::validate_select_variables(&query)?;

    // 4. Validate predicates against schema
    validate_predicates(&query, schema)?;

    // 5. Lower to resource-level IR
    let plan = crate::lower::lower(&query, schema)?;

    // 6. Evaluate plan
    let bindings = engine.evaluate_plan(&plan, schema)?;

    // 7. Apply solution modifiers
    let bindings = apply_modifiers(bindings, &plan.modifier);

    // 8. Project to selected variables
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
