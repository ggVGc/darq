use std::process;

use darq::lower;
use darq::sparql;
use darq::sql;
use darq::test_schema;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: print_sql <query-file>");
        process::exit(1);
    }

    let input = match std::fs::read_to_string(&args[1]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading {}: {}", args[1], e);
            process::exit(1);
        }
    };

    let mut query = match sparql::parser::parse(&input) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            process::exit(1);
        }
    };

    if let Err(e) = sparql::expand_prefixes(&mut query) {
        eprintln!("Prefix expansion error: {}", e);
        process::exit(1);
    }

    if let Err(e) = sparql::validate_select_variables(&query) {
        eprintln!("Validation error: {}", e);
        process::exit(1);
    }

    let schema = test_schema::test_schema();

    let plan = match lower::lower(&query, &schema) {
        Ok(plan) => plan,
        Err(e) => {
            eprintln!("Lowering error: {}", e);
            process::exit(1);
        }
    };

    match sql::to_sql(&plan, &schema, "_subject", "_subject") {
        Ok(sql) => println!("{}", sql),
        Err(e) => {
            eprintln!("SQL translation error: {}", e);
            process::exit(1);
        }
    }
}
