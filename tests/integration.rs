use darq::engine;
use darq::error::DarqError;
use darq::rdf::{Iri, Literal, Term};
use darq::resource_store::ResourceStore;
use darq::schema::{FieldDescriptor, FieldType, Resource, Schema};

// ---------------------------------------------------------------------------
// Test data model
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

fn setup() -> (Schema, ResourceStore) {
    let mut schema = Schema::new();
    schema.register::<Person>();

    let mut store = ResourceStore::new();
    store.load(&Person { id: "alice".into(), name: "Alice".into(), age: 30 });
    store.load(&Person { id: "bob".into(), name: "Bob".into(), age: 25 });
    store.load(&Person { id: "carol".into(), name: "Carol".into(), age: 35 });

    (schema, store)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_select_star_with_type() {
    let (schema, store) = setup();
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT * WHERE {
            ?person a ex:Person .
            ?person ex:name ?name .
        }
        "#,
        &schema,
        &store,
    )
    .unwrap();

    assert_eq!(result.rows.len(), 3);
    assert!(result.variables.contains(&"person".to_string()));
    assert!(result.variables.contains(&"name".to_string()));
}

#[test]
fn test_select_with_all_fields() {
    let (schema, store) = setup();
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name ?age
        WHERE {
            ?p a ex:Person .
            ?p ex:name ?name .
            ?p ex:age ?age .
        }
        ORDER BY ?name
        "#,
        &schema,
        &store,
    )
    .unwrap();

    assert_eq!(result.variables, vec!["name", "age"]);
    assert_eq!(result.rows.len(), 3);

    // Alice, 30
    assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Alice".into()))));
    assert_eq!(result.rows[0][1], Some(Term::Literal(Literal::Integer(30))));
    // Bob, 25
    assert_eq!(result.rows[1][0], Some(Term::Literal(Literal::String("Bob".into()))));
    assert_eq!(result.rows[1][1], Some(Term::Literal(Literal::Integer(25))));
    // Carol, 35
    assert_eq!(result.rows[2][0], Some(Term::Literal(Literal::String("Carol".into()))));
    assert_eq!(result.rows[2][1], Some(Term::Literal(Literal::Integer(35))));
}

#[test]
fn test_limit_and_offset() {
    let (schema, store) = setup();
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name
        WHERE { ?p ex:name ?name }
        ORDER BY ?name
        LIMIT 2
        OFFSET 1
        "#,
        &schema,
        &store,
    )
    .unwrap();

    // Ordered: Alice, Bob, Carol — skip 1, take 2 = Bob, Carol
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Bob".into()))));
    assert_eq!(result.rows[1][0], Some(Term::Literal(Literal::String("Carol".into()))));
}

#[test]
fn test_unknown_predicate_errors() {
    let (schema, store) = setup();
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?email
        WHERE { ?p ex:email ?email }
        "#,
        &schema,
        &store,
    );

    match result {
        Err(DarqError::UnknownPredicate(iri)) => {
            assert_eq!(iri.0, "http://example.org/email");
        }
        other => panic!("expected UnknownPredicate error, got {:?}", other),
    }
}

#[test]
fn test_unknown_prefix_errors() {
    let (schema, store) = setup();
    let result = engine::execute(
        "SELECT ?x WHERE { ?s foaf:name ?x }",
        &schema,
        &store,
    );

    assert!(matches!(result, Err(DarqError::UnknownPrefix(_))));
}

#[test]
fn test_semicolon_shorthand_integration() {
    let (schema, store) = setup();
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name ?age
        WHERE {
            ?p a ex:Person ;
               ex:name ?name ;
               ex:age ?age .
        }
        ORDER BY DESC(?age)
        LIMIT 1
        "#,
        &schema,
        &store,
    )
    .unwrap();

    // Oldest person: Carol, 35
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Carol".into()))));
    assert_eq!(result.rows[0][1], Some(Term::Literal(Literal::Integer(35))));
}

#[test]
fn test_full_iri_without_prefix() {
    let (schema, store) = setup();
    let result = engine::execute(
        r#"SELECT ?name WHERE { ?p <http://example.org/name> ?name } ORDER BY ?name LIMIT 1"#,
        &schema,
        &store,
    )
    .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Alice".into()))));
}

#[test]
fn test_empty_result() {
    let (schema, store) = setup();
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name
        WHERE {
            ?p ex:name "NonexistentPerson" .
            ?p ex:name ?name .
        }
        "#,
        &schema,
        &store,
    )
    .unwrap();

    assert_eq!(result.rows.len(), 0);
}

#[test]
fn test_variable_predicate_expansion() {
    let (schema, store) = setup();
    // ?s ?p ?o should expand over all fields and return all data
    let result = engine::execute(
        "SELECT ?s ?p ?o WHERE { ?s ?p ?o }",
        &schema,
        &store,
    )
    .unwrap();

    // 3 people x 3 fields (rdf:type, name, age) = 9 rows
    assert_eq!(result.rows.len(), 9);
    // ?p should be bound to actual predicate IRIs
    assert!(result.variables.contains(&"p".to_string()));
    let pred_col = result.variables.iter().position(|v| v == "p").unwrap();
    for row in &result.rows {
        match &row[pred_col] {
            Some(Term::Iri(_)) => {} // good — predicate is always an IRI
            other => panic!("expected predicate to be an IRI, got {:?}", other),
        }
    }
}

#[test]
fn test_variable_predicate_constrained_by_prior_pattern() {
    let (schema, store) = setup();
    // First pattern binds ?person via ex:name, second pattern scans fields
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name ?p ?o
        WHERE {
            ?person ex:name ?name .
            ?person ?p ?o .
        }
        ORDER BY ?name ?p
        "#,
        &schema,
        &store,
    )
    .unwrap();

    // Each of 3 people has 3 fields scanned by ?person ?p ?o = 9 rows
    assert_eq!(result.rows.len(), 9);
}
