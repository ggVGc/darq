use darq::engine::{self, InMemoryEngine};
use darq::rdf::{Iri, Term};
use darq::engine::memory::ResourceStore;
use darq::schema::{FieldDescriptor, FieldType, Resource, Schema};

// ---------------------------------------------------------------------------
// Data model: each struct maps to an RDF type, fields map to predicates.
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
// Example queries
// ---------------------------------------------------------------------------

fn main() {
    // Set up schema and store
    let mut schema = Schema::new();
    schema.register::<Person>();

    let mut store = ResourceStore::new();
    store.load(&Person { id: "alice".into(), name: "Alice".into(), age: 30 });
    store.load(&Person { id: "bob".into(), name: "Bob".into(), age: 25 });
    store.load(&Person { id: "carol".into(), name: "Carol".into(), age: 35 });

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
    run_query(query, &schema, &store);

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
    run_query(query, &schema, &store);

    // Query 3: unknown predicate should error
    let query = r#"
        PREFIX ex: <http://example.org/>
        SELECT ?email WHERE { ?p ex:email ?email }
    "#;

    println!("\n=== Query with unknown predicate ===");
    run_query(query, &schema, &store);
}

fn run_query(query: &str, schema: &Schema, store: &ResourceStore) {
    let eng = InMemoryEngine::new(store);
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
