use std::process;

use darq::cpmeta_schema;
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

    let query = match sparql::parser::parse(&input) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            process::exit(1);
        }
    };

    if let Err(e) = sparql::validate_select_variables(&query) {
        eprintln!("Validation error: {}", e);
        process::exit(1);
    }

    let schema = if input.contains("meta.icos-cp.eu") {
        cpmeta_schema::cpmeta_schema()
    } else {
        test_schema::test_schema()
    };

    let plans = match lower::lower(&query, &schema) {
        Ok(plans) => plans,
        Err(e) => {
            eprintln!("Lowering error: {}", e);
            process::exit(1);
        }
    };

    if plans.len() == 1 {
        match sql::to_sql(&plans[0], &schema, "_subject", "_subject") {
            Ok(sql) => println!("{}", sql),
            Err(e) => {
                eprintln!("SQL translation error: {}", e);
                process::exit(1);
            }
        }
    } else {
        match sql::to_union_sql(&plans, &schema, "_subject", "_subject") {
            Ok(sql) => println!("{}", sql),
            Err(e) => {
                eprintln!("SQL translation error: {}", e);
                process::exit(1);
            }
        }
    }
}
