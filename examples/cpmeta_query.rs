use std::env;
use std::fs;
use std::process;

use postgres::Client;

use darq::cpmeta_schema::cpmeta_schema;
use darq::engine;
use darq::engine::postgres::PostgresExecutor;
use darq::engine::sql::SqlEngine;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: cpmeta_query <query.rq>");
        process::exit(1);
    }

    let query = fs::read_to_string(&args[1]).unwrap_or_else(|e| {
        eprintln!("Failed to read {}: {e}", args[1]);
        process::exit(1);
    });

    let conn_str = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "host=localhost user=postgres dbname=rdfsql".to_string());

    let client = Client::connect(&conn_str, postgres::NoTls)
        .expect("failed to connect to PostgreSQL");

    let executor = PostgresExecutor::with_sql_callback(client, |sql| eprintln!("  SQL: {sql}"));
    let schema = cpmeta_schema();

    run_query(&query, &schema, &executor);
}

fn run_query(query: &str, schema: &darq::schema::Schema, executor: &PostgresExecutor) {
    let eng = SqlEngine::new(executor).with_subject_column("rdf_subject");
    match engine::execute(query, schema, &eng) {
        Ok(result) => {
            let header: Vec<_> = result.variables.iter().map(|v| format!("?{}", v)).collect();
            println!("{}", header.join("\t"));
            println!("{}", "-".repeat(header.join("\t").len()));

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
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}
