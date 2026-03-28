use std::cell::RefCell;
use std::env;

use postgres::types::Type;
use postgres::Client;
use rand::seq::SliceRandom;
use rand::Rng;

use darq::engine;
use darq::engine::sql::{SqlEngine, SqlExecutor, SqlResultSet};
use darq::rdf::{Iri, Term};
use darq::schema::{FieldDescriptor, FieldType, Resource, Schema};

// ---------------------------------------------------------------------------
// Data model (same as people.rs / people_sql.rs)
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
// SqlExecutor implementation for PostgreSQL
// ---------------------------------------------------------------------------

struct PostgresExecutor {
    client: RefCell<Client>,
}

impl PostgresExecutor {
    fn new(client: Client) -> Self {
        Self {
            client: RefCell::new(client),
        }
    }
}

fn column_to_string(row: &postgres::Row, idx: usize, col_type: &Type) -> Option<String> {
    if *col_type == Type::INT8 {
        row.get::<_, Option<i64>>(idx).map(|v| v.to_string())
    } else if *col_type == Type::INT4 {
        row.get::<_, Option<i32>>(idx).map(|v| v.to_string())
    } else if *col_type == Type::FLOAT4 {
        row.get::<_, Option<f32>>(idx).map(|v| v.to_string())
    } else if *col_type == Type::FLOAT8 {
        row.get::<_, Option<f64>>(idx).map(|v| v.to_string())
    } else if *col_type == Type::BOOL {
        row.get::<_, Option<bool>>(idx)
            .map(|v| if v { "true" } else { "false" }.to_string())
    } else {
        // TEXT, VARCHAR, and anything else — try as String
        row.get::<_, Option<String>>(idx)
    }
}

impl SqlExecutor for PostgresExecutor {
    fn execute_sql(&self, sql: &str) -> Result<SqlResultSet, darq::error::DarqError> {
        println!("  SQL: {}", sql);

        let mut client = self.client.borrow_mut();
        let rows = client
            .query(sql, &[])
            .map_err(|e| darq::error::DarqError::SqlError(e.to_string()))?;

        if rows.is_empty() {
            // Still need column names from the statement — run with LIMIT 0
            // to get metadata. But for simplicity, if there are no rows we
            // can derive columns from the first query's column descriptions.
            // Actually postgres crate doesn't give columns without rows easily,
            // so just return empty with column names from the SQL.
            // Fortunately, the engine handles empty result sets fine.
            return Ok(SqlResultSet {
                columns: Vec::new(),
                rows: Vec::new(),
            });
        }

        let columns: Vec<String> = rows[0]
            .columns()
            .iter()
            .map(|c| c.name().to_string())
            .collect();

        let col_types: Vec<Type> = rows[0]
            .columns()
            .iter()
            .map(|c| c.type_().clone())
            .collect();

        let result_rows: Vec<Vec<Option<String>>> = rows
            .iter()
            .map(|row| {
                (0..columns.len())
                    .map(|i| column_to_string(row, i, &col_types[i]))
                    .collect()
            })
            .collect();

        Ok(SqlResultSet {
            columns,
            rows: result_rows,
        })
    }
}

// ---------------------------------------------------------------------------
// Example queries
// ---------------------------------------------------------------------------

const FIRST_NAMES: &[&str] = &[
    "Alice", "Bob", "Carol", "Dave", "Eve", "Frank", "Grace", "Heidi", "Ivan", "Judy",
    "Karl", "Lena", "Mallory", "Nora", "Oscar", "Peggy", "Quinn", "Rupert", "Sybil", "Trent",
];

fn main() {
    let args: Vec<String> = env::args().collect();
    let count: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let conn_str = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "host=localhost user=postgres dbname=rdfsql".to_string());

    let mut client = Client::connect(&conn_str, postgres::NoTls)
        .expect("failed to connect to PostgreSQL");

    // Set up the table and insert random people
    let mut rng = rand::thread_rng();
    let values: Vec<String> = (0..count)
        .map(|i| {
            let name = FIRST_NAMES.choose(&mut rng).unwrap();
            let age: i64 = rng.gen_range(18..=80);
            let id = format!("{}_{}", name.to_lowercase(), i);
            format!(
                "('http://example.org/person/{}', '{}', {})",
                id, name, age
            )
        })
        .collect();

    let setup_sql = format!(
        r#"
        DROP TABLE IF EXISTS "Person";
        CREATE TABLE "Person" (
            "_subject" TEXT NOT NULL,
            "name"     TEXT NOT NULL,
            "age"      BIGINT NOT NULL
        );
        INSERT INTO "Person" ("_subject", "name", "age") VALUES
            {};
        "#,
        values.join(",\n            ")
    );

    client
        .batch_execute(&setup_sql)
        .expect("failed to set up table");

    println!("Inserted {} people with random names and ages.\n", count);

    let executor = PostgresExecutor::new(client);

    let mut schema = Schema::new();
    schema.register::<Person>();

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
    run_query(query, &schema, &executor);

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
    run_query(query, &schema, &executor);

    // Clean up
    executor
        .client
        .borrow_mut()
        .batch_execute(r#"DROP TABLE IF EXISTS "Person""#)
        .expect("failed to clean up");
}

fn run_query(query: &str, schema: &Schema, executor: &PostgresExecutor) {
    let eng = SqlEngine::new(executor);
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
