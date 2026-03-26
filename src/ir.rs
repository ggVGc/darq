use std::collections::HashSet;

use crate::rdf::{Iri, Term};
use crate::sparql::ast::{SelectClause, SolutionModifier};

/// How a pattern references a resource's identity (subject position).
#[derive(Debug, Clone)]
pub enum Subject {
    Variable(String),
    Bound(Iri),
}

/// How a field value is constrained.
#[derive(Debug, Clone)]
pub enum Value {
    Variable(String),
    Bound(Term),
}

/// A constraint on a named field of a resource.
#[derive(Debug, Clone)]
pub struct FieldConstraint {
    pub field_name: String,
    pub value: Value,
}

/// A pattern in the resource-level query plan.
#[derive(Debug, Clone)]
pub enum QueryPattern {
    /// Match instances of a resource type where all field constraints hold.
    /// Groups what were previously multiple triple patterns into one check.
    Resource {
        subject: Subject,
        /// None = scan all types (for `?s a ?type` with no concrete predicates).
        type_iri: Option<Iri>,
        constraints: Vec<FieldConstraint>,
        /// If Some, bind the type IRI to this variable (from `?s a ?type`).
        type_variable: Option<String>,
    },

    /// Iterate all fields of matching resources, one row per field.
    /// Used for variable-predicate patterns like `?s ?p ?o`.
    /// Includes synthetic rdf:type field in iteration.
    FieldScan {
        subject: Subject,
        predicate_var: String,
        object: Value,
        /// If known, restrict to this type. None = scan all types.
        type_iri: Option<Iri>,
    },
}

/// A complete resource-level query plan, lowered from SPARQL AST.
#[derive(Debug, Clone)]
pub struct QueryPlan {
    pub patterns: Vec<QueryPattern>,
    pub select: SelectClause,
    pub modifier: SolutionModifier,
}

impl QueryPlan {
    /// Collect all variable names introduced by patterns, in first-appearance order.
    /// Used for `SELECT *`.
    pub fn collect_variables(&self) -> Vec<String> {
        let mut vars = Vec::new();
        let mut seen = HashSet::new();

        let mut add = |name: &String| -> bool {
            if seen.insert(name.clone()) {
                true
            } else {
                false
            }
        };

        for pattern in &self.patterns {
            match pattern {
                QueryPattern::Resource {
                    subject,
                    constraints,
                    type_variable,
                    ..
                } => {
                    if let Subject::Variable(v) = subject {
                        if add(v) {
                            vars.push(v.clone());
                        }
                    }
                    if let Some(tv) = type_variable {
                        if add(tv) {
                            vars.push(tv.clone());
                        }
                    }
                    for c in constraints {
                        if let Value::Variable(v) = &c.value {
                            if add(v) {
                                vars.push(v.clone());
                            }
                        }
                    }
                }
                QueryPattern::FieldScan {
                    subject,
                    predicate_var,
                    object,
                    ..
                } => {
                    if let Subject::Variable(v) = subject {
                        if add(v) {
                            vars.push(v.clone());
                        }
                    }
                    if add(predicate_var) {
                        vars.push(predicate_var.clone());
                    }
                    if let Value::Variable(v) = object {
                        if add(v) {
                            vars.push(v.clone());
                        }
                    }
                }
            }
        }

        vars
    }
}
