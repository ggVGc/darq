use std::collections::HashMap;

use super::Binding;
use super::Engine;
use crate::error::DarqError;
use crate::ir::{QueryPattern, QueryPlan, Subject, Value};
use crate::rdf::{Iri, Literal, Term, RDF_TYPE};
use crate::schema::{FieldType, Schema};
use crate::sql::sql_literal;

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
/// patterns constrain later ones via WHERE filters, making subsequent queries
/// cheaper.
pub struct SqlEngine<'a, E> {
    executor: &'a E,
}

impl<'a, E: SqlExecutor> SqlEngine<'a, E> {
    pub fn new(executor: &'a E) -> Self {
        Self { executor }
    }
}

impl<E: SqlExecutor> Engine for SqlEngine<'_, E> {
    fn evaluate_plan(
        &self,
        plan: &QueryPlan,
        schema: &Schema,
    ) -> Result<Vec<Binding>, DarqError> {
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
                        let bindings =
                            self.eval_resource(subject, type_iri, constraints, type_variable, existing, schema)?;
                        next_solutions.extend(bindings);
                    }
                    QueryPattern::FieldScan {
                        subject,
                        predicate_var,
                        object,
                        type_iri,
                    } => {
                        let bindings =
                            self.eval_field_scan(subject, predicate_var, object, type_iri, existing, schema)?;
                        next_solutions.extend(bindings);
                    }
                }
            }

            solutions = next_solutions;
        }

        Ok(solutions)
    }
}

impl<E: SqlExecutor> SqlEngine<'_, E> {
    /// Evaluate a Resource pattern by generating and executing a single SQL query.
    fn eval_resource(
        &self,
        subject: &Subject,
        type_iri: &Option<Iri>,
        constraints: &[crate::ir::FieldConstraint],
        type_variable: &Option<String>,
        existing: &Binding,
        schema: &Schema,
    ) -> Result<Vec<Binding>, DarqError> {
        let type_iris: Vec<&Iri> = match type_iri {
            Some(ti) => vec![ti],
            None => schema.known_types().collect(),
        };

        let mut all_bindings = Vec::new();

        for ti in &type_iris {
            let fields = schema.fields_for_type(ti).unwrap_or(&[]);
            let table = table_name(ti);

            // Build SELECT columns: always _subject, plus constraint fields
            let mut select_cols = vec!["\"_subject\"".to_string()];
            for c in constraints {
                select_cols.push(format!("\"{}\"", c.field_name));
            }

            let mut where_parts = Vec::new();

            // Subject constraint from existing bindings
            if let Subject::Variable(v) = subject {
                if let Some(term) = existing.get(v) {
                    where_parts.push(format!("\"_subject\" = {}", sql_literal(term)));
                }
            } else if let Subject::Bound(iri) = subject {
                where_parts.push(format!("\"_subject\" = '{}'", iri.0.replace('\'', "''")));
            }

            // Field constraints from existing bindings or bound values
            for c in constraints {
                match &c.value {
                    Value::Bound(term) => {
                        where_parts.push(format!("\"{}\" = {}", c.field_name, sql_literal(term)));
                    }
                    Value::Variable(v) => {
                        if let Some(term) = existing.get(v) {
                            where_parts
                                .push(format!("\"{}\" = {}", c.field_name, sql_literal(term)));
                        }
                    }
                }
            }

            let sql = if where_parts.is_empty() {
                format!(
                    "SELECT {} FROM \"{}\"",
                    select_cols.join(", "),
                    table
                )
            } else {
                format!(
                    "SELECT {} FROM \"{}\" WHERE {}",
                    select_cols.join(", "),
                    table,
                    where_parts.join(" AND ")
                )
            };

            let result = self.executor.execute_sql(&sql)?;

            // Convert rows to bindings
            for row in &result.rows {
                let col_map: HashMap<&str, &Option<String>> = result
                    .columns
                    .iter()
                    .zip(row.iter())
                    .map(|(c, v)| (c.as_str(), v))
                    .collect();

                let mut binding = existing.clone();
                let mut ok = true;

                // Bind subject
                if let Some(Some(subj_str)) = col_map.get("_subject") {
                    let term = Term::Iri(Iri::new(subj_str.as_str()));
                    if let Subject::Variable(v) = subject {
                        if !try_bind(v, &term, &mut binding) {
                            ok = false;
                        }
                    }
                } else {
                    ok = false;
                }

                // Bind type variable
                if ok {
                    if let Some(tv) = type_variable {
                        let term = Term::Iri((*ti).clone());
                        if !try_bind(tv, &term, &mut binding) {
                            ok = false;
                        }
                    }
                }

                // Bind constraint variables
                if ok {
                    for c in constraints {
                        if let Value::Variable(v) = &c.value {
                            if let Some(Some(raw)) = col_map.get(c.field_name.as_str()) {
                                let fd = fields.iter().find(|f| f.name == c.field_name);
                                let term = match fd {
                                    Some(fd) => parse_sql_value(raw, &fd.field_type),
                                    None => Term::Literal(Literal::String(raw.clone())),
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

        Ok(all_bindings)
    }

    /// Evaluate a FieldScan pattern by generating one query per (type, field).
    fn eval_field_scan(
        &self,
        subject: &Subject,
        predicate_var: &str,
        object: &Value,
        type_iri: &Option<Iri>,
        existing: &Binding,
        schema: &Schema,
    ) -> Result<Vec<Binding>, DarqError> {
        let type_iris: Vec<&Iri> = match type_iri {
            Some(ti) => vec![ti],
            None => schema.known_types().collect(),
        };

        let mut all_bindings = Vec::new();

        for ti in &type_iris {
            let fields = schema.fields_for_type(ti).unwrap_or(&[]);
            let table = table_name(ti);

            // Build subject WHERE clause (shared across all field queries for this type)
            let subject_filter = subject_where_clause(subject, existing);

            // Synthetic rdf:type field
            {
                let pred_term = Term::Iri(Iri::new(RDF_TYPE));
                let obj_term = Term::Iri((*ti).clone());

                // Check if predicate var is already bound and conflicts
                if let Some(existing_pred) = existing.get(predicate_var) {
                    if *existing_pred != pred_term {
                        // Skip this synthetic field — predicate doesn't match
                    } else {
                        self.exec_field_scan_query(
                            &table,
                            &subject_filter,
                            subject,
                            predicate_var,
                            &pred_term,
                            object,
                            &obj_term,
                            None, // rdf:type object is an IRI, not a field value
                            existing,
                            &mut all_bindings,
                        )?;
                    }
                } else {
                    self.exec_field_scan_query(
                        &table,
                        &subject_filter,
                        subject,
                        predicate_var,
                        &pred_term,
                        object,
                        &obj_term,
                        None,
                        existing,
                        &mut all_bindings,
                    )?;
                }
            }

            // Real fields
            for fd in fields {
                let pred_term = Term::Iri(fd.predicate.clone());

                // Check if predicate var is already bound and conflicts
                if let Some(existing_pred) = existing.get(predicate_var) {
                    if *existing_pred != pred_term {
                        continue;
                    }
                }

                // Build object filter
                let mut where_parts = Vec::new();
                if let Some(f) = &subject_filter {
                    where_parts.push(f.clone());
                }

                // If object is bound or variable is already bound, add WHERE filter
                match object {
                    Value::Bound(term) => {
                        where_parts
                            .push(format!("\"{}\" = {}", fd.name, sql_literal(term)));
                    }
                    Value::Variable(v) => {
                        if let Some(term) = existing.get(v) {
                            where_parts
                                .push(format!("\"{}\" = {}", fd.name, sql_literal(term)));
                        }
                    }
                }

                let sql = if where_parts.is_empty() {
                    format!(
                        "SELECT \"_subject\", \"{}\" FROM \"{}\"",
                        fd.name, table
                    )
                } else {
                    format!(
                        "SELECT \"_subject\", \"{}\" FROM \"{}\" WHERE {}",
                        fd.name, table, where_parts.join(" AND ")
                    )
                };

                let result = self.executor.execute_sql(&sql)?;

                for row in &result.rows {
                    let subj_str = match &row[0] {
                        Some(s) => s,
                        None => continue,
                    };
                    let obj_str = match &row[1] {
                        Some(s) => s,
                        None => continue,
                    };

                    let subj_term = Term::Iri(Iri::new(subj_str.as_str()));
                    let obj_term = parse_sql_value(obj_str, &fd.field_type);

                    let mut binding = existing.clone();
                    let mut ok = true;

                    // Bind subject
                    if let Subject::Variable(v) = subject {
                        if !try_bind(v, &subj_term, &mut binding) {
                            ok = false;
                        }
                    }

                    // Bind predicate
                    if ok && !try_bind(predicate_var, &pred_term, &mut binding) {
                        ok = false;
                    }

                    // Bind object
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
                        all_bindings.push(binding);
                    }
                }
            }
        }

        Ok(all_bindings)
    }

    /// Execute a FieldScan query for the synthetic rdf:type field.
    fn exec_field_scan_query(
        &self,
        table: &str,
        subject_filter: &Option<String>,
        subject: &Subject,
        predicate_var: &str,
        pred_term: &Term,
        object: &Value,
        obj_term: &Term,
        _field_type: Option<&FieldType>,
        existing: &Binding,
        out: &mut Vec<Binding>,
    ) -> Result<(), DarqError> {
        // Check if object is bound and conflicts with the synthetic value
        match object {
            Value::Bound(expected) => {
                if expected != obj_term {
                    return Ok(());
                }
            }
            Value::Variable(v) => {
                if let Some(existing_val) = existing.get(v) {
                    if *existing_val != *obj_term {
                        return Ok(());
                    }
                }
            }
        }

        let sql = if let Some(f) = subject_filter {
            format!("SELECT \"_subject\" FROM \"{}\" WHERE {}", table, f)
        } else {
            format!("SELECT \"_subject\" FROM \"{}\"", table)
        };

        let result = self.executor.execute_sql(&sql)?;

        for row in &result.rows {
            let subj_str = match &row[0] {
                Some(s) => s,
                None => continue,
            };

            let subj_term = Term::Iri(Iri::new(subj_str.as_str()));

            let mut binding = existing.clone();
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
                if let Value::Variable(v) = object {
                    if !try_bind(v, obj_term, &mut binding) {
                        ok = false;
                    }
                }
            }

            if ok {
                out.push(binding);
            }
        }

        Ok(())
    }
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

/// Build a WHERE clause for the subject, if it can be constrained.
fn subject_where_clause(subject: &Subject, existing: &Binding) -> Option<String> {
    match subject {
        Subject::Variable(v) => {
            if let Some(term) = existing.get(v) {
                Some(format!("\"_subject\" = {}", sql_literal(term)))
            } else {
                None
            }
        }
        Subject::Bound(iri) => Some(format!(
            "\"_subject\" = '{}'",
            iri.0.replace('\'', "''")
        )),
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
        /// Canned responses keyed by SQL prefix match.
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
                    },
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/age"),
                        name: "age",
                        field_type: FieldType::Integer,
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

        // Second pattern: FieldScan with subject constraint from first result
        // These will have WHERE "_subject" = ... from pipelining
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
        // The second+ queries (FieldScan) should contain the alice IRI as a filter
        for q in &queries[1..] {
            assert!(
                q.contains("alice"),
                "FieldScan query should contain subject constraint from pipelining: {}",
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
