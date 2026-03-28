use std::env;

use postgres::Client;
use rand::seq::SliceRandom;
use rand::Rng;

use darq::define_resource;
use darq::engine;
use darq::engine::postgres::PostgresExecutor;
use darq::engine::sql::SqlEngine;
use darq::schema::{FieldType, Schema};

// ---------------------------------------------------------------------------
// Data model (same as people.rs / people_sql.rs)
// ---------------------------------------------------------------------------

define_resource!(
    Person, "http://example.org/Person", table = "people", [
        ("name", "http://example.org/name", FieldType::String),
        ("age", "http://example.org/age", FieldType::Integer),
    ]
);

// ---------------------------------------------------------------------------
// Example queries
// ---------------------------------------------------------------------------

const FIRST_NAMES: &[&str] = &[
    "Alice", "Bob", "Carol", "Dave", "Eve", "Frank", "Grace", "Heidi", "Ivan", "Judy",
    "Karl", "Lena", "Mallory", "Nora", "Oscar", "Peggy", "Quinn", "Rupert", "Sybil", "Trent",
];

fn main() {
    let args: Vec<String> = env::args().collect();
    let seed_pos = args.iter().position(|a| a == "--seed");
    let seed_count: Option<usize> = seed_pos.map(|i| {
        args.get(i + 1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(10)
    });

    let conn_str = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "host=localhost user=postgres dbname=rdfsql".to_string());

    let mut client = Client::connect(&conn_str, postgres::NoTls)
        .expect("failed to connect to PostgreSQL");

    if let Some(count) = seed_count {
        const BATCH_SIZE: usize = 100_000;

        client
            .batch_execute(
                r#"
                DROP TABLE IF EXISTS "people";
                CREATE TABLE "people" (
                    "_subject" TEXT NOT NULL,
                    "name"     TEXT NOT NULL,
                    "age"      INT NOT NULL
                );
                "#,
            )
            .expect("failed to create table");

        let mut rng = rand::thread_rng();
        let mut inserted = 0;
        while inserted < count {
            let batch = (count - inserted).min(BATCH_SIZE);
            let values: Vec<String> = (inserted..inserted + batch)
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

            let sql = format!(
                r#"INSERT INTO "people" ("_subject", "name", "age") VALUES {};"#,
                values.join(",")
            );
            client.batch_execute(&sql).expect("failed to insert batch");

            inserted += batch;
            println!("Inserted {}/{} people", inserted, count);
        }

        println!("Creating indices");

        client
            .batch_execute(r#"CREATE INDEX ON "people"(name);"#)
            .expect("failed to create index");

        client
            .batch_execute(r#"CREATE INDEX ON "people"(age);"#)
            .expect("failed to create index");
        println!("Done");

        return;
    }

    let executor = PostgresExecutor::with_sql_callback(client, |sql| println!("  SQL: {sql}"));

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
        LIMIT 10
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
