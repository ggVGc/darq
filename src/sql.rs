use std::collections::HashMap;

use crate::error::DarqError;
use crate::ir::{NotExistsFilter, QueryPattern, QueryPlan, Subject, Value};
use crate::rdf::{Iri, Literal, Term};
use crate::schema::Schema;
use crate::sparql::ast::{OrderDirection, SelectClause};

/// How a SPARQL variable maps to a SQL expression.
#[derive(Clone)]
enum SqlExpr {
    /// A column reference: `"p<idx>"."<column>"`.
    Column { pattern_idx: usize, column: String },
    /// An unnested array column: `unnest("p<idx>"."<column>")`.
    Unnest { pattern_idx: usize, column: String },
    /// A constant SQL literal (e.g., a type IRI string).
    Constant(String),
}

impl SqlExpr {
    fn to_sql(&self) -> String {
        match self {
            SqlExpr::Column { pattern_idx, column } => {
                format!("\"p{}\".\"{}\"", pattern_idx, column)
            }
            SqlExpr::Unnest { pattern_idx, column } => {
                format!("unnest(\"p{}\".\"{}\")", pattern_idx, column)
            }
            SqlExpr::Constant(s) => s.clone(),
        }
    }
}

/// Translate a resource-level query plan into a SQL string.
///
/// Assumes one table per resource type, named after the local part of the
/// type IRI (e.g. `http://example.org/Person` → `"Person"`), with a
/// `subject_column` for the resource IRI, an `id_column` for the primary key
/// used in joins, and one column per field.
///
/// When `id_column` differs from `subject_column`, joins use the id column
/// (matching foreign-key references) while SELECT output uses the subject
/// column (returning full IRIs).
pub fn to_sql(plan: &QueryPlan, schema: &Schema, subject_column: &str, id_column: &str) -> Result<String, DarqError> {
    // select_bindings: used for SELECT output and ORDER BY (returns full IRIs for subjects).
    // join_bindings:   used for JOIN/WHERE conditions (uses id for subjects).
    let mut select_bindings: HashMap<String, SqlExpr> = HashMap::new();
    let mut join_bindings: HashMap<String, SqlExpr> = HashMap::new();
    let mut from_parts: Vec<String> = Vec::new();
    let mut where_parts: Vec<String> = Vec::new();
    // Track reference-typed field variables that may need LEFT JOIN resolution
    // to return full IRIs instead of raw foreign key IDs.
    // Key: variable name, Value: (pattern_idx, field_column, target_type_iris)
    let mut ref_field_bindings: HashMap<String, (usize, String, Vec<Iri>)> = HashMap::new();

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
                    let tbl = schema.table_name(ti).unwrap_or_else(|| iri_local_name(ti));
                    format!("\"{}\"", tbl)
                } else {
                    // No concrete type — UNION ALL over all registered types.
                    let parts: Vec<String> = schema
                        .known_types()
                        .map(|ti| {
                            let tbl = schema.table_name(ti).unwrap_or_else(|| iri_local_name(ti));
                            format!(
                                "SELECT \"{}\", \"{}\", '{}' AS \"_type\" FROM \"{}\"",
                                subject_column,
                                id_column,
                                ti.0,
                                tbl
                            )
                        })
                        .collect();
                    if parts.is_empty() {
                        format!("(SELECT NULL AS \"{}\", NULL AS \"{}\", NULL AS \"_type\" WHERE FALSE)", subject_column, id_column)
                    } else {
                        format!("({})", parts.join(" UNION ALL "))
                    }
                };

                // Subject binding.
                match subject {
                    Subject::Variable(v) => {
                        if let Some(existing) = join_bindings.get(v) {
                            join_conds.push(format!(
                                "\"{}\".\"{}\" = {}",
                                alias,
                                id_column,
                                existing.to_sql()
                            ));
                            // Override select binding: use the subject column
                            // from this pattern (canonical IRI).
                            select_bindings.insert(
                                v.clone(),
                                SqlExpr::Column {
                                    pattern_idx: i,
                                    column: subject_column.to_string(),
                                },
                            );
                            // No longer needs LEFT JOIN resolution — the subject
                            // pattern provides the full IRI directly.
                            ref_field_bindings.remove(v);
                        } else {
                            select_bindings.insert(
                                v.clone(),
                                SqlExpr::Column {
                                    pattern_idx: i,
                                    column: subject_column.to_string(),
                                },
                            );
                            join_bindings.insert(
                                v.clone(),
                                SqlExpr::Column {
                                    pattern_idx: i,
                                    column: id_column.to_string(),
                                },
                            );
                        }
                    }
                    Subject::Bound(iri) => {
                        where_parts.push(format!(
                            "\"{}\".\"{}\" = '{}'",
                            alias, subject_column, iri.0
                        ));
                    }
                }

                // Type variable binding.
                if let Some(tv) = type_variable {
                    if let Some(ti) = type_iri {
                        // Known type — bind to a constant.
                        let constant = SqlExpr::Constant(format!("'{}'", ti.0));
                        select_bindings.insert(tv.clone(), constant.clone());
                        join_bindings.insert(tv.clone(), constant);
                    } else {
                        // Unknown type — bind to the _type column from the UNION.
                        if let Some(existing) = join_bindings.get(tv) {
                            join_conds.push(format!(
                                "\"{}\".\"_type\" = {}",
                                alias,
                                existing.to_sql()
                            ));
                        } else {
                            let expr = SqlExpr::Column {
                                pattern_idx: i,
                                column: "_type".into(),
                            };
                            select_bindings.insert(tv.clone(), expr.clone());
                            join_bindings.insert(tv.clone(), expr);
                        }
                    }
                }

                // Field constraint bindings.
                let fields = type_iri
                    .as_ref()
                    .and_then(|ti| schema.fields_for_type(ti));
                for c in constraints {
                    let is_array = fields
                        .map(|fs| {
                            fs.iter().any(|f| {
                                f.name == c.field_name
                                    && matches!(f.field_type, crate::schema::FieldType::StringArray | crate::schema::FieldType::ReferenceArray(_))
                            })
                        })
                        .unwrap_or(false);

                    match &c.value {
                        Value::Variable(v) => {
                            if let Some(existing) = join_bindings.get(v) {
                                if is_array {
                                    join_conds.push(format!(
                                        "{} = ANY(\"{}\".\"{}\")",
                                        existing.to_sql(),
                                        alias,
                                        c.field_name,
                                    ));
                                } else {
                                    join_conds.push(format!(
                                        "\"{}\".\"{}\" = {}",
                                        alias,
                                        c.field_name,
                                        existing.to_sql()
                                    ));
                                }
                            } else if is_array {
                                let expr = SqlExpr::Unnest {
                                    pattern_idx: i,
                                    column: c.field_name.clone(),
                                };
                                select_bindings.insert(v.clone(), expr.clone());
                                join_bindings.insert(v.clone(), expr);
                            } else {
                                let expr = SqlExpr::Column {
                                    pattern_idx: i,
                                    column: c.field_name.clone(),
                                };
                                select_bindings.insert(v.clone(), expr.clone());
                                join_bindings.insert(v.clone(), expr);
                                // SPARQL basic graph pattern: the triple must exist.
                                join_conds.push(format!(
                                    "\"{}\".\"{}\" IS NOT NULL",
                                    alias, c.field_name
                                ));
                                // Track Reference fields for LEFT JOIN IRI resolution.
                                if let Some(fd) = fields.and_then(|fs| fs.iter().find(|f| f.name == c.field_name)) {
                                    if let crate::schema::FieldType::Reference(ref targets) = fd.field_type {
                                        if !targets.is_empty() {
                                            ref_field_bindings.insert(
                                                v.clone(),
                                                (i, c.field_name.clone(), targets.clone()),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        Value::Bound(term) => {
                            if is_array {
                                where_parts.push(format!(
                                    "{} = ANY(\"{}\".\"{}\")",
                                    sql_literal(term), alias, c.field_name
                                ));
                            } else {
                                where_parts.push(format!(
                                    "\"{}\".\"{}\" = {}",
                                    alias, c.field_name, sql_literal(term)
                                ));
                            }
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

    // Add LEFT JOINs for reference field variables that were never resolved
    // by appearing as a subject in a later pattern.  This ensures SELECT
    // returns the full IRI (from the referenced table's subject column)
    // instead of the raw foreign-key ID.
    let mut ref_alias_counter = 0usize;
    for (var_name, (pattern_idx, field_col, target_types)) in &ref_field_bindings {
        let field_expr = format!("\"p{}\".\"{}\"", pattern_idx, field_col);

        if target_types.len() == 1 {
            let target_type = &target_types[0];
            let target_table = schema
                .table_name(target_type)
                .unwrap_or_else(|| iri_local_name(target_type));
            let ref_alias = format!("_ref{}", ref_alias_counter);
            ref_alias_counter += 1;

            from_parts.push(format!(
                "LEFT JOIN \"{}\" AS \"{}\" ON \"{}\".\"{}\" = {}",
                target_table, ref_alias, ref_alias, id_column, field_expr
            ));
            select_bindings.insert(
                var_name.clone(),
                SqlExpr::Constant(format!("\"{}\".\"{}\"", ref_alias, subject_column)),
            );
        } else {
            // Multi-target reference: LEFT JOIN each target, COALESCE results.
            let mut coalesce_parts = Vec::new();
            for target_type in target_types {
                let target_table = schema
                    .table_name(target_type)
                    .unwrap_or_else(|| iri_local_name(target_type));
                let ref_alias = format!("_ref{}", ref_alias_counter);
                ref_alias_counter += 1;

                from_parts.push(format!(
                    "LEFT JOIN \"{}\" AS \"{}\" ON \"{}\".\"{}\" = {}",
                    target_table, ref_alias, ref_alias, id_column, field_expr
                ));
                coalesce_parts.push(format!("\"{}\".\"{}\"", ref_alias, subject_column));
            }
            select_bindings.insert(
                var_name.clone(),
                SqlExpr::Constant(format!("COALESCE({})", coalesce_parts.join(", "))),
            );
        }
    }

    // VALUES clause: inline data filtering/joining.
    if let Some(ref values) = plan.values {
        let values_alias = "_values";
        let no_undefs = values
            .rows
            .iter()
            .all(|r| r.iter().all(|v| v.is_some()));

        if values.variables.len() == 1 && no_undefs {
            // Single-variable, no UNDEFs: use IN clause or derived table.
            let var = &values.variables[0];
            let in_values: Vec<String> = values
                .rows
                .iter()
                .filter_map(|r| r[0].as_ref().map(sql_literal))
                .collect();

            if let Some(expr) = select_bindings.get(var) {
                // Variable already bound by a pattern — add WHERE IN filter.
                where_parts.push(format!(
                    "{} IN ({})",
                    expr.to_sql(),
                    in_values.join(", ")
                ));
            } else {
                // Variable only from VALUES — add as a derived table.
                let rows_sql: Vec<String> = in_values
                    .iter()
                    .map(|v| format!("SELECT {} AS \"{}\"", v, var))
                    .collect();
                let derived = format!("({})", rows_sql.join(" UNION ALL "));
                if from_parts.is_empty() {
                    from_parts.push(format!("{} AS \"{}\"", derived, values_alias));
                } else {
                    from_parts.push(format!(
                        "CROSS JOIN {} AS \"{}\"",
                        derived, values_alias
                    ));
                }
                let expr = SqlExpr::Constant(format!(
                    "\"{}\".\"{}\"",
                    values_alias, var
                ));
                select_bindings.insert(var.clone(), expr.clone());
                join_bindings.insert(var.clone(), expr);
            }
        } else {
            // Multi-variable or has UNDEFs: derived table with UNION ALL rows.
            let rows_sql: Vec<String> = values
                .rows
                .iter()
                .map(|row| {
                    let vals: Vec<String> = row
                        .iter()
                        .enumerate()
                        .map(|(i, v)| {
                            let sql_val = match v {
                                Some(term) => sql_literal(term),
                                None => "NULL".to_string(),
                            };
                            format!("{} AS \"{}\"", sql_val, values.variables[i])
                        })
                        .collect();
                    format!("SELECT {}", vals.join(", "))
                })
                .collect();
            let derived = format!("({})", rows_sql.join(" UNION ALL "));

            let mut join_conds = Vec::new();
            for var in &values.variables {
                if let Some(expr) = join_bindings.get(var) {
                    join_conds.push(format!(
                        "\"{}\".\"{}\" = {}",
                        values_alias, var, expr.to_sql()
                    ));
                } else {
                    let expr = SqlExpr::Constant(format!(
                        "\"{}\".\"{}\"",
                        values_alias, var
                    ));
                    select_bindings.insert(var.clone(), expr.clone());
                    join_bindings.insert(var.clone(), expr);
                }
            }

            if from_parts.is_empty() {
                from_parts.push(format!("{} AS \"{}\"", derived, values_alias));
            } else if join_conds.is_empty() {
                from_parts.push(format!(
                    "CROSS JOIN {} AS \"{}\"",
                    derived, values_alias
                ));
            } else {
                from_parts.push(format!(
                    "INNER JOIN {} AS \"{}\" ON {}",
                    derived, values_alias, join_conds.join(" AND ")
                ));
            }
        }
    }

    // NOT EXISTS filters: try decorrelation for anonymous-subject patterns,
    // fall back to correlated subqueries otherwise.
    let mut decorrelated_groups: HashMap<String, Vec<DecorrelatedAntiJoin>> = HashMap::new();
    let mut correlated_filters: Vec<(usize, &NotExistsFilter)> = Vec::new();

    for (fi, filter) in plan.filters.iter().enumerate() {
        if let Some(aj) = try_decorrelate_not_exists(filter, schema, id_column, &join_bindings) {
            decorrelated_groups
                .entry(aj.outer_variable.clone())
                .or_default()
                .push(aj);
        } else {
            correlated_filters.push((fi, filter));
        }
    }

    // Emit combined NOT IN clauses for decorrelated groups.
    for anti_joins in decorrelated_groups.values() {
        let outer_expr = anti_joins[0].outer_expr.to_sql();
        let subqueries: Vec<String> = anti_joins
            .iter()
            .map(|aj| {
                if aj.is_array {
                    format!(
                        "SELECT _elem FROM \"{}\", unnest(\"{}\") AS _elem WHERE _elem IS NOT NULL",
                        aj.table_name, aj.field_column
                    )
                } else {
                    format!(
                        "SELECT \"{}\" FROM \"{}\" WHERE \"{}\" IS NOT NULL",
                        aj.field_column, aj.table_name, aj.field_column
                    )
                }
            })
            .collect();
        let combined = subqueries.join(" UNION ALL ");
        where_parts.push(format!("{} NOT IN ({})", outer_expr, combined));
    }

    // Emit correlated NOT EXISTS for non-decorrelatable filters.
    for (fi, filter) in &correlated_filters {
        let subquery = generate_not_exists_subquery(
            filter, *fi, schema, subject_column, id_column, &join_bindings,
        )?;
        where_parts.push(format!("NOT EXISTS ({})", subquery));
    }

    // Resolve selected variables (using select_bindings for output).
    let vars = match &plan.select {
        SelectClause::Variables(vars) => vars.iter().map(|v| v.0.clone()).collect::<Vec<_>>(),
        SelectClause::Star => plan.collect_variables(),
    };

    let select_cols: Vec<String> = vars
        .iter()
        .filter_map(|v| {
            select_bindings
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
                match select_bindings.get(&oc.variable.0) {
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

/// Translate multiple resource-level query plans into a single SQL string
/// using UNION ALL, letting the database handle the combined result set.
///
/// Each plan becomes a subquery; modifiers (ORDER BY, LIMIT, OFFSET) are
/// applied on the outer query. DISTINCT is omitted from SQL — it is handled
/// post-projection by `execute()`.
pub fn to_union_sql(
    plans: &[QueryPlan],
    schema: &Schema,
    subject_column: &str,
    id_column: &str,
) -> Result<String, DarqError> {
    use crate::sparql::ast::SolutionModifier;

    let modifier = &plans[0].modifier;

    // Generate inner SQL for each plan (no modifiers, all variables).
    let mut subqueries = Vec::new();
    for plan in plans {
        let inner_plan = QueryPlan {
            patterns: plan.patterns.clone(),
            filters: plan.filters.clone(),
            select: SelectClause::Star,
            modifier: SolutionModifier::default(),
            values: plan.values.clone(),
        };
        subqueries.push(to_sql(&inner_plan, schema, subject_column, id_column)?);
    }

    let union_body = subqueries.join("\nUNION ALL\n");
    let mut sql = format!("SELECT DISTINCT * FROM (\n{}\n) AS \"_combined\"", union_body);

    if !modifier.order_by.is_empty() {
        let order_parts: Vec<String> = modifier
            .order_by
            .iter()
            .map(|oc| {
                let dir = match oc.direction {
                    OrderDirection::Ascending => "ASC",
                    OrderDirection::Descending => "DESC",
                };
                format!("\"{}\" {}", oc.variable.0, dir)
            })
            .collect();
        sql.push_str(&format!("\nORDER BY {}", order_parts.join(", ")));
    }

    // DISTINCT is handled post-projection by execute(). LIMIT/OFFSET are
    // applied in SQL only when DISTINCT is not active (same as to_sql).
    if !modifier.distinct {
        if let Some(limit) = modifier.limit {
            sql.push_str(&format!("\nLIMIT {}", limit));
        }
        if let Some(offset) = modifier.offset {
            sql.push_str(&format!("\nOFFSET {}", offset));
        }
    }

    Ok(sql)
}

/// Extract the local name from an IRI (the part after the last `#` or `/`).
/// Used as fallback when a type is not registered in the schema.
fn iri_local_name(iri: &Iri) -> &str {
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
pub(crate) fn sql_literal(term: &Term) -> String {
    match term {
        Term::Iri(iri) => format!("'{}'", iri.0.replace('\'', "''")),
        Term::Literal(lit) => match lit {
            Literal::String(s) => format!("'{}'", s.replace('\'', "''")),
            Literal::Integer(n) => n.to_string(),
            Literal::Boolean(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            Literal::Float(v) | Literal::Double(v) => format!("{}", v.0),
            Literal::Decimal(s) => format!("'{}'", s.replace('\'', "''")),
            Literal::Date(s) => format!("'{}'", s.replace('\'', "''")),
            Literal::DateTime(s) => format!("'{}'", s.replace('\'', "''")),
        },
    }
}

/// Information for a decorrelated NOT IN anti-join.
struct DecorrelatedAntiJoin {
    /// The outer variable being excluded (e.g., "dobj").
    outer_variable: String,
    /// The SQL expression for the outer variable from join_bindings.
    outer_expr: SqlExpr,
    /// The inner table to scan.
    table_name: String,
    /// The field column containing the referenced IDs.
    field_column: String,
    /// Whether the field is an array (ReferenceArray) or scalar (Reference).
    is_array: bool,
}

/// Try to decorrelate a NOT EXISTS filter into a NOT IN anti-join.
///
/// A filter is decorrelatable when:
/// 1. It has exactly one inner Resource pattern.
/// 2. The inner subject is a variable NOT in outer bindings (anonymous/fresh).
/// 3. There is exactly one field constraint whose value is a variable in outer bindings.
/// 4. The type is known (Some).
fn try_decorrelate_not_exists(
    filter: &NotExistsFilter,
    schema: &Schema,
    _id_column: &str,
    outer_bindings: &HashMap<String, SqlExpr>,
) -> Option<DecorrelatedAntiJoin> {
    if filter.inner_patterns.len() != 1 {
        return None;
    }

    match &filter.inner_patterns[0] {
        QueryPattern::Resource {
            subject,
            type_iri: Some(ti),
            constraints,
            ..
        } => {
            // Subject must NOT be in outer bindings (anonymous/uncorrelated).
            match subject {
                Subject::Variable(v) if !outer_bindings.contains_key(v) => {}
                _ => return None,
            }

            // Must have exactly one constraint with a correlated variable.
            if constraints.len() != 1 {
                return None;
            }
            let c = &constraints[0];
            let outer_var = match &c.value {
                Value::Variable(v) if outer_bindings.contains_key(v) => v.clone(),
                _ => return None,
            };

            let outer_expr = outer_bindings.get(&outer_var)?.clone();
            let table_name = schema
                .table_name(ti)
                .unwrap_or_else(|| iri_local_name(ti))
                .to_string();

            let is_array = schema
                .fields_for_type(ti)
                .and_then(|fields| fields.iter().find(|f| f.name == c.field_name))
                .map(|f| {
                    matches!(
                        f.field_type,
                        crate::schema::FieldType::StringArray
                            | crate::schema::FieldType::ReferenceArray(_)
                    )
                })
                .unwrap_or(false);

            Some(DecorrelatedAntiJoin {
                outer_variable: outer_var,
                outer_expr,
                table_name,
                field_column: c.field_name.clone(),
                is_array,
            })
        }
        _ => None,
    }
}

/// Generate a correlated NOT EXISTS subquery from a filter's inner patterns.
fn generate_not_exists_subquery(
    filter: &NotExistsFilter,
    filter_idx: usize,
    schema: &Schema,
    subject_column: &str,
    id_column: &str,
    outer_bindings: &HashMap<String, SqlExpr>,
) -> Result<String, DarqError> {
    let mut inner_from: Vec<String> = Vec::new();
    let mut inner_where: Vec<String> = Vec::new();
    let mut inner_bindings: HashMap<String, SqlExpr> = HashMap::new();

    for (i, pattern) in filter.inner_patterns.iter().enumerate() {
        let alias = format!("_ne{}_{}", filter_idx, i);

        match pattern {
            QueryPattern::Resource {
                subject,
                type_iri,
                constraints,
                ..
            } => {
                let source = if let Some(ti) = type_iri {
                    let tbl = schema.table_name(ti).unwrap_or_else(|| iri_local_name(ti));
                    format!("\"{}\"", tbl)
                } else {
                    return Err(DarqError::SqlError(
                        "NOT EXISTS inner pattern requires a known type".into(),
                    ));
                };

                let mut join_conds: Vec<String> = Vec::new();

                // Subject binding/correlation.
                match subject {
                    Subject::Variable(v) => {
                        if let Some(outer_expr) = outer_bindings.get(v) {
                            // Correlate with outer query
                            join_conds.push(format!(
                                "\"{}\".\"{}\" = {}",
                                alias, id_column, outer_expr.to_sql()
                            ));
                        } else if let Some(inner_expr) = inner_bindings.get(v) {
                            join_conds.push(format!(
                                "\"{}\".\"{}\" = {}",
                                alias, id_column, inner_expr.to_sql()
                            ));
                        } else {
                            inner_bindings.insert(
                                v.clone(),
                                SqlExpr::Column {
                                    pattern_idx: i,
                                    column: id_column.to_string(),
                                },
                            );
                        }
                    }
                    Subject::Bound(iri) => {
                        inner_where.push(format!(
                            "\"{}\".\"{}\" = '{}'",
                            alias, subject_column, iri.0
                        ));
                    }
                }

                // Field constraints.
                let fields = type_iri
                    .as_ref()
                    .and_then(|ti| schema.fields_for_type(ti));

                for c in constraints {
                    let is_array = fields
                        .map(|fs| {
                            fs.iter().any(|f| {
                                f.name == c.field_name
                                    && matches!(f.field_type, crate::schema::FieldType::StringArray | crate::schema::FieldType::ReferenceArray(_))
                            })
                        })
                        .unwrap_or(false);

                    match &c.value {
                        Value::Variable(v) => {
                            // Check outer bindings first (correlation), then inner
                            if let Some(outer_expr) = outer_bindings.get(v) {
                                if is_array {
                                    join_conds.push(format!(
                                        "{} = ANY(\"{}\".\"{}\")",
                                        outer_expr.to_sql(), alias, c.field_name,
                                    ));
                                } else {
                                    join_conds.push(format!(
                                        "\"{}\".\"{}\" = {}",
                                        alias, c.field_name, outer_expr.to_sql()
                                    ));
                                }
                            } else if let Some(inner_expr) = inner_bindings.get(v) {
                                if is_array {
                                    join_conds.push(format!(
                                        "{} = ANY(\"{}\".\"{}\")",
                                        inner_expr.to_sql(), alias, c.field_name,
                                    ));
                                } else {
                                    join_conds.push(format!(
                                        "\"{}\".\"{}\" = {}",
                                        alias, c.field_name, inner_expr.to_sql()
                                    ));
                                }
                            } else {
                                // New inner binding
                                let col_ref = format!("\"{}\".\"{}\"", alias, c.field_name);
                                inner_bindings.insert(
                                    v.clone(),
                                    SqlExpr::Constant(col_ref),
                                );
                            }
                        }
                        Value::Bound(term) => {
                            if is_array {
                                inner_where.push(format!(
                                    "{} = ANY(\"{}\".\"{}\")",
                                    sql_literal(term), alias, c.field_name
                                ));
                            } else {
                                inner_where.push(format!(
                                    "\"{}\".\"{}\" = {}",
                                    alias, c.field_name, sql_literal(term)
                                ));
                            }
                        }
                    }
                }

                // Build FROM/JOIN for inner query using same pattern as outer.
                if inner_from.is_empty() {
                    inner_from.push(format!("{} AS \"{}\"", source, alias));
                    inner_where.extend(join_conds);
                } else if join_conds.is_empty() {
                    inner_from.push(format!("CROSS JOIN {} AS \"{}\"", source, alias));
                } else {
                    inner_from.push(format!(
                        "INNER JOIN {} AS \"{}\" ON {}",
                        source, alias, join_conds.join(" AND ")
                    ));
                }
            }

            QueryPattern::FieldScan { .. } => {
                return Err(DarqError::SqlError(
                    "FieldScan not supported in NOT EXISTS subquery".into(),
                ));
            }
        }
    }

    let mut sql = "SELECT 1".to_string();
    if !inner_from.is_empty() {
        sql.push_str(&format!(" FROM {}", inner_from.join("\n")));
    }
    if !inner_where.is_empty() {
        sql.push_str(&format!(" WHERE {}", inner_where.join(" AND ")));
    }
    Ok(sql)
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"name\", \"p0\".\"age\" AS \"age\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"name\" IS NOT NULL AND \"p0\".\"age\" IS NOT NULL"
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"name\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"age\" = 30 AND \"p0\".\"name\" IS NOT NULL"
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"pname\", \"p1\".\"name\" AS \"dname\"\n\
             FROM \"Person\" AS \"p0\"\n\
             INNER JOIN \"Duck\" AS \"p1\" ON \"p1\".\"_subject\" = \"p0\".\"pet\" AND \"p1\".\"name\" IS NOT NULL\n\
             WHERE \"p0\".\"name\" IS NOT NULL AND \"p0\".\"pet\" IS NOT NULL"
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"name\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"_subject\" = 'http://example.org/person/alice' AND \"p0\".\"name\" IS NOT NULL"
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"_subject\" AS \"p\", \"p0\".\"name\" AS \"name\", \"p0\".\"age\" AS \"age\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"name\" IS NOT NULL AND \"p0\".\"age\" IS NOT NULL"
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        assert_eq!(
            sql,
            "SELECT DISTINCT \"p0\".\"name\" AS \"name\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"name\" IS NOT NULL\n\
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"name\", \"p0\".\"age\" AS \"age\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"name\" IS NOT NULL AND \"p0\".\"age\" IS NOT NULL\n\
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"x\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"name\" IS NOT NULL AND \"p0\".\"label\" = \"p0\".\"name\""
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"_subject\" AS \"p\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"name\" = 'Alice'"
        );
    }

    fn schema_with_array() -> Schema {
        use crate::schema::{FieldDescriptor, Resource};

        struct Instrument;
        impl Resource for Instrument {
            fn rdf_type() -> Iri {
                Iri::new("http://example.org/Instrument")
            }
            fn subject_iri(&self) -> Iri {
                Iri::new("http://example.org/instrument/test")
            }
            fn field_descriptors() -> Vec<crate::schema::FieldDescriptor> {
                vec![
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/model"),
                        name: "model",
                        field_type: crate::schema::FieldType::String,
                        indexed: false,
                    },
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/tags"),
                        name: "tags",
                        field_type: crate::schema::FieldType::StringArray,
                        indexed: false,
                    },
                ]
            }
            fn field_values(&self) -> Vec<Term> {
                vec![]
            }
        }

        let mut schema = Schema::new();
        schema.register::<Instrument>();
        schema
    }

    #[test]
    fn test_array_variable_uses_unnest_in_select() {
        let schema = schema_with_array();
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("s".into()),
                type_iri: Some(Iri::new("http://example.org/Instrument")),
                constraints: vec![FieldConstraint {
                    field_name: "tags".into(),
                    value: Value::Variable("tag".into()),
                }],
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![Variable("tag".into())]),
            modifier: empty_modifier(),
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &schema, "_subject", "_subject").unwrap();
        assert!(
            sql.contains("unnest(\"p0\".\"tags\")"),
            "Should use unnest for array variable column: {}",
            sql
        );
    }

    #[test]
    fn test_array_bound_uses_any_in_where() {
        let schema = schema_with_array();
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("s".into()),
                type_iri: Some(Iri::new("http://example.org/Instrument")),
                constraints: vec![FieldConstraint {
                    field_name: "tags".into(),
                    value: Value::Bound(Term::Literal(Literal::String("red".into()))),
                }],
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![Variable("s".into())]),
            modifier: empty_modifier(),
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &schema, "_subject", "_subject").unwrap();
        assert!(
            sql.contains("= ANY(\"p0\".\"tags\")"),
            "Should use = ANY() for bound array constraint: {}",
            sql
        );
    }

    #[test]
    fn test_cross_type_join_with_separate_id() {
        // When id_column differs from subject_column, joins use id_column
        // and SELECT uses subject_column for subject variables.
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "rdf_subject", "id").unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"pname\", \"p1\".\"name\" AS \"dname\"\n\
             FROM \"Person\" AS \"p0\"\n\
             INNER JOIN \"Duck\" AS \"p1\" ON \"p1\".\"id\" = \"p0\".\"pet\" AND \"p1\".\"name\" IS NOT NULL\n\
             WHERE \"p0\".\"name\" IS NOT NULL AND \"p0\".\"pet\" IS NOT NULL"
        );
    }

    #[test]
    fn test_field_then_subject_variable_uses_subject_column() {
        // When a variable is first bound as a field value and later used as a
        // subject, SELECT should use subject_column from the subject pattern
        // (giving the full IRI), not the original field column (which stores an id).
        let plan = QueryPlan {
            patterns: vec![
                QueryPattern::Resource {
                    subject: Subject::Variable("person".into()),
                    type_iri: Some(Iri::new("http://example.org/Person")),
                    constraints: vec![FieldConstraint {
                        field_name: "pet".into(),
                        value: Value::Variable("pet".into()),
                    }],
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
                Variable("pet".into()),
                Variable("dname".into()),
            ]),
            modifier: empty_modifier(),
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "rdf_subject", "id").unwrap();
        // ?pet in SELECT should reference p1.rdf_subject (the subject pattern),
        // not p0.pet (the field where it was first bound).
        assert_eq!(
            sql,
            "SELECT \"p1\".\"rdf_subject\" AS \"pet\", \"p1\".\"name\" AS \"dname\"\n\
             FROM \"Person\" AS \"p0\"\n\
             INNER JOIN \"Duck\" AS \"p1\" ON \"p1\".\"id\" = \"p0\".\"pet\" AND \"p1\".\"name\" IS NOT NULL\n\
             WHERE \"p0\".\"pet\" IS NOT NULL"
        );
    }

    #[test]
    fn test_bound_subject_with_separate_id() {
        // Bound subjects should still match against subject_column (full IRI).
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "rdf_subject", "id").unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"name\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"rdf_subject\" = 'http://example.org/person/alice' AND \"p0\".\"name\" IS NOT NULL"
        );
    }

    #[test]
    fn test_select_star_with_separate_id() {
        // SELECT * with separate id/subject columns should use subject_column
        // for subject variables in the output.
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
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "rdf_subject", "id").unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"rdf_subject\" AS \"p\", \"p0\".\"name\" AS \"name\", \"p0\".\"age\" AS \"age\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"name\" IS NOT NULL AND \"p0\".\"age\" IS NOT NULL"
        );
    }

    /// Build a schema where Person has a Reference field "pet" pointing to Duck.
    fn schema_with_ref() -> Schema {
        use crate::schema::{FieldDescriptor, FieldType, Resource};

        struct Person;
        impl Resource for Person {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Person") }
            fn subject_iri(&self) -> Iri { Iri::new("http://example.org/person/test") }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/name"),
                        name: "name",
                        field_type: FieldType::String,
                        indexed: false,
                    },
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/pet"),
                        name: "pet",
                        field_type: FieldType::Reference(vec![Iri::new("http://example.org/Duck")]),
                        indexed: false,
                    },
                ]
            }
            fn field_values(&self) -> Vec<Term> { vec![] }
        }

        struct Duck;
        impl Resource for Duck {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Duck") }
            fn subject_iri(&self) -> Iri { Iri::new("http://example.org/duck/test") }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![FieldDescriptor {
                    predicate: Iri::new("http://example.org/duckName"),
                    name: "name",
                    field_type: FieldType::String,
                    indexed: false,
                }]
            }
            fn field_values(&self) -> Vec<Term> { vec![] }
        }

        let mut schema = Schema::new();
        schema.register::<Person>();
        schema.register::<Duck>();
        schema
    }

    #[test]
    fn test_reference_field_not_used_as_subject_gets_left_join() {
        // When a Reference field variable is NOT later used as a subject,
        // the SQL should LEFT JOIN the referenced table to return the full IRI.
        let schema = schema_with_ref();
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
                        field_name: "pet".into(),
                        value: Value::Variable("pet".into()),
                    },
                ],
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![
                Variable("name".into()),
                Variable("pet".into()),
            ]),
            modifier: empty_modifier(),
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &schema, "rdf_subject", "id").unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"name\", \"_ref0\".\"rdf_subject\" AS \"pet\"\n\
             FROM \"Person\" AS \"p0\"\n\
             LEFT JOIN \"Duck\" AS \"_ref0\" ON \"_ref0\".\"id\" = \"p0\".\"pet\"\n\
             WHERE \"p0\".\"name\" IS NOT NULL AND \"p0\".\"pet\" IS NOT NULL"
        );
    }

    #[test]
    fn test_reference_field_used_as_subject_no_extra_left_join() {
        // When a Reference field variable IS later used as a subject,
        // the INNER JOIN from the subject pattern resolves the IRI —
        // no extra LEFT JOIN should be added.
        let schema = schema_with_ref();
        let plan = QueryPlan {
            patterns: vec![
                QueryPattern::Resource {
                    subject: Subject::Variable("p".into()),
                    type_iri: Some(Iri::new("http://example.org/Person")),
                    constraints: vec![FieldConstraint {
                        field_name: "pet".into(),
                        value: Value::Variable("pet".into()),
                    }],
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
                Variable("pet".into()),
                Variable("dname".into()),
            ]),
            modifier: empty_modifier(),
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &schema, "rdf_subject", "id").unwrap();
        // No LEFT JOIN — the INNER JOIN from the subject pattern resolves the IRI.
        assert_eq!(
            sql,
            "SELECT \"p1\".\"rdf_subject\" AS \"pet\", \"p1\".\"name\" AS \"dname\"\n\
             FROM \"Person\" AS \"p0\"\n\
             INNER JOIN \"Duck\" AS \"p1\" ON \"p1\".\"id\" = \"p0\".\"pet\" AND \"p1\".\"name\" IS NOT NULL\n\
             WHERE \"p0\".\"pet\" IS NOT NULL"
        );
    }

    #[test]
    fn test_multi_target_reference_uses_coalesce() {
        use crate::schema::{FieldDescriptor, FieldType, Resource};

        struct Owner;
        impl Resource for Owner {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Owner") }
            fn subject_iri(&self) -> Iri { Iri::new("http://example.org/owner/test") }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![FieldDescriptor {
                    predicate: Iri::new("http://example.org/pet"),
                    name: "pet",
                    field_type: FieldType::Reference(vec![
                        Iri::new("http://example.org/Cat"),
                        Iri::new("http://example.org/Dog"),
                    ]),
                    indexed: false,
                }]
            }
            fn field_values(&self) -> Vec<Term> { vec![] }
        }

        struct Cat;
        impl Resource for Cat {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Cat") }
            fn subject_iri(&self) -> Iri { Iri::new("http://example.org/cat/test") }
            fn field_descriptors() -> Vec<FieldDescriptor> { vec![] }
            fn field_values(&self) -> Vec<Term> { vec![] }
        }

        struct Dog;
        impl Resource for Dog {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Dog") }
            fn subject_iri(&self) -> Iri { Iri::new("http://example.org/dog/test") }
            fn field_descriptors() -> Vec<FieldDescriptor> { vec![] }
            fn field_values(&self) -> Vec<Term> { vec![] }
        }

        let mut schema = Schema::new();
        schema.register::<Owner>();
        schema.register::<Cat>();
        schema.register::<Dog>();

        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("o".into()),
                type_iri: Some(Iri::new("http://example.org/Owner")),
                constraints: vec![FieldConstraint {
                    field_name: "pet".into(),
                    value: Value::Variable("pet".into()),
                }],
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![Variable("pet".into())]),
            modifier: empty_modifier(),
            filters: vec![],
            values: None,
        };

        let sql = to_sql(&plan, &schema, "rdf_subject", "id").unwrap();
        assert_eq!(
            sql,
            "SELECT COALESCE(\"_ref0\".\"rdf_subject\", \"_ref1\".\"rdf_subject\") AS \"pet\"\n\
             FROM \"Owner\" AS \"p0\"\n\
             LEFT JOIN \"Cat\" AS \"_ref0\" ON \"_ref0\".\"id\" = \"p0\".\"pet\"\n\
             LEFT JOIN \"Dog\" AS \"_ref1\" ON \"_ref1\".\"id\" = \"p0\".\"pet\"\n\
             WHERE \"p0\".\"pet\" IS NOT NULL"
        );
    }

    #[test]
    fn test_not_exists_simple() {
        // Outer: ?p a Person, ?p name ?name
        // Filter: NOT EXISTS { ?p age 30 }
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
            filters: vec![NotExistsFilter {
                inner_patterns: vec![QueryPattern::Resource {
                    subject: Subject::Variable("p".into()),
                    type_iri: Some(Iri::new("http://example.org/Person")),
                    constraints: vec![FieldConstraint {
                        field_name: "age".into(),
                        value: Value::Bound(Term::Literal(Literal::Integer(30))),
                    }],
                    type_variable: None,
                }],
            }],
            select: SelectClause::Variables(vec![Variable("name".into())]),
            modifier: empty_modifier(),
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"name\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"name\" IS NOT NULL AND \
             NOT EXISTS (SELECT 1 FROM \"Person\" AS \"_ne0_0\" \
             WHERE \"_ne0_0\".\"age\" = 30 AND \
             \"_ne0_0\".\"_subject\" = \"p0\".\"_subject\")"
        );
    }

    #[test]
    fn test_not_exists_with_correlated_field() {
        // Outer: ?dobj a Person, ?dobj name ?name
        // Filter: NOT EXISTS { ?x a Person, ?x age ?name } (correlate on ?name)
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("dobj".into()),
                type_iri: Some(Iri::new("http://example.org/Person")),
                constraints: vec![FieldConstraint {
                    field_name: "name".into(),
                    value: Value::Variable("name".into()),
                }],
                type_variable: None,
            }],
            filters: vec![NotExistsFilter {
                inner_patterns: vec![QueryPattern::Resource {
                    subject: Subject::Variable("x".into()),
                    type_iri: Some(Iri::new("http://example.org/Person")),
                    constraints: vec![FieldConstraint {
                        field_name: "name".into(),
                        value: Value::Variable("name".into()),
                    }],
                    type_variable: None,
                }],
            }],
            select: SelectClause::Variables(vec![Variable("name".into())]),
            modifier: empty_modifier(),
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        // Inner subject ?x is not in outer bindings → decorrelated to NOT IN
        assert_eq!(
            sql,
            "SELECT \"p0\".\"name\" AS \"name\"\n\
             FROM \"Person\" AS \"p0\"\n\
             WHERE \"p0\".\"name\" IS NOT NULL AND \
             \"p0\".\"name\" NOT IN (\
             SELECT \"name\" FROM \"Person\" WHERE \"name\" IS NOT NULL)"
        );
    }

    #[test]
    fn test_not_exists_decorrelated_array() {
        // Anonymous subject with StringArray field → decorrelated NOT IN with unnest.
        let schema = schema_with_array();
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("s".into()),
                type_iri: Some(Iri::new("http://example.org/Instrument")),
                constraints: vec![FieldConstraint {
                    field_name: "name".into(),
                    value: Value::Variable("name".into()),
                }],
                type_variable: None,
            }],
            filters: vec![NotExistsFilter {
                inner_patterns: vec![QueryPattern::Resource {
                    subject: Subject::Variable("__anon_0".into()),
                    type_iri: Some(Iri::new("http://example.org/Instrument")),
                    constraints: vec![FieldConstraint {
                        field_name: "tags".into(),
                        value: Value::Variable("s".into()),
                    }],
                    type_variable: None,
                }],
            }],
            select: SelectClause::Variables(vec![Variable("name".into())]),
            modifier: empty_modifier(),
            values: None,
        };

        let sql = to_sql(&plan, &schema, "_subject", "_id").unwrap();
        assert!(
            sql.contains("\"p0\".\"_id\" NOT IN (SELECT _elem FROM \"Instrument\", unnest(\"tags\") AS _elem WHERE _elem IS NOT NULL)"),
            "Should decorrelate array NOT EXISTS to NOT IN with unnest: {}",
            sql
        );
    }

    #[test]
    fn test_not_exists_decorrelated_combined() {
        // Two decorrelatable filters on the same outer variable → single NOT IN with UNION ALL.
        let plan = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("dobj".into()),
                type_iri: Some(Iri::new("http://example.org/Person")),
                constraints: vec![FieldConstraint {
                    field_name: "name".into(),
                    value: Value::Variable("name".into()),
                }],
                type_variable: None,
            }],
            filters: vec![
                NotExistsFilter {
                    inner_patterns: vec![QueryPattern::Resource {
                        subject: Subject::Variable("__anon_0".into()),
                        type_iri: Some(Iri::new("http://example.org/Person")),
                        constraints: vec![FieldConstraint {
                            field_name: "age".into(),
                            value: Value::Variable("dobj".into()),
                        }],
                        type_variable: None,
                    }],
                },
                NotExistsFilter {
                    inner_patterns: vec![QueryPattern::Resource {
                        subject: Subject::Variable("__anon_1".into()),
                        type_iri: Some(Iri::new("http://example.org/Person")),
                        constraints: vec![FieldConstraint {
                            field_name: "name".into(),
                            value: Value::Variable("dobj".into()),
                        }],
                        type_variable: None,
                    }],
                },
            ],
            select: SelectClause::Variables(vec![Variable("name".into())]),
            modifier: empty_modifier(),
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        // Both filters target "dobj" → combined into one NOT IN
        assert!(
            sql.contains("UNION ALL"),
            "Should combine two decorrelatable filters with UNION ALL: {}",
            sql
        );
        assert!(
            sql.contains("NOT IN"),
            "Should use NOT IN for decorrelated filters: {}",
            sql
        );
        // Should NOT have any NOT EXISTS
        assert!(
            !sql.contains("NOT EXISTS"),
            "Should not have correlated NOT EXISTS: {}",
            sql
        );
    }

    #[test]
    fn test_not_exists_correlated_subject_not_decorrelated() {
        // Correlated subject (?p in both outer and inner) → must fall back to NOT EXISTS.
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
            filters: vec![NotExistsFilter {
                inner_patterns: vec![QueryPattern::Resource {
                    subject: Subject::Variable("p".into()),
                    type_iri: Some(Iri::new("http://example.org/Person")),
                    constraints: vec![FieldConstraint {
                        field_name: "age".into(),
                        value: Value::Bound(Term::Literal(Literal::Integer(30))),
                    }],
                    type_variable: None,
                }],
            }],
            select: SelectClause::Variables(vec![Variable("name".into())]),
            modifier: empty_modifier(),
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        assert!(
            sql.contains("NOT EXISTS"),
            "Correlated subject should use NOT EXISTS: {}",
            sql
        );
    }

    #[test]
    fn test_not_exists_multi_constraint_not_decorrelated() {
        // Anonymous subject but two constraints → must fall back to NOT EXISTS.
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
            filters: vec![NotExistsFilter {
                inner_patterns: vec![QueryPattern::Resource {
                    subject: Subject::Variable("__anon_0".into()),
                    type_iri: Some(Iri::new("http://example.org/Person")),
                    constraints: vec![
                        FieldConstraint {
                            field_name: "age".into(),
                            value: Value::Variable("p".into()),
                        },
                        FieldConstraint {
                            field_name: "name".into(),
                            value: Value::Bound(Term::Literal(Literal::String("Bob".into()))),
                        },
                    ],
                    type_variable: None,
                }],
            }],
            select: SelectClause::Variables(vec![Variable("name".into())]),
            modifier: empty_modifier(),
            values: None,
        };

        let sql = to_sql(&plan, &Schema::new(), "_subject", "_subject").unwrap();
        assert!(
            sql.contains("NOT EXISTS"),
            "Multiple constraints should fall back to NOT EXISTS: {}",
            sql
        );
    }

    #[test]
    fn test_iri_fragment_table_name() {
        assert_eq!(iri_local_name(&Iri::new("http://xmlns.com/foaf/0.1#Person")), "Person");
    }

    #[test]
    fn test_iri_path_table_name() {
        assert_eq!(iri_local_name(&Iri::new("http://example.org/Person")), "Person");
    }

    #[test]
    fn test_union_sql_combines_plans() {
        let plan_a = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("x".into()),
                type_iri: Some(Iri::new("http://example.org/Alpha")),
                constraints: vec![FieldConstraint {
                    field_name: "name".into(),
                    value: Value::Variable("name".into()),
                }],
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![Variable("x".into())]),
            modifier: SolutionModifier {
                limit: Some(10),
                ..SolutionModifier::default()
            },
            filters: vec![],
            values: None,
        };
        let plan_b = QueryPlan {
            patterns: vec![QueryPattern::Resource {
                subject: Subject::Variable("x".into()),
                type_iri: Some(Iri::new("http://example.org/Beta")),
                constraints: vec![FieldConstraint {
                    field_name: "name".into(),
                    value: Value::Variable("name".into()),
                }],
                type_variable: None,
            }],
            select: SelectClause::Variables(vec![Variable("x".into())]),
            modifier: SolutionModifier {
                limit: Some(10),
                ..SolutionModifier::default()
            },
            filters: vec![],
            values: None,
        };

        let sql = to_union_sql(&[plan_a, plan_b], &Schema::new(), "_subject", "_subject").unwrap();
        // Should contain UNION ALL wrapping two subqueries
        assert!(sql.contains("UNION ALL"), "expected UNION ALL in:\n{}", sql);
        assert!(sql.contains("\"Alpha\""), "expected Alpha table in:\n{}", sql);
        assert!(sql.contains("\"Beta\""), "expected Beta table in:\n{}", sql);
        // LIMIT should be on the outer query, not duplicated in subqueries
        assert!(sql.contains("LIMIT 10"), "expected LIMIT 10 in:\n{}", sql);
        // Only one LIMIT — subqueries should not have it
        assert_eq!(sql.matches("LIMIT").count(), 1, "expected exactly one LIMIT in:\n{}", sql);
    }
}
