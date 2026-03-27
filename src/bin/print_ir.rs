use std::process;

use darq::lower;
use darq::rdf::{Iri, Term};
use darq::schema::{FieldDescriptor, FieldType, Resource, Schema};
use darq::sparql;

struct Person {
    _id: String,
    _name: String,
    _age: i64,
}

impl Resource for Person {
    fn rdf_type() -> Iri {
        Iri::new("http://example.org/Person")
    }

    fn subject_iri(&self) -> Iri {
        Iri::new(format!("http://example.org/person/{}", self._id))
    }

    fn field_descriptors() -> Vec<FieldDescriptor> {
        vec![
            FieldDescriptor {
                predicate: Iri::new("http://example.org/name"),
                name: "name",
                field_type: FieldType::Literal,
            },
            FieldDescriptor {
                predicate: Iri::new("http://example.org/age"),
                name: "age",
                field_type: FieldType::Literal,
            },
        ]
    }

    fn field_values(&self) -> Vec<Term> {
        vec![self._name.clone().into(), self._age.into()]
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: print_ir <query-file>");
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

    let mut schema = Schema::new();
    schema.register::<Person>();

    match lower::lower(&query, &schema) {
        Ok(plan) => println!("{:#?}", plan),
        Err(e) => {
            eprintln!("Lowering error: {}", e);
            process::exit(1);
        }
    }
}
