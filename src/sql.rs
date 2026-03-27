use std::collections::HashMap;

use crate::error::DarqError;
use crate::ir::{QueryPattern, QueryPlan, Subject, Value};
use crate::rdf::{Iri, Literal, Term};
use crate::schema::Schema;
use crate::sparql::ast::{OrderDirection, SelectClause};

/// How a SPARQL variable maps to a SQL expression.
#[derive(Clone)]
enum SqlExpr {
    /// A column reference: `"p<idx>"."<column>"`.
    Column { pattern_idx: usize, column: String },
    /// A constant SQL literal (e.g., a type IRI string).
    Constant(String),
}

impl SqlExpr {
    fn to_sql(&self) -> String {
        match self {
            SqlExpr::Column { pattern_idx, column } => {
                format!("\"p{}\".\"{}\"", pattern_idx, column)
            }
            SqlExpr::Constant(s) => s.clone(),
        }
    }
}

/// Translate a resource-level query plan into a SQL string.
///
/// Assumes one table per resource type, named after the local part of the
/// type IRI (e.g. `http://example.org/Person` → `"Person"`), with a
/// `_subject` column for the resource IRI and one column per field.
pub fn to_sql(plan: &QueryPlan, schema: &Schema) -> Result<String, DarqError> {
    let mut bindings: HashMap<String, SqlExpr> = HashMap::new();
    let mut from_parts: Vec<String> = Vec::new();
    let mut where_parts: Vec<String> = Vec::new();

    for (i, pattern) in plan.patterns.iter().enumerate() {
        let alias = format!("p{}", i);

        match pattern {
            QueryPattern::Resource {
                subject,
                type_iri,
                constraints,
                type_variable,
            } => {
                let mut join_conds: Vec<String> = Vec::new();

                // Determine the table source.
                let source = if let Some(ti) = type_iri {
                    format!("\"{}\"", table_name(ti))
                } else {
                    // No concrete type — UNION ALL over all registered types.
                    let parts: Vec<String> = schema
                        .known_types()
                        .map(|ti| {
                            format!(
                                "SELECT \"_subject\", '{}' AS \"_type\" FROM \"{}\"",
                                ti.0,
                                table_name(ti)
                            )
                        })
                        .collect();
                    if parts.is_empty() {
                        "(SELECT NULL AS \"_subject\", NULL AS \"_type\" WHERE FALSE)".to_string()
                    } else {
                        format!("({})", parts.join(" UNION ALL "))
                    }
                };

                // Subject binding.
                match subject {
                    Subject::Variable(v) => {
                        if let Some(existing) = bindings.get(v) {
                            join_conds.push(format!(
                                "\"{}\".\"_subject\" = {}",
                                alias,
                                existing.to_sql()
                            ));
                        } else {
                            bindings.insert(
                                v.clone(),
                                SqlExpr::Column {
                                    pattern_idx: i,
                                    column: "_subject".into(),
                                },
                            );
                        }
                    }
                    Subject::Bound(iri) => {
                        where_parts.push(format!(
                            "\"{}\".\"_subject\" = '{}'",
                            alias, iri.0
                        ));
                    }
                }

                // Type variable binding.
                if let Some(tv) = type_variable {
                    if let Some(ti) = type_iri {
                        // Known type — bind to a constant.
                        bindings
                            .insert(tv.clone(), SqlExpr::Constant(format!("'{}'", ti.0)));
                    } else {
                        // Unknown type — bind to the _type column from the UNION.
                        if let Some(existing) = bindings.get(tv) {
                            join_conds.push(format!(
                                "\"{}\".\"_type\" = {}",
                                alias,
                                existing.to_sql()
                            ));
                        } else {
                            bindings.insert(
                                tv.clone(),
                                SqlExpr::Column {
                                    pattern_idx: i,
                                    column: "_type".into(),
                                },
                            );
                        }
                    }
                }

                // Field constraint bindings.
                for c in constraints {
                    match &c.value {
                        Value::Variable(v) => {
                            if let Some(existing) = bindings.get(v) {
                                join_conds.push(format!(
                                    "\"{}\".\"{}\" = {}",
                                    alias,
                                    c.field_name,
                                    existing.to_sql()
                                ));
                            } else {
                                bindings.insert(
                                    v.clone(),
                                    SqlExpr::Column {
                                        pattern_idx: i,
                                        column: c.field_name.clone(),
                                    },
                                );
                            }
                        }
                        Value::Bound(term) => {
                            where_parts.push(format!(
                                "\"{}\".\"{}\" = {}",
                                alias, c.field_name, sql_literal(term)
                            ));
                        }
                    }
                }

                // Assemble FROM / JOIN.
                if from_parts.is_empty() {
                    from_parts.push(format!("{} AS \"{}\"", source, alias));
                    where_parts.extend(join_conds);
                } else if join_conds.is_empty() {
                    from_parts.push(format!("CROSS JOIN {} AS \"{}\"", source, alias));
                } else {
                    from_parts.push(format!(
                        "INNER JOIN {} AS \"{}\" ON {}",
                        source,
                        alias,
                        join_conds.join(" AND ")
                    ));
                }
            }

            QueryPattern::FieldScan { .. } => {
                return Err(DarqError::ParseError(
                    "FieldScan patterns are not yet supported in SQL translation".into(),
                ));
            }
        }
    }

    // Resolve selected variables.
    let vars = match &plan.select {
        SelectClause::Variables(vars) => vars.iter().map(|v| v.0.clone()).collect::<Vec<_>>(),
        SelectClause::Star => plan.collect_variables(),
    };

    let select_cols: Vec<String> = vars
        .iter()
        .filter_map(|v| {
            bindings
                .get(v)
                .map(|expr| format!("{} AS \"{}\"", expr.to_sql(), v))
        })
        .collect();

    let distinct = if plan.modifier.distinct {
        "DISTINCT "
    } else {
        ""
    };

    let mut sql = format!("SELECT {}{}", distinct, select_cols.join(", "));

    if !from_parts.is_empty() {
        sql.push_str(&format!("\nFROM {}", from_parts.join("\n")));
    }

    if !where_parts.is_empty() {
        sql.push_str(&format!("\nWHERE {}", where_parts.join(" AND ")));
    }

    if !plan.modifier.order_by.is_empty() {
        let order_parts: Vec<String> = plan
            .modifier
            .order_by
            .iter()
            .map(|oc| {
                let dir = match oc.direction {
                    OrderDirection::Ascending => "ASC",
                    OrderDirection::Descending => "DESC",
                };
                match bindings.get(&oc.variable.0) {
                    Some(expr) => format!("{} {}", expr.to_sql(), dir),
                    None => format!("\"{}\" {}", oc.variable.0, dir),
                }
            })
            .collect();
        sql.push_str(&format!("\nORDER BY {}", order_parts.join(", ")));
    }

    if let Some(limit) = plan.modifier.limit {
        sql.push_str(&format!("\nLIMIT {}", limit));
    }

    if let Some(offset) = plan.modifier.offset {
        sql.push_str(&format!("\nOFFSET {}", offset));
    }

    Ok(sql)
}

/// Extract the local name from an IRI (the part after the last `#` or `/`).
fn table_name(iri: &Iri) -> &str {
    let s = &iri.0;
    if let Some(pos) = s.rfind('#') {
        &s[pos + 1..]
    } else if let Some(pos) = s.rfind('/') {
        &s[pos + 1..]
    } else {
        s
    }
}

/// Convert a Term to a SQL literal string.
fn sql_literal(term: &Term) -> String {
    match term {
        Term::Iri(iri) => format!("'{}'", iri.0.replace('\'', "''")),
        Term::Literal(lit) => match lit {
            Literal::String(s) => format!("'{}'", s.replace('\'', "''")),
            Literal::Integer(n) => n.to_string(),
            Literal::Boolean(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{FieldConstraint, QueryPattern, QueryPlan, Subject, Value};
    use crate::rdf::{Iri, Literal, Term};
    use crate::sparql::ast::*;

    fn empty_modifier() -> SolutionModifier {
        SolutionModifier::default()
    }

    #[test]
    fn test_simple_select() {
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
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![
                Variable("name".into()),
                Variable("age".into()),
            ]),
            modifier: empty_modifier(),
        };

        let sql = to_sql(&plan, &Schema::new()).unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"name\", \"p0\".\"age\" AS \"age\"\n\
             FROM \"Person\" AS \"p0\""
        );
    }

    #[test]
    fn test_bound_value_filter() {
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
                        value: Value::Bound(Term::Literal(Literal::Integer(30))),
                    },
                ],
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![Variable("name".into())]),
            modifier: empty_modifier(),
        };

        let sql = to_sql(&plan, &Schema::new()).unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"name\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"age\" = 30"
        );
    }

    #[test]
    fn test_cross_type_join() {
        let plan = QueryPlan {
            patterns: vec![
                QueryPattern::Resource {
                    subject: Subject::Variable("person".into()),
                    type_iri: Some(Iri::new("http://example.org/Person")),
                    constraints: vec![
                        FieldConstraint {
                            field_name: "name".into(),
                            value: Value::Variable("pname".into()),
                        },
                        FieldConstraint {
                            field_name: "pet".into(),
                            value: Value::Variable("pet".into()),
                        },
                    ],
                    type_variable: None,
                },
                QueryPattern::Resource {
                    subject: Subject::Variable("pet".into()),
                    type_iri: Some(Iri::new("http://example.org/Duck")),
                    constraints: vec![FieldConstraint {
                        field_name: "name".into(),
                        value: Value::Variable("dname".into()),
                    }],
                    type_variable: None,
                },
            ],
            select: SelectClause::Variables(vec![
                Variable("pname".into()),
                Variable("dname".into()),
            ]),
            modifier: empty_modifier(),
        };

        let sql = to_sql(&plan, &Schema::new()).unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"pname\", \"p1\".\"name\" AS \"dname\"\n\
             FROM \"Person\" AS \"p0\"\n\
             INNER JOIN \"Duck\" AS \"p1\" ON \"p1\".\"_subject\" = \"p0\".\"pet\""
        );
    }

    #[test]
    fn test_bound_subject() {
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Bound(Iri::new("http://example.org/person/alice")),
                type_iri: Some(Iri::new("http://example.org/Person")),
                constraints: vec![FieldConstraint {
                    field_name: "name".into(),
                    value: Value::Variable("name".into()),
                }],
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![Variable("name".into())]),
            modifier: empty_modifier(),
        };

        let sql = to_sql(&plan, &Schema::new()).unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"name\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"_subject\" = 'http://example.org/person/alice'"
        );
    }

    #[test]
    fn test_select_star() {
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
                type_variable: None,
            }],
            select: SelectClause::Star,
            modifier: empty_modifier(),
        };

        let sql = to_sql(&plan, &Schema::new()).unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"_subject\" AS \"p\", \"p0\".\"name\" AS \"name\", \"p0\".\"age\" AS \"age\"\n\
             FROM \"Person\" AS \"p0\""
        );
    }

    #[test]
    fn test_modifiers() {
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("p".into()),
                type_iri: Some(Iri::new("http://example.org/Person")),
                constraints: vec![FieldConstraint {
                    field_name: "name".into(),
                    value: Value::Variable("name".into()),
                }],
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![Variable("name".into())]),
            modifier: SolutionModifier {
                distinct: true,
                order_by: vec![OrderCondition {
                    variable: Variable("name".into()),
                    direction: OrderDirection::Ascending,
                }],
                limit: Some(10),
                offset: Some(5),
            },
        };

        let sql = to_sql(&plan, &Schema::new()).unwrap();
        assert_eq!(
            sql,
            "SELECT DISTINCT \"p0\".\"name\" AS \"name\"\n\
             FROM \"Person\" AS \"p0\"\n\
             ORDER BY \"p0\".\"name\" ASC\n\
             LIMIT 10\n\
             OFFSET 5"
        );
    }

    #[test]
    fn test_type_variable_known_type() {
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("s".into()),
                type_iri: Some(Iri::new("http://example.org/Person")),
                constraints: vec![],
                type_variable: Some("type".into()),
            }],
            select: SelectClause::Variables(vec![
                Variable("s".into()),
                Variable("type".into()),
            ]),
            modifier: empty_modifier(),
        };

        let sql = to_sql(&plan, &Schema::new()).unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"_subject\" AS \"s\", 'http://example.org/Person' AS \"type\"\n\
             FROM \"Person\" AS \"p0\""
        );
    }

    #[test]
    fn test_order_by_desc() {
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
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![
                Variable("name".into()),
                Variable("age".into()),
            ]),
            modifier: SolutionModifier {
                distinct: false,
                order_by: vec![OrderCondition {
                    variable: Variable("age".into()),
                    direction: OrderDirection::Descending,
                }],
                limit: Some(1),
                offset: None,
            },
        };

        let sql = to_sql(&plan, &Schema::new()).unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"name\", \"p0\".\"age\" AS \"age\"\n\
             FROM \"Person\" AS \"p0\"\n\
             ORDER BY \"p0\".\"age\" DESC\n\
             LIMIT 1"
        );
    }

    #[test]
    fn test_string_literal_escaping() {
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("p".into()),
                type_iri: Some(Iri::new("http://example.org/Person")),
                constraints: vec![FieldConstraint {
                    field_name: "name".into(),
                    value: Value::Bound(Term::Literal(Literal::String(
                        "O'Brien".into(),
                    ))),
                }],
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![Variable("p".into())]),
            modifier: empty_modifier(),
        };

        let sql = to_sql(&plan, &Schema::new()).unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"_subject\" AS \"p\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"name\" = 'O''Brien'"
        );
    }

    #[test]
    fn test_boolean_literal() {
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("p".into()),
                type_iri: Some(Iri::new("http://example.org/Person")),
                constraints: vec![FieldConstraint {
                    field_name: "active".into(),
                    value: Value::Bound(Term::Literal(Literal::Boolean(true))),
                }],
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![Variable("p".into())]),
            modifier: empty_modifier(),
        };

        let sql = to_sql(&plan, &Schema::new()).unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"_subject\" AS \"p\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"active\" = TRUE"
        );
    }

    #[test]
    fn test_same_variable_two_fields() {
        // ?p ex:name ?x . ?p ex:label ?x — same variable binds two columns
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("p".into()),
                type_iri: Some(Iri::new("http://example.org/Person")),
                constraints: vec![
                    FieldConstraint {
                        field_name: "name".into(),
                        value: Value::Variable("x".into()),
                    },
                    FieldConstraint {
                        field_name: "label".into(),
                        value: Value::Variable("x".into()),
                    },
                ],
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![Variable("x".into())]),
            modifier: empty_modifier(),
        };

        let sql = to_sql(&plan, &Schema::new()).unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"x\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"label\" = \"p0\".\"name\""
        );
    }

    #[test]
    fn test_unbound_variable_skipped() {
        // If an unbound variable somehow reaches SQL generation (e.g. built
        // by hand), it is silently omitted from the SELECT list.
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("p".into()),
                type_iri: Some(Iri::new("http://example.org/Person")),
                constraints: vec![FieldConstraint {
                    field_name: "name".into(),
                    value: Value::Bound(Term::Literal(Literal::String("Alice".into()))),
                }],
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![
                Variable("name".into()),
                Variable("p".into()),
            ]),
            modifier: empty_modifier(),
        };

        let sql = to_sql(&plan, &Schema::new()).unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"_subject\" AS \"p\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"name\" = 'Alice'"
        );
    }

    #[test]
    fn test_iri_fragment_table_name() {
        assert_eq!(table_name(&Iri::new("http://xmlns.com/foaf/0.1#Person")), "Person");
    }

    #[test]
    fn test_iri_path_table_name() {
        assert_eq!(table_name(&Iri::new("http://example.org/Person")), "Person");
    }
}
