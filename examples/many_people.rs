use std::time::Instant;

use darq::engine::{self, InMemoryEngine};
use darq::rdf::{Iri, Term};
use darq::resource_store::ResourceStore;
use darq::schema::{FieldDescriptor, FieldType, Resource, Schema};

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
            },
            FieldDescriptor {
                predicate: Iri::new("http://example.org/age"),
                name: "age",
                field_type: FieldType::Integer,
            },
        ]
    }

    fn field_values(&self) -> Vec<Term> {
        vec![self.name.clone().into(), self.age.into()]
    }
}

fn main() {
    let count = 1_000_000;

    // Load data
    let mut schema = Schema::new();
    schema.register::<Person>();

    println!("Loading {} people...", count);
    let t = Instant::now();
    let mut store = ResourceStore::new();
    for i in 0..count {
        store.load(&Person {
            id: format!("{}", i),
            name: format!("Person_{}", i),
            age: (i % 120) as i64,
        });
    }
    println!("Loaded in {:.2?}", t.elapsed());

    // Query: 10 youngest people by age
    let query = r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name ?age
        WHERE {
            ?p a ex:Person ;
               ex:name ?name ;
               ex:age ?age .
        }
        ORDER BY ?age
        LIMIT 10
    "#;

    println!("\nRunning query (ORDER BY ?age ?name, LIMIT 10)...");
    let t = Instant::now();
    let eng = InMemoryEngine::new(&store);
    let result = engine::execute(query, &schema, &eng).unwrap();
    println!("Query completed in {:.2?}", t.elapsed());

    println!("\n?name\t\t?age");
    println!("{}", "-".repeat(24));
    for row in &result.rows {
        let name = match &row[0] {
            Some(term) => format!("{}", term),
            None => "UNBOUND".to_string(),
        };
        let age = match &row[1] {
            Some(term) => format!("{}", term),
            None => "UNBOUND".to_string(),
        };
        println!("{}\t{}", name, age);
    }
    println!("\n({} total results projected)", result.rows.len());
}
