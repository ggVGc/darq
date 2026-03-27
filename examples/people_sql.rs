use std::collections::HashMap;

use darq::engine::sql::{SqlEngine, SqlExecutor, SqlResultSet};
use darq::engine;
use darq::rdf::{Iri, Term};
use darq::schema::{FieldDescriptor, FieldType, Resource, Schema};

// ---------------------------------------------------------------------------
// Data model (same as people.rs for easy comparison)
// ---------------------------------------------------------------------------

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
        vec![self.name.clone().into(), self.age.into()]
    }
}

// ---------------------------------------------------------------------------
// Minimal in-memory SQL database
// ---------------------------------------------------------------------------

struct Table {
    columns: Vec<String>,
    rows: Vec<Vec<Option<String>>>,
}

struct InMemoryDatabase {
    tables: HashMap<String, Table>,
}

impl InMemoryDatabase {
    fn new() -> Self {
        Self { tables: HashMap::new() }
    }

    /// Create a table from Resource instances.  The table name is the local
    /// part of the type IRI (after the last `/` or `#`).
    fn load<R: Resource>(&mut self, resources: &[R]) {
        let type_iri = R::rdf_type();
        let table_name = local_name(&type_iri.0);
        let descriptors = R::field_descriptors();

        let mut columns = vec!["_subject".to_string()];
        columns.extend(descriptors.iter().map(|d| d.name.to_string()));

        let rows = resources
            .iter()
            .map(|r| {
                let mut row: Vec<Option<String>> = vec![Some(r.subject_iri().0.clone())];
                for val in r.field_values() {
                    row.push(Some(term_to_sql_string(&val)));
                }
                row
            })
            .collect();

        self.tables.insert(table_name.to_string(), Table { columns, rows });
    }
}

/// Extract the local name from an IRI (after last `/` or `#`).
fn local_name(iri: &str) -> &str {
    if let Some(pos) = iri.rfind('#') {
        &iri[pos + 1..]
    } else if let Some(pos) = iri.rfind('/') {
        &iri[pos + 1..]
    } else {
        iri
    }
}

/// Convert a Term to the plain string form that SqlEngine expects back from
/// SqlResultSet (no angle brackets for IRIs, no quotes around strings).
fn term_to_sql_string(term: &Term) -> String {
    match term {
        Term::Iri(iri) => iri.0.clone(),
        Term::Literal(lit) => match lit {
            darq::rdf::Literal::String(s) => s.clone(),
            darq::rdf::Literal::Integer(n) => n.to_string(),
            darq::rdf::Literal::Boolean(b) => if *b { "true" } else { "false" }.to_string(),
            darq::rdf::Literal::Float(v) | darq::rdf::Literal::Double(v) => format!("{}", v.0),
            darq::rdf::Literal::Decimal(s)
            | darq::rdf::Literal::Date(s)
            | darq::rdf::Literal::DateTime(s) => s.clone(),
        },
    }
}

impl SqlExecutor for InMemoryDatabase {
    fn execute_sql(&self, sql: &str) -> Result<SqlResultSet, darq::error::DarqError> {
        println!("  SQL: {}", sql);

        let parsed = parse_sql(sql)
            .map_err(|e| darq::error::DarqError::SqlError(format!("{}: {}", e, sql)))?;

        let table = self.tables.get(&parsed.table).ok_or_else(|| {
            darq::error::DarqError::SqlError(format!("no such table: {}", parsed.table))
        })?;

        // Map SELECT column names to indices in the table
        let col_indices: Vec<usize> = parsed
            .columns
            .iter()
            .map(|name| {
                table.columns.iter().position(|c| c == name).ok_or_else(|| {
                    darq::error::DarqError::SqlError(format!(
                        "no such column: {} in table {}",
                        name, parsed.table
                    ))
                })
            })
            .collect::<Result<_, _>>()?;

        // Filter rows by WHERE conditions
        let rows: Vec<Vec<Option<String>>> = table
            .rows
            .iter()
            .filter(|row| {
                parsed.conditions.iter().all(|cond| {
                    let idx = match table.columns.iter().position(|c| c == &cond.column) {
                        Some(i) => i,
                        None => return false,
                    };
                    let cell = match &row[idx] {
                        Some(v) => v.as_str(),
                        None => return false,
                    };
                    match &cond.op {
                        CondOp::Eq(val) => cell == val,
                        CondOp::In(vals) => vals.iter().any(|v| v == cell),
                    }
                })
            })
            .map(|row| col_indices.iter().map(|&i| row[i].clone()).collect())
            .collect();

        Ok(SqlResultSet {
            columns: parsed.columns,
            rows,
        })
    }
}

// ---------------------------------------------------------------------------
// Minimal SQL parser for the subset SqlEngine generates
// ---------------------------------------------------------------------------

struct ParsedQuery {
    columns: Vec<String>,
    table: String,
    conditions: Vec<Condition>,
}

struct Condition {
    column: String,
    op: CondOp,
}

enum CondOp {
    Eq(String),
    In(Vec<String>),
}

fn parse_sql(sql: &str) -> Result<ParsedQuery, String> {
    // Split into SELECT ... FROM ... [WHERE ...]
    let after_select = sql
        .strip_prefix("SELECT ")
        .ok_or("expected SELECT")?;

    let from_pos = after_select.find(" FROM ").ok_or("expected FROM")?;
    let select_part = &after_select[..from_pos];
    let after_from = &after_select[from_pos + 6..];

    // Table name is double-quoted
    let (table, rest) = parse_quoted_identifier(after_from)?;

    // Parse optional WHERE
    let conditions = if let Some(where_part) = rest.strip_prefix(" WHERE ") {
        parse_where(where_part)?
    } else {
        Vec::new()
    };

    // Parse SELECT columns
    let columns = select_part
        .split(", ")
        .map(|col| strip_quotes(col.trim()))
        .collect();

    Ok(ParsedQuery { columns, table, conditions })
}

/// Parse a double-quoted identifier, returning (name, rest_of_string).
fn parse_quoted_identifier(s: &str) -> Result<(String, &str), String> {
    let s = s.strip_prefix('"').ok_or("expected '\"'")?;
    let end = s.find('"').ok_or("unterminated identifier")?;
    Ok((s[..end].to_string(), &s[end + 1..]))
}

/// Strip surrounding double quotes from an identifier.
fn strip_quotes(s: &str) -> String {
    s.trim_matches('"').to_string()
}

fn parse_where(s: &str) -> Result<Vec<Condition>, String> {
    s.split(" AND ")
        .map(|part| {
            if let Some(in_pos) = part.find(" IN (") {
                let col = strip_quotes(part[..in_pos].trim());
                let vals_str = &part[in_pos + 5..];
                let vals_str = vals_str
                    .strip_suffix(')')
                    .ok_or("unterminated IN clause")?;
                let vals = parse_value_list(vals_str);
                Ok(Condition { column: col, op: CondOp::In(vals) })
            } else if let Some(eq_pos) = part.find(" = ") {
                let col = strip_quotes(part[..eq_pos].trim());
                let val = parse_sql_value(part[eq_pos + 3..].trim());
                Ok(Condition { column: col, op: CondOp::Eq(val) })
            } else {
                Err(format!("cannot parse condition: {}", part))
            }
        })
        .collect()
}

/// Parse a comma-separated list of SQL values (quoted strings or bare numbers).
fn parse_value_list(s: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        if rest.starts_with('\'') {
            // Quoted string — handle '' escapes
            let (val, after) = parse_quoted_string(&rest[1..]);
            values.push(val);
            rest = after.trim_start_matches(',').trim();
        } else {
            // Bare value (number, TRUE/FALSE)
            let end = rest.find(',').unwrap_or(rest.len());
            values.push(rest[..end].trim().to_string());
            rest = if end < rest.len() { rest[end + 1..].trim() } else { "" };
        }
    }
    values
}

/// Parse a single SQL value (strip quotes from strings, leave numbers as-is).
fn parse_sql_value(s: &str) -> String {
    let s = s.trim();
    if s.starts_with('\'') && s.ends_with('\'') {
        s[1..s.len() - 1].replace("''", "'")
    } else {
        s.to_string()
    }
}

/// Parse a single-quoted string starting after the opening quote.
/// Returns (unescaped_value, rest_of_input_after_closing_quote).
fn parse_quoted_string(s: &str) -> (String, &str) {
    let mut result = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        if c == '\'' {
            // Check for escaped quote ('')
            if s[i + 1..].starts_with('\'') {
                result.push('\'');
                chars.next(); // skip the second quote
            } else {
                return (result, &s[i + 1..]);
            }
        } else {
            result.push(c);
        }
    }
    (result, "")
}

// ---------------------------------------------------------------------------
// Example queries
// ---------------------------------------------------------------------------

fn main() {
    let mut schema = Schema::new();
    schema.register::<Person>();

    let mut db = InMemoryDatabase::new();
    db.load(&[
        Person { id: "alice".into(), name: "Alice".into(), age: 30 },
        Person { id: "bob".into(), name: "Bob".into(), age: 25 },
        Person { id: "carol".into(), name: "Carol".into(), age: 35 },
    ]);

    // Query 1: all people with names and ages, ordered by name
    let query = r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name ?age
        WHERE {
            ?person a ex:Person ;
                    ex:name ?name ;
                    ex:age ?age .
        }
        ORDER BY ?name
    "#;

    println!("=== All people (ordered by name) ===");
    run_query(query, &schema, &db);

    // Query 2: oldest person
    let query = r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name ?age
        WHERE {
            ?p ex:name ?name .
            ?p ex:age ?age .
        }
        ORDER BY DESC(?age)
        LIMIT 1
    "#;

    println!("\n=== Oldest person ===");
    run_query(query, &schema, &db);

    // Query 3: unknown predicate should error
    let query = r#"
        PREFIX ex: <http://example.org/>
        SELECT ?email WHERE { ?p ex:email ?email }
    "#;

    println!("\n=== Query with unknown predicate ===");
    run_query(query, &schema, &db);
}

fn run_query(query: &str, schema: &Schema, db: &InMemoryDatabase) {
    let eng = SqlEngine::new(db);
    match engine::execute(query, schema, &eng) {
        Ok(result) => {
            // Header
            let header: Vec<_> = result.variables.iter().map(|v| format!("?{}", v)).collect();
            println!("{}", header.join("\t"));
            println!("{}", "-".repeat(header.join("\t").len()));

            // Rows
            for row in &result.rows {
                let cells: Vec<_> = row
                    .iter()
                    .map(|cell| match cell {
                        Some(term) => format!("{}", term),
                        None => "UNBOUND".to_string(),
                    })
                    .collect();
                println!("{}", cells.join("\t"));
            }
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }
}
