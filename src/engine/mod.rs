#[cfg(feature = "postgres")]
pub mod postgres;
pub mod sql;

use std::collections::{HashMap, HashSet};

use crate::error::DarqError;
use crate::ir::{InlineData, QueryPlan};
use crate::rdf::Term;
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

/// Trait for query plan evaluation backends.
///
/// Implementations must apply `plan.modifier` (DISTINCT, ORDER BY, LIMIT,
/// OFFSET) before returning.  The [`apply_modifiers`] helper in this
/// module provides a ready-made implementation.
///
/// When multiple plans are provided (ambiguous-type alternatives), the engine
/// must union their results and apply modifiers on the combined set.
pub trait Engine {
    fn evaluate_plans(&self, plans: &[QueryPlan], schema: &Schema) -> Result<Vec<Binding>, DarqError>;
}

/// Execute a SPARQL SELECT query using the given engine and schema.
pub fn execute(
    query_str: &str,
    schema: &Schema,
    engine: &dyn Engine,
) -> Result<QueryResult, DarqError> {
    // 1. Parse
    let query = parser::parse(query_str)?;

    // 2. Validate SELECT variables are bound
    sparql::validate_select_variables(&query)?;

    // 4. Validate predicates against schema
    validate_predicates(&query, schema)?;

    // 5. Lower to resource-level IR (may produce multiple plans for ambiguous types)
    let plans = crate::lower::lower(&query, schema)?;

    // 6. Evaluate plan(s) — engine handles union for multiple plans
    let bindings = engine.evaluate_plans(&plans, schema)?;

    // 7. Project to selected variables
    let mut result = project(bindings, &plans[0])?;

    // 8. Apply DISTINCT after projection so deduplication uses only selected
    //    variables. OFFSET and LIMIT follow DISTINCT per the SPARQL spec.
    if plans[0].modifier.distinct {
        let mut seen = HashSet::new();
        result.rows.retain(|row| seen.insert(row.clone()));

        if let Some(offset) = plans[0].modifier.offset {
            if offset < result.rows.len() {
                result.rows = result.rows.into_iter().skip(offset).collect();
            } else {
                result.rows.clear();
            }
        }
        if let Some(limit) = plans[0].modifier.limit {
            result.rows.truncate(limit);
        }
    }

    Ok(result)
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
        SelectClause::Variables(vars) => vars.iter().map(|v| v.as_str().to_owned()).collect(),
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

/// Apply ORDER BY and, when DISTINCT is not active, OFFSET and LIMIT.
///
/// DISTINCT, OFFSET, and LIMIT are applied post-projection by `execute()`
/// when DISTINCT is requested, so this helper skips them in that case.
pub fn apply_modifiers(mut bindings: Vec<Binding>, modifier: &SolutionModifier) -> Vec<Binding> {
    if !modifier.order_by.is_empty() {
        bindings.sort_by(|a, b| {
            for cond in &modifier.order_by {
                let va = a.get(cond.variable.as_str());
                let vb = b.get(cond.variable.as_str());
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

    if !modifier.distinct {
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
    }

    bindings
}

/// Join existing solutions with inline VALUES data.
pub fn join_with_values(solutions: Vec<Binding>, values: &InlineData) -> Vec<Binding> {
    let mut result = Vec::new();

    for existing in &solutions {
        for row in &values.rows {
            let mut compatible = true;
            let mut new_bindings = Binding::new();

            for (i, var_name) in values.variables.iter().enumerate() {
                match &row[i] {
                    None => {
                        // UNDEF: do not constrain or bind
                    }
                    Some(term) => {
                        if let Some(existing_val) = existing.get(var_name) {
                            if *existing_val != *term {
                                compatible = false;
                                break;
                            }
                        } else {
                            new_bindings.insert(var_name.clone(), term.clone());
                        }
                    }
                }
            }

            if compatible {
                let mut merged = existing.clone();
                merged.extend(new_bindings);
                result.push(merged);
            }
        }
    }

    result
}
