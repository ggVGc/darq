pub mod memory;
#[cfg(feature = "postgres")]
pub mod postgres;
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
///
/// Implementations must apply `plan.modifier` (DISTINCT, ORDER BY, LIMIT,
/// OFFSET) before returning.  The [`apply_modifiers`] helper provides a
/// ready-made in-memory implementation.
pub trait Engine {
    fn evaluate_plan(&self, plan: &QueryPlan, schema: &Schema) -> Result<Vec<Binding>, DarqError>;
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

    // 5. Lower to resource-level IR (may produce multiple plans for ambiguous types)
    let plans = crate::lower::lower(&query, schema)?;

    if plans.len() == 1 {
        // Single plan — fast path, same as before.
        let plan = &plans[0];
        let bindings = engine.evaluate_plan(plan, schema)?;
        let mut result = project(bindings, plan)?;

        if plan.modifier.distinct {
            let mut seen = HashSet::new();
            result.rows.retain(|row| seen.insert(row.clone()));

            if let Some(offset) = plan.modifier.offset {
                if offset < result.rows.len() {
                    result.rows = result.rows.into_iter().skip(offset).collect();
                } else {
                    result.rows.clear();
                }
            }
            if let Some(limit) = plan.modifier.limit {
                result.rows.truncate(limit);
            }
        }

        Ok(result)
    } else {
        // Multiple plans — evaluate each without LIMIT/OFFSET, union results.
        let modifier = plans[0].modifier.clone();

        let mut all_bindings = Vec::new();
        for plan in &plans {
            let mut eval_plan = plan.clone();
            eval_plan.modifier = SolutionModifier::default();
            let bindings = engine.evaluate_plan(&eval_plan, schema)?;
            all_bindings.extend(bindings);
        }

        // Sort combined bindings by ORDER BY before projection.
        if !modifier.order_by.is_empty() {
            all_bindings.sort_by(|a, b| {
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

        let mut result = project(all_bindings, &plans[0])?;

        // Apply DISTINCT, OFFSET, LIMIT on the combined result.
        if modifier.distinct {
            let mut seen = HashSet::new();
            result.rows.retain(|row| seen.insert(row.clone()));
        }
        if let Some(offset) = modifier.offset {
            if offset < result.rows.len() {
                result.rows = result.rows.into_iter().skip(offset).collect();
            } else {
                result.rows.clear();
            }
        }
        if let Some(limit) = modifier.limit {
            result.rows.truncate(limit);
        }

        Ok(result)
    }
}

/// Validate that all concrete predicates in the query are known to the schema.
fn validate_predicates(query: &SelectQuery, schema: &Schema) -> Result<(), DarqError> {
    validate_predicates_in_ggp(&query.where_pattern, schema)
}

fn validate_predicates_in_ggp(
    ggp: &GroupGraphPattern,
    schema: &Schema,
) -> Result<(), DarqError> {
    for pattern in &ggp.patterns {
        if let TermOrVariable::Iri(iri) = &pattern.predicate {
            if !schema.is_known_predicate(iri) {
                return Err(DarqError::UnknownPredicate(iri.clone()));
            }
        }
    }
    for filter in &ggp.filters {
        match filter {
            Filter::NotExists(inner) => validate_predicates_in_ggp(inner, schema)?,
        }
    }
    Ok(())
}

/// Project bindings to the requested variables.
fn project(bindings: Vec<Binding>, plan: &QueryPlan) -> Result<QueryResult, DarqError> {
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
