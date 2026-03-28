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

/// A row of inline data: one Option<Term> per variable. None = UNDEF.
pub type InlineRow = Vec<Option<Term>>;

/// Inline data from a VALUES clause, lowered to Term values.
#[derive(Debug, Clone)]
pub struct InlineData {
    pub variables: Vec<String>,
    pub rows: Vec<InlineRow>,
}

/// A complete resource-level query plan, lowered from SPARQL AST.
#[derive(Debug, Clone)]
pub struct QueryPlan {
    pub patterns: Vec<QueryPattern>,
    pub select: SelectClause,
    pub modifier: SolutionModifier,
    pub values: Option<InlineData>,
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

        if let Some(ref vd) = self.values {
            for v in &vd.variables {
                if add(v) {
                    vars.push(v.clone());
                }
            }
        }

        vars
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::{Iri, Literal};
    use crate::sparql::ast::{SelectClause, SolutionModifier};

    fn empty_modifier() -> SolutionModifier {
        SolutionModifier::default()
    }

    #[test]
    fn test_collect_variables_resource() {
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("p".into()),
                type_iri: Some(Iri::new("http://example.org/Person")),
                constraints: vec![
                    FieldConstraint {
                        field_name: "name".into(),
                        value: Value::Variable("name".into()),
                    },
                    FieldConstraint {
                        field_name: "age".into(),
                        value: Value::Variable("age".into()),
                    },
                ],
                type_variable: Some("type".into()),
            }],
            select: SelectClause::Star,
            modifier: empty_modifier(),
            values: None,
        };

        assert_eq!(
            plan.collect_variables(),
            vec!["p", "type", "name", "age"]
        );
    }

    #[test]
    fn test_collect_variables_field_scan() {
        let plan = QueryPlan {
            patterns: vec![QueryPattern::FieldScan {
                subject: Subject::Variable("s".into()),
                predicate_var: "p".into(),
                object: Value::Variable("o".into()),
                type_iri: None,
            }],
            select: SelectClause::Star,
            modifier: empty_modifier(),
            values: None,
        };

        assert_eq!(plan.collect_variables(), vec!["s", "p", "o"]);
    }

    #[test]
    fn test_collect_variables_deduplication() {
        // Resource binds ?p and ?name, FieldScan reuses ?p and adds ?pred, ?o
        let plan = QueryPlan {
            patterns: vec![
                QueryPattern::Resource {
                    subject: Subject::Variable("p".into()),
                    type_iri: Some(Iri::new("http://example.org/Person")),
                    constraints: vec![FieldConstraint {
                        field_name: "name".into(),
                        value: Value::Variable("name".into()),
                    }],
                    type_variable: None,
                },
                QueryPattern::FieldScan {
                    subject: Subject::Variable("p".into()),
                    predicate_var: "pred".into(),
                    object: Value::Variable("o".into()),
                    type_iri: Some(Iri::new("http://example.org/Person")),
                },
            ],
            select: SelectClause::Star,
            modifier: empty_modifier(),
            values: None,
        };

        // "p" appears in both patterns but should only appear once
        assert_eq!(
            plan.collect_variables(),
            vec!["p", "name", "pred", "o"]
        );
    }

    #[test]
    fn test_collect_variables_skips_bound() {
        // Bound subject and bound object should not appear in variables
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Bound(Iri::new("http://example.org/person/alice")),
                type_iri: Some(Iri::new("http://example.org/Person")),
                constraints: vec![
                    FieldConstraint {
                        field_name: "name".into(),
                        value: Value::Variable("name".into()),
                    },
                    FieldConstraint {
                        field_name: "age".into(),
                        value: Value::Bound(Term::Literal(Literal::Integer(30))),
                    },
                ],
                type_variable: None,
            }],
            select: SelectClause::Star,
            modifier: empty_modifier(),
            values: None,
        };

        // Only "name" — bound subject and bound age value are excluded
        assert_eq!(plan.collect_variables(), vec!["name"]);
    }

    #[test]
    fn test_collect_variables_empty_plan() {
        let plan = QueryPlan {
            patterns: vec![],
            select: SelectClause::Star,
            modifier: empty_modifier(),
            values: None,
        };

        assert!(plan.collect_variables().is_empty());
    }
}
