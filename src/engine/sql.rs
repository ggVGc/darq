use std::collections::HashMap;
use std::collections::HashSet;

use super::Binding;
use super::Engine;
use crate::error::DarqError;
use crate::ir::{QueryPattern, QueryPlan, Subject, Value};
use crate::rdf::{Iri, Literal, Term, RDF_TYPE};
use crate::schema::{FieldDescriptor, FieldType, Schema};
use crate::sparql::ast::OrderDirection;
use crate::sparql::ast::SolutionModifier;
use crate::sql::sql_literal;

/// Maximum number of values in a single SQL IN clause.
const MAX_IN_CLAUSE_SIZE: usize = 500;

/// Result set returned by a SQL executor.
pub struct SqlResultSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

/// Trait for executing SQL queries against a database backend.
pub trait SqlExecutor {
    fn execute_sql(&self, sql: &str) -> Result<SqlResultSet, DarqError>;
}

/// SQL-backed engine that evaluates query plans by pipelining simple SQL queries.
///
/// Each pattern in the plan is evaluated sequentially. Results from earlier
/// patterns constrain later ones via batched IN clauses, making subsequent
/// queries cheaper.
pub struct SqlEngine<'a, E> {
    executor: &'a E,
}

impl<'a, E: SqlExecutor> SqlEngine<'a, E> {
    pub fn new(executor: &'a E) -> Self {
        Self { executor }
    }
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

impl<E: SqlExecutor> Engine for SqlEngine<'_, E> {
    fn evaluate_plan(
        &self,
        plan: &QueryPlan,
        schema: &Schema,
    ) -> Result<Vec<Binding>, DarqError> {
        let mut solutions: Vec<Binding> = vec![HashMap::new()];

        for pattern in &plan.patterns {
            solutions = match pattern {
                QueryPattern::Resource {
                    subject,
                    type_iri,
                    constraints,
                    type_variable,
                } => self.eval_resource(
                    subject,
                    type_iri,
                    constraints,
                    type_variable,
                    &solutions,
                    schema,
                )?,
                QueryPattern::FieldScan {
                    subject,
                    predicate_var,
                    object,
                    type_iri,
                } => self.eval_field_scan(
                    subject,
                    predicate_var,
                    object,
                    type_iri,
                    &solutions,
                    schema,
                )?,
            };
        }

        Ok(apply_modifiers(solutions, &plan.modifier))
    }
}

impl<E: SqlExecutor> SqlEngine<'_, E> {
    /// Evaluate a Resource pattern against all existing solutions using batched IN clauses.
    fn eval_resource(
        &self,
        subject: &Subject,
        type_iri: &Option<Iri>,
        constraints: &[crate::ir::FieldConstraint],
        type_variable: &Option<String>,
        solutions: &[Binding],
        schema: &Schema,
    ) -> Result<Vec<Binding>, DarqError> {
        if solutions.is_empty() {
            return Ok(Vec::new());
        }

        let type_iris: Vec<&Iri> = match type_iri {
            Some(ti) => vec![ti],
            None => schema.known_types().collect(),
        };

        // Collect bound subject values from solutions for batching
        let subject_var = match subject {
            Subject::Variable(v) => Some(v.as_str()),
            Subject::Bound(_) => None,
        };

        let mut all_bindings = Vec::new();

        for ti in &type_iris {
            let fields = schema.fields_for_type(ti).unwrap_or(&[]);
            let table = table_name(ti);

            // Build SELECT columns
            let mut select_cols = vec!["\"_subject\"".to_string()];
            for c in constraints {
                select_cols.push(format!("\"{}\"", c.field_name));
            }
            let select_str = select_cols.join(", ");

            // Static WHERE parts (bound subject, bound constraint values)
            let mut static_where = Vec::new();
            if let Subject::Bound(iri) = subject {
                static_where.push(format!(
                    "\"_subject\" = '{}'",
                    iri.0.replace('\'', "''")
                ));
            }
            for c in constraints {
                if let Value::Bound(term) = &c.value {
                    static_where.push(format!("\"{}\" = {}", c.field_name, sql_literal(term)));
                }
            }

            // Group solutions by their bound subject value (if any) for batching
            let subject_groups = group_by_subject(solutions, subject_var);

            for group in &subject_groups {
                // Build per-group WHERE parts: static + subject IN + constraint variable IN
                let mut where_parts = static_where.clone();

                if let Some(subject_values) = &group.subject_values {
                    where_parts.push(in_clause("\"_subject\"", subject_values));
                }

                // Add constraint variable filters from the group's common bindings
                // For batched queries, we use the first solution's bindings for
                // constraint variables (they're the same across the group since
                // grouping is by subject only). If they differ, we filter in Rust.
                for c in constraints {
                    if let Value::Variable(v) = &c.value {
                        // Collect distinct bound values for this variable across the group
                        let bound_values: Vec<String> = group
                            .solutions
                            .iter()
                            .filter_map(|s| s.get(v.as_str()))
                            .map(sql_literal)
                            .collect::<std::collections::HashSet<_>>()
                            .into_iter()
                            .collect();
                        if bound_values.len() == 1 {
                            where_parts.push(format!(
                                "\"{}\" = {}",
                                c.field_name, bound_values[0]
                            ));
                        } else if !bound_values.is_empty() {
                            where_parts.push(format!(
                                "\"{}\" IN ({})",
                                c.field_name,
                                bound_values.join(", ")
                            ));
                        }
                    }
                }

                let sql = if where_parts.is_empty() {
                    format!("SELECT {} FROM \"{}\"", select_str, table)
                } else {
                    format!(
                        "SELECT {} FROM \"{}\" WHERE {}",
                        select_str,
                        table,
                        where_parts.join(" AND ")
                    )
                };

                let result = self.executor.execute_sql(&sql)?;

                // Build a lookup from subject IRI to matching solutions
                let solutions_by_subject = index_by_subject(group.solutions, subject_var);

                for row in &result.rows {
                    let col_map: HashMap<&str, &Option<String>> = result
                        .columns
                        .iter()
                        .zip(row.iter())
                        .map(|(c, v)| (c.as_str(), v))
                        .collect();

                    let subj_str = match col_map.get("_subject") {
                        Some(Some(s)) => s.as_str(),
                        _ => continue,
                    };
                    let subj_term = Term::Iri(Iri::new(subj_str));

                    // Find which existing solutions this row matches
                    let default_refs: Vec<&Binding> =
                        group.solutions.iter().collect();
                    let matching: &[&Binding] = match &solutions_by_subject {
                        Some(map) => map
                            .get(subj_str)
                            .map(|v| v.as_slice())
                            .unwrap_or(&[]),
                        None => &default_refs,
                    };

                    for &existing in matching {
                        let mut binding: Binding = existing.clone();
                        let mut ok = true;

                        if let Subject::Variable(v) = subject {
                            if !try_bind(v, &subj_term, &mut binding) {
                                ok = false;
                            }
                        }

                        if ok {
                            if let Some(tv) = type_variable {
                                let term = Term::Iri((*ti).clone());
                                if !try_bind(tv, &term, &mut binding) {
                                    ok = false;
                                }
                            }
                        }

                        if ok {
                            for c in constraints {
                                if let Value::Variable(v) = &c.value {
                                    if let Some(Some(raw)) = col_map.get(c.field_name.as_str()) {
                                        let fd = fields.iter().find(|f| f.name == c.field_name);
                                        let term = match fd {
                                            Some(fd) => parse_sql_value(raw, &fd.field_type),
                                            None => {
                                                Term::Literal(Literal::String(raw.clone()))
                                            }
                                        };
                                        if !try_bind(v, &term, &mut binding) {
                                            ok = false;
                                            break;
                                        }
                                    } else {
                                        ok = false;
                                        break;
                                    }
                                }
                            }
                        }

                        if ok {
                            all_bindings.push(binding);
                        }
                    }
                }
            }
        }

        Ok(all_bindings)
    }

    /// Evaluate a FieldScan pattern using batched IN clauses and indexed field pipelining.
    fn eval_field_scan(
        &self,
        subject: &Subject,
        predicate_var: &str,
        object: &Value,
        type_iri: &Option<Iri>,
        solutions: &[Binding],
        schema: &Schema,
    ) -> Result<Vec<Binding>, DarqError> {
        if solutions.is_empty() {
            return Ok(Vec::new());
        }

        let type_iris: Vec<&Iri> = match type_iri {
            Some(ti) => vec![ti],
            None => schema.known_types().collect(),
        };

        let subject_var = match subject {
            Subject::Variable(v) => Some(v.as_str()),
            Subject::Bound(_) => None,
        };

        let mut all_bindings = Vec::new();

        for ti in &type_iris {
            let fields = schema.fields_for_type(ti).unwrap_or(&[]);
            let table = table_name(ti);

            let subject_groups = group_by_subject(solutions, subject_var);

            for group in &subject_groups {
                let subject_in = group
                    .subject_values
                    .as_ref()
                    .map(|vals| in_clause("\"_subject\"", vals));

                // Determine if the object is constrained (bound or variable already bound)
                let object_is_bound = match object {
                    Value::Bound(_) => true,
                    Value::Variable(v) => group.solutions.iter().any(|s| s.contains_key(v.as_str())),
                };

                // Partition fields into probe fields (indexed + object bound) and rest
                let (probe_fields, rest_fields): (Vec<&FieldDescriptor>, Vec<&FieldDescriptor>) =
                    fields.iter().partition(|fd| fd.indexed && object_is_bound);

                // Phase 1: Query probe fields to narrow subjects
                let mut narrowed_subjects: Option<Vec<String>> = None;

                for fd in &probe_fields {
                    let pred_term = Term::Iri(fd.predicate.clone());

                    let field_bindings = self.exec_field_query(
                        table,
                        fd,
                        &pred_term,
                        subject,
                        predicate_var,
                        object,
                        &subject_in,
                        &narrowed_subjects,
                        group.solutions,
                        subject_var,
                    )?;

                    // Collect subjects from probe results and intersect
                    let result_subjects: Vec<String> = field_bindings
                        .iter()
                        .filter_map(|b| match subject {
                            Subject::Variable(v) => b.get(v.as_str()),
                            Subject::Bound(_) => None,
                        })
                        .filter_map(|t| match t {
                            Term::Iri(iri) => Some(iri.0.clone()),
                            _ => None,
                        })
                        .collect::<std::collections::HashSet<_>>()
                        .into_iter()
                        .collect();

                    if !result_subjects.is_empty() {
                        narrowed_subjects = Some(match narrowed_subjects {
                            Some(existing) => existing
                                .into_iter()
                                .filter(|s| result_subjects.contains(s))
                                .collect(),
                            None => result_subjects,
                        });
                    } else if !probe_fields.is_empty() {
                        // Probe returned nothing — no results possible
                        narrowed_subjects = Some(Vec::new());
                    }

                    all_bindings.extend(field_bindings);
                }

                // If probe narrowed to empty set, skip remaining fields
                if let Some(ref ns) = narrowed_subjects {
                    if ns.is_empty() {
                        // Still need to process synthetic rdf:type — but no subjects match
                        continue;
                    }
                }

                // Build effective subject filter: combine group's subject IN with narrowed subjects
                let effective_subject_in = match (&subject_in, &narrowed_subjects) {
                    (_, Some(narrowed)) => {
                        let quoted: Vec<String> =
                            narrowed.iter().map(|s| format!("'{}'", s.replace('\'', "''"))).collect();
                        if quoted.is_empty() {
                            continue;
                        }
                        Some(format!("\"_subject\" IN ({})", quoted.join(", ")))
                    }
                    (Some(existing), None) => Some(existing.clone()),
                    (None, None) => None,
                };

                // Synthetic rdf:type field
                {
                    let pred_term = Term::Iri(Iri::new(RDF_TYPE));
                    let obj_term = Term::Iri((*ti).clone());

                    // Check predicate compatibility across solutions
                    let pred_compatible = group.solutions.iter().all(|s| {
                        match s.get(predicate_var) {
                            Some(existing_pred) => *existing_pred == pred_term,
                            None => true,
                        }
                    });
                    let any_pred_bound = group.solutions.iter().any(|s| s.contains_key(predicate_var));
                    let skip_rdf_type = any_pred_bound && !pred_compatible;

                    if !skip_rdf_type {
                        // Check object compatibility
                        let obj_ok = match object {
                            Value::Bound(expected) => *expected == obj_term,
                            Value::Variable(v) => group.solutions.iter().all(|s| {
                                match s.get(v.as_str()) {
                                    Some(val) => *val == obj_term,
                                    None => true,
                                }
                            }),
                        };

                        if obj_ok {
                            let sql = match &effective_subject_in {
                                Some(f) => format!(
                                    "SELECT \"_subject\" FROM \"{}\" WHERE {}",
                                    table, f
                                ),
                                None => format!("SELECT \"_subject\" FROM \"{}\"", table),
                            };

                            let result = self.executor.execute_sql(&sql)?;
                            let solutions_by_subject =
                                index_by_subject(group.solutions, subject_var);

                            for row in &result.rows {
                                let subj_str = match &row[0] {
                                    Some(s) => s.as_str(),
                                    None => continue,
                                };
                                let subj_term = Term::Iri(Iri::new(subj_str));

                                let default_refs: Vec<&Binding> =
                                    group.solutions.iter().collect();
                                let matching: &[&Binding] = match &solutions_by_subject {
                                    Some(map) => map
                                        .get(subj_str)
                                        .map(|v| v.as_slice())
                                        .unwrap_or(&[]),
                                    None => &default_refs,
                                };

                                for &existing in matching {
                                    let mut binding: Binding = existing.clone();
                                    let mut ok = true;

                                    if let Subject::Variable(v) = subject {
                                        if !try_bind(v, &subj_term, &mut binding) {
                                            ok = false;
                                        }
                                    }
                                    if ok && !try_bind(predicate_var, &pred_term, &mut binding) {
                                        ok = false;
                                    }
                                    if ok {
                                        if let Value::Variable(v) = object {
                                            if !try_bind(v, &obj_term, &mut binding) {
                                                ok = false;
                                            }
                                        }
                                    }
                                    if ok {
                                        all_bindings.push(binding);
                                    }
                                }
                            }
                        }
                    }
                }

                // Phase 2: Query remaining (non-probe) fields
                for fd in &rest_fields {
                    let pred_term = Term::Iri(fd.predicate.clone());

                    let field_bindings = self.exec_field_query(
                        table,
                        fd,
                        &pred_term,
                        subject,
                        predicate_var,
                        object,
                        &effective_subject_in,
                        &None, // narrowed subjects already folded into effective_subject_in
                        group.solutions,
                        subject_var,
                    )?;

                    all_bindings.extend(field_bindings);
                }
            }
        }

        Ok(all_bindings)
    }

    /// Execute a single field query within a FieldScan and produce bindings.
    fn exec_field_query(
        &self,
        table: &str,
        fd: &FieldDescriptor,
        pred_term: &Term,
        subject: &Subject,
        predicate_var: &str,
        object: &Value,
        subject_in: &Option<String>,
        narrowed_subjects: &Option<Vec<String>>,
        solutions: &[Binding],
        subject_var: Option<&str>,
    ) -> Result<Vec<Binding>, DarqError> {
        // Check predicate compatibility
        let any_pred_bound = solutions.iter().any(|s| s.contains_key(predicate_var));
        if any_pred_bound {
            let all_match = solutions.iter().all(|s| {
                match s.get(predicate_var) {
                    Some(p) => *p == *pred_term,
                    None => true,
                }
            });
            if !all_match {
                return Ok(Vec::new());
            }
        }

        let mut where_parts = Vec::new();

        // Subject filter
        if let Some(narrowed) = narrowed_subjects {
            let quoted: Vec<String> =
                narrowed.iter().map(|s| format!("'{}'", s.replace('\'', "''"))).collect();
            if quoted.is_empty() {
                return Ok(Vec::new());
            }
            where_parts.push(format!("\"_subject\" IN ({})", quoted.join(", ")));
        } else if let Some(subj_in) = subject_in {
            where_parts.push(subj_in.clone());
        }
        if let Subject::Bound(iri) = subject {
            where_parts.push(format!(
                "\"_subject\" = '{}'",
                iri.0.replace('\'', "''")
            ));
        }

        // Object filter
        match object {
            Value::Bound(term) => {
                where_parts.push(format!("\"{}\" = {}", fd.name, sql_literal(term)));
            }
            Value::Variable(v) => {
                let bound_values: Vec<String> = solutions
                    .iter()
                    .filter_map(|s| s.get(v.as_str()))
                    .map(sql_literal)
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                if bound_values.len() == 1 {
                    where_parts.push(format!("\"{}\" = {}", fd.name, bound_values[0]));
                } else if !bound_values.is_empty() {
                    where_parts.push(format!(
                        "\"{}\" IN ({})",
                        fd.name,
                        bound_values.join(", ")
                    ));
                }
            }
        }

        let sql = if where_parts.is_empty() {
            format!("SELECT \"_subject\", \"{}\" FROM \"{}\"", fd.name, table)
        } else {
            format!(
                "SELECT \"_subject\", \"{}\" FROM \"{}\" WHERE {}",
                fd.name, table, where_parts.join(" AND ")
            )
        };

        let result = self.executor.execute_sql(&sql)?;
        let solutions_by_subject = index_by_subject(solutions, subject_var);

        let mut bindings = Vec::new();

        for row in &result.rows {
            let subj_str = match &row[0] {
                Some(s) => s.as_str(),
                None => continue,
            };
            let obj_str = match &row[1] {
                Some(s) => s,
                None => continue,
            };

            let subj_term = Term::Iri(Iri::new(subj_str));
            let obj_term = parse_sql_value(obj_str, &fd.field_type);

            let default_refs: Vec<&Binding> = solutions.iter().collect();
            let matching: &[&Binding] = match &solutions_by_subject {
                Some(map) => map.get(subj_str).map(|v| v.as_slice()).unwrap_or(&[]),
                None => &default_refs,
            };

            for &existing in matching {
                let mut binding: Binding = existing.clone();
                let mut ok = true;

                if let Subject::Variable(v) = subject {
                    if !try_bind(v, &subj_term, &mut binding) {
                        ok = false;
                    }
                }
                if ok && !try_bind(predicate_var, pred_term, &mut binding) {
                    ok = false;
                }
                if ok {
                    match object {
                        Value::Variable(v) => {
                            if !try_bind(v, &obj_term, &mut binding) {
                                ok = false;
                            }
                        }
                        Value::Bound(expected) => {
                            if *expected != obj_term {
                                ok = false;
                            }
                        }
                    }
                }
                if ok {
                    bindings.push(binding);
                }
            }
        }

        Ok(bindings)
    }
}

/// A group of solutions that share the same subject values (or all have unbound subjects).
struct SubjectGroup<'a> {
    solutions: &'a [Binding],
    /// The subject IRI strings for the IN clause, or None if subject is not bound.
    subject_values: Option<Vec<String>>,
}

/// Group solutions by their bound subject value, chunking into MAX_IN_CLAUSE_SIZE groups.
fn group_by_subject<'a>(
    solutions: &'a [Binding],
    subject_var: Option<&str>,
) -> Vec<SubjectGroup<'a>> {
    let subject_var = match subject_var {
        Some(v) => v,
        None => {
            // Bound subject or no subject variable — one group, no IN clause
            return vec![SubjectGroup {
                solutions,
                subject_values: None,
            }];
        }
    };

    // Check if any solution has the subject bound
    let any_bound = solutions.iter().any(|s| s.contains_key(subject_var));
    if !any_bound {
        return vec![SubjectGroup {
            solutions,
            subject_values: None,
        }];
    }

    // Collect distinct subject values
    let subject_iris: Vec<String> = solutions
        .iter()
        .filter_map(|s| match s.get(subject_var) {
            Some(Term::Iri(iri)) => Some(iri.0.clone()),
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    // Chunk into groups of MAX_IN_CLAUSE_SIZE
    if subject_iris.len() <= MAX_IN_CLAUSE_SIZE {
        vec![SubjectGroup {
            solutions,
            subject_values: Some(subject_iris),
        }]
    } else {
        subject_iris
            .chunks(MAX_IN_CLAUSE_SIZE)
            .map(|chunk| {
                SubjectGroup {
                    solutions,
                    subject_values: Some(chunk.to_vec()),
                }
            })
            .collect()
    }
}

/// Build a SQL IN clause from a list of IRI strings.
fn in_clause(column: &str, values: &[String]) -> String {
    let quoted: Vec<String> = values
        .iter()
        .map(|v| format!("'{}'", v.replace('\'', "''")))
        .collect();
    if quoted.len() == 1 {
        format!("{} = {}", column, quoted[0])
    } else {
        format!("{} IN ({})", column, quoted.join(", "))
    }
}

/// Build a HashMap from subject IRI string to matching solutions.
/// Returns None if no subject variable is bound (all solutions match any subject).
fn index_by_subject<'a>(
    solutions: &'a [Binding],
    subject_var: Option<&str>,
) -> Option<HashMap<&'a str, Vec<&'a Binding>>> {
    let var = subject_var?;
    let any_bound = solutions.iter().any(|s| s.contains_key(var));
    if !any_bound {
        return None;
    }

    let mut map: HashMap<&str, Vec<&Binding>> = HashMap::new();
    for s in solutions {
        if let Some(Term::Iri(iri)) = s.get(var) {
            map.entry(iri.0.as_str()).or_default().push(s);
        }
    }
    Some(map)
}

/// Try to bind a variable to a term, checking for conflicts with existing bindings.
/// Returns false if there is a conflict.
fn try_bind(var: &str, term: &Term, binding: &mut Binding) -> bool {
    if let Some(existing) = binding.get(var) {
        *existing == *term
    } else {
        binding.insert(var.to_string(), term.clone());
        true
    }
}

/// Extract the local name from an IRI for use as a table name.
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

/// Convert a SQL string value to a typed Term based on schema field type info.
fn parse_sql_value(raw: &str, field_type: &FieldType) -> Term {
    match field_type {
        FieldType::String => Term::Literal(Literal::String(raw.to_string())),
        FieldType::Integer => {
            let n: i64 = raw.parse().unwrap_or(0);
            Term::Literal(Literal::Integer(n))
        }
        FieldType::Boolean => {
            let b = matches!(raw, "TRUE" | "true" | "t" | "1");
            Term::Literal(Literal::Boolean(b))
        }
        FieldType::Float => {
            let v: f64 = raw.parse().unwrap_or(0.0);
            Term::Literal(Literal::Float(crate::rdf::Float64(v)))
        }
        FieldType::Double => {
            let v: f64 = raw.parse().unwrap_or(0.0);
            Term::Literal(Literal::Double(crate::rdf::Float64(v)))
        }
        FieldType::Decimal => Term::Literal(Literal::Decimal(raw.to_string())),
        FieldType::Date => Term::Literal(Literal::Date(raw.to_string())),
        FieldType::DateTime => Term::Literal(Literal::DateTime(raw.to_string())),
        FieldType::Reference(_) => Term::Iri(Iri::new(raw)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{FieldConstraint, QueryPlan, Subject, Value};
    use crate::rdf::{Iri, Literal, Term};
    use crate::schema::{FieldDescriptor, FieldType, Resource, Schema};
    use crate::sparql::ast::{SelectClause, SolutionModifier, Variable};
    use std::cell::RefCell;

    /// Mock SQL executor that records queries and returns canned results.
    struct MockExecutor {
        /// Canned responses keyed by SQL substring match.
        responses: Vec<(String, SqlResultSet)>,
        /// Queries that were executed, for assertions.
        queries: RefCell<Vec<String>>,
    }

    impl MockExecutor {
        fn new() -> Self {
            Self {
                responses: Vec::new(),
                queries: RefCell::new(Vec::new()),
            }
        }

        fn add_response(&mut self, sql_contains: &str, result: SqlResultSet) {
            self.responses.push((sql_contains.to_string(), result));
        }

        fn executed_queries(&self) -> Vec<String> {
            self.queries.borrow().clone()
        }
    }

    impl SqlExecutor for MockExecutor {
        fn execute_sql(&self, sql: &str) -> Result<SqlResultSet, DarqError> {
            self.queries.borrow_mut().push(sql.to_string());

            for (pattern, result) in &self.responses {
                if sql.contains(pattern.as_str()) {
                    let cloned = SqlResultSet {
                        columns: result.columns.clone(),
                        rows: result.rows.clone(),
                    };
                    return Ok(cloned);
                }
            }

            // Default: empty result
            Ok(SqlResultSet {
                columns: vec![],
                rows: vec![],
            })
        }
    }

    fn test_schema() -> Schema {
        struct Person;
        impl Resource for Person {
            fn rdf_type() -> Iri {
                Iri::new("http://example.org/Person")
            }
            fn subject_iri(&self) -> Iri {
                Iri::new("http://example.org/person/test")
            }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/name"),
                        name: "name",
                        field_type: FieldType::String,
                        indexed: false,
                    },
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/age"),
                        name: "age",
                        field_type: FieldType::Integer,
                        indexed: false,
                    },
                ]
            }
            fn field_values(&self) -> Vec<Term> {
                vec![]
            }
        }

        let mut schema = Schema::new();
        schema.register::<Person>();
        schema
    }

    fn test_schema_with_indexed_name() -> Schema {
        struct Person;
        impl Resource for Person {
            fn rdf_type() -> Iri {
                Iri::new("http://example.org/Person")
            }
            fn subject_iri(&self) -> Iri {
                Iri::new("http://example.org/person/test")
            }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/name"),
                        name: "name",
                        field_type: FieldType::String,
                        indexed: true,
                    },
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/age"),
                        name: "age",
                        field_type: FieldType::Integer,
                        indexed: false,
                    },
                ]
            }
            fn field_values(&self) -> Vec<Term> {
                vec![]
            }
        }

        let mut schema = Schema::new();
        schema.register::<Person>();
        schema
    }

    #[test]
    fn test_resource_pattern_generates_correct_sql() {
        let mut executor = MockExecutor::new();
        executor.add_response("FROM \"Person\"", SqlResultSet {
            columns: vec!["_subject".into(), "name".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into()), Some("Alice".into())],
            ],
        });

        let engine = SqlEngine::new(&executor);
        let schema = test_schema();

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
            modifier: SolutionModifier::default(),
        };

        let bindings = engine.evaluate_plan(&plan, &schema).unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(
            bindings[0].get("name"),
            Some(&Term::Literal(Literal::String("Alice".into())))
        );
        assert_eq!(
            bindings[0].get("p"),
            Some(&Term::Iri(Iri::new("http://example.org/person/alice")))
        );

        let queries = executor.executed_queries();
        assert_eq!(queries.len(), 1);
        assert!(queries[0].contains("FROM \"Person\""));
    }

    #[test]
    fn test_resource_pattern_with_bound_subject() {
        let mut executor = MockExecutor::new();
        executor.add_response("FROM \"Person\"", SqlResultSet {
            columns: vec!["_subject".into(), "name".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into()), Some("Alice".into())],
            ],
        });

        let engine = SqlEngine::new(&executor);
        let schema = test_schema();

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
            modifier: SolutionModifier::default(),
        };

        let bindings = engine.evaluate_plan(&plan, &schema).unwrap();
        assert_eq!(bindings.len(), 1);

        let queries = executor.executed_queries();
        assert!(queries[0].contains("_subject"));
        assert!(queries[0].contains("alice"));
    }

    #[test]
    fn test_field_scan_generates_per_field_queries() {
        let mut executor = MockExecutor::new();

        // Query for synthetic rdf:type
        executor.add_response("SELECT \"_subject\" FROM \"Person\"", SqlResultSet {
            columns: vec!["_subject".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into())],
            ],
        });

        // Query for name field
        executor.add_response("\"name\" FROM \"Person\"", SqlResultSet {
            columns: vec!["_subject".into(), "name".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into()), Some("Alice".into())],
            ],
        });

        // Query for age field
        executor.add_response("\"age\" FROM \"Person\"", SqlResultSet {
            columns: vec!["_subject".into(), "age".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into()), Some("30".into())],
            ],
        });

        let engine = SqlEngine::new(&executor);
        let schema = test_schema();

        let plan = QueryPlan {
            patterns: vec![QueryPattern::FieldScan {
                subject: Subject::Variable("s".into()),
                predicate_var: "p".into(),
                object: Value::Variable("o".into()),
                type_iri: Some(Iri::new("http://example.org/Person")),
            }],
            select: SelectClause::Star,
            modifier: SolutionModifier::default(),
        };

        let bindings = engine.evaluate_plan(&plan, &schema).unwrap();

        // Should have 3 rows: rdf:type + name + age
        assert_eq!(bindings.len(), 3);

        // Verify queries were generated (rdf:type query + 2 field queries)
        let queries = executor.executed_queries();
        assert_eq!(queries.len(), 3);
    }

    #[test]
    fn test_pipelining_constrains_subsequent_queries() {
        let mut executor = MockExecutor::new();

        // First pattern: Resource query returns alice
        executor.add_response("FROM \"Person\" WHERE", SqlResultSet {
            columns: vec!["_subject".into(), "name".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into()), Some("Alice".into())],
            ],
        });

        // Second pattern: FieldScan queries should contain alice constraint
        executor.add_response("SELECT \"_subject\" FROM \"Person\"", SqlResultSet {
            columns: vec!["_subject".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into())],
            ],
        });
        executor.add_response("\"name\" FROM \"Person\"", SqlResultSet {
            columns: vec!["_subject".into(), "name".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into()), Some("Alice".into())],
            ],
        });
        executor.add_response("\"age\" FROM \"Person\"", SqlResultSet {
            columns: vec!["_subject".into(), "age".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into()), Some("30".into())],
            ],
        });

        let engine = SqlEngine::new(&executor);
        let schema = test_schema();

        let plan = QueryPlan {
            patterns: vec![
                QueryPattern::Resource {
                    subject: Subject::Variable("s".into()),
                    type_iri: Some(Iri::new("http://example.org/Person")),
                    constraints: vec![FieldConstraint {
                        field_name: "name".into(),
                        value: Value::Bound(Term::Literal(Literal::String("Alice".into()))),
                    }],
                    type_variable: None,
                },
                QueryPattern::FieldScan {
                    subject: Subject::Variable("s".into()),
                    predicate_var: "p".into(),
                    object: Value::Variable("o".into()),
                    type_iri: Some(Iri::new("http://example.org/Person")),
                },
            ],
            select: SelectClause::Star,
            modifier: SolutionModifier::default(),
        };

        let bindings = engine.evaluate_plan(&plan, &schema).unwrap();

        // All bindings should have ?s bound to alice
        for b in &bindings {
            assert_eq!(
                b.get("s"),
                Some(&Term::Iri(Iri::new("http://example.org/person/alice")))
            );
        }

        // FieldScan queries should have subject filter from pipelining
        let queries = executor.executed_queries();
        for q in &queries[1..] {
            assert!(
                q.contains("alice"),
                "FieldScan query should contain subject constraint from pipelining: {}",
                q
            );
        }
    }

    #[test]
    fn test_batch_in_clause_for_multiple_subjects() {
        let mut executor = MockExecutor::new();

        // First pattern returns two subjects
        executor.add_response("FROM \"Person\"", SqlResultSet {
            columns: vec!["_subject".into(), "name".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into()), Some("Alice".into())],
                vec![Some("http://example.org/person/bob".into()), Some("Bob".into())],
            ],
        });

        // Second pattern: FieldScan — should use IN clause
        executor.add_response("SELECT \"_subject\" FROM", SqlResultSet {
            columns: vec!["_subject".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into())],
                vec![Some("http://example.org/person/bob".into())],
            ],
        });
        executor.add_response("\"age\" FROM", SqlResultSet {
            columns: vec!["_subject".into(), "age".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into()), Some("30".into())],
                vec![Some("http://example.org/person/bob".into()), Some("25".into())],
            ],
        });
        executor.add_response("\"name\" FROM", SqlResultSet {
            columns: vec!["_subject".into(), "name".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into()), Some("Alice".into())],
                vec![Some("http://example.org/person/bob".into()), Some("Bob".into())],
            ],
        });

        let engine = SqlEngine::new(&executor);
        let schema = test_schema();

        let plan = QueryPlan {
            patterns: vec![
                QueryPattern::Resource {
                    subject: Subject::Variable("s".into()),
                    type_iri: Some(Iri::new("http://example.org/Person")),
                    constraints: vec![FieldConstraint {
                        field_name: "name".into(),
                        value: Value::Variable("name".into()),
                    }],
                    type_variable: None,
                },
                QueryPattern::FieldScan {
                    subject: Subject::Variable("s".into()),
                    predicate_var: "p".into(),
                    object: Value::Variable("o".into()),
                    type_iri: Some(Iri::new("http://example.org/Person")),
                },
            ],
            select: SelectClause::Star,
            modifier: SolutionModifier::default(),
        };

        let bindings = engine.evaluate_plan(&plan, &schema).unwrap();

        // 2 subjects x 3 fields (rdf:type + name + age) = 6 bindings
        assert_eq!(bindings.len(), 6);

        // FieldScan should use IN clause, not individual queries
        let queries = executor.executed_queries();
        // Should be: 1 Resource query + 3 FieldScan queries (not 2*3=6)
        assert_eq!(queries.len(), 4, "Expected 4 queries (1 Resource + 3 FieldScan), got {}: {:?}", queries.len(), queries);

        // The FieldScan queries should contain IN clause with both subjects
        for q in &queries[1..] {
            assert!(
                q.contains("IN") || (q.contains("alice") && q.contains("bob")),
                "FieldScan query should batch subjects: {}",
                q
            );
        }
    }

    #[test]
    fn test_indexed_field_probed_first_in_field_scan() {
        let mut executor = MockExecutor::new();

        // Indexed name field query (probed first because object is bound)
        executor.add_response("\"name\" FROM \"Person\"", SqlResultSet {
            columns: vec!["_subject".into(), "name".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into()), Some("Alice".into())],
            ],
        });

        // Non-indexed age field query (should be constrained by probe results)
        executor.add_response("\"age\" FROM \"Person\"", SqlResultSet {
            columns: vec!["_subject".into(), "age".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into()), Some("30".into())],
            ],
        });

        // rdf:type query
        executor.add_response("SELECT \"_subject\" FROM \"Person\"", SqlResultSet {
            columns: vec!["_subject".into()],
            rows: vec![
                vec![Some("http://example.org/person/alice".into())],
            ],
        });

        let engine = SqlEngine::new(&executor);
        let schema = test_schema_with_indexed_name();

        let plan = QueryPlan {
            patterns: vec![QueryPattern::FieldScan {
                subject: Subject::Variable("s".into()),
                predicate_var: "p".into(),
                object: Value::Bound(Term::Literal(Literal::String("Alice".into()))),
                type_iri: Some(Iri::new("http://example.org/Person")),
            }],
            select: SelectClause::Star,
            modifier: SolutionModifier::default(),
        };

        let _bindings = engine.evaluate_plan(&plan, &schema).unwrap();

        let queries = executor.executed_queries();

        // First query should be the indexed name field (probe)
        assert!(
            queries[0].contains("\"name\""),
            "First query should probe indexed name field: {}",
            queries[0]
        );

        // Subsequent queries should have subject constraint from probe
        for q in &queries[1..] {
            assert!(
                q.contains("alice"),
                "Non-probe query should be constrained by probe results: {}",
                q
            );
        }
    }

    #[test]
    fn test_parse_sql_value_types() {
        assert_eq!(
            parse_sql_value("hello", &FieldType::String),
            Term::Literal(Literal::String("hello".into()))
        );
        assert_eq!(
            parse_sql_value("42", &FieldType::Integer),
            Term::Literal(Literal::Integer(42))
        );
        assert_eq!(
            parse_sql_value("true", &FieldType::Boolean),
            Term::Literal(Literal::Boolean(true))
        );
        assert_eq!(
            parse_sql_value("FALSE", &FieldType::Boolean),
            Term::Literal(Literal::Boolean(false))
        );
        assert_eq!(
            parse_sql_value("http://example.org/foo", &FieldType::Reference(vec![])),
            Term::Iri(Iri::new("http://example.org/foo"))
        );
    }
}
