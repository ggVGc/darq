use darq::engine::{self, InMemoryEngine};
use darq::error::DarqError;
use darq::rdf::{Iri, Literal, Term};
use darq::engine::memory::ResourceStore;
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
    let eng = InMemoryEngine::new(&store);
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT * WHERE {
            ?person a ex:Person .
            ?person ex:name ?name .
        }
        "#,
        &schema,
        &eng,
    )
    .unwrap();

    assert_eq!(result.rows.len(), 3);
    assert!(result.variables.contains(&"person".to_string()));
    assert!(result.variables.contains(&"name".to_string()));
}

#[test]
fn test_select_with_all_fields() {
    let (schema, store) = setup();
    let eng = InMemoryEngine::new(&store);
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
        &eng,
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
    let eng = InMemoryEngine::new(&store);
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
        &eng,
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
    let eng = InMemoryEngine::new(&store);
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?email
        WHERE { ?p ex:email ?email }
        "#,
        &schema,
        &eng,
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
    let eng = InMemoryEngine::new(&store);
    let result = engine::execute(
        "SELECT ?x WHERE { ?s foaf:name ?x }",
        &schema,
        &eng,
    );

    assert!(matches!(result, Err(DarqError::ParseError(_))));
}

#[test]
fn test_semicolon_shorthand_integration() {
    let (schema, store) = setup();
    let eng = InMemoryEngine::new(&store);
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
        &eng,
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
    let eng = InMemoryEngine::new(&store);
    let result = engine::execute(
        r#"SELECT ?name WHERE { ?p <http://example.org/name> ?name } ORDER BY ?name LIMIT 1"#,
        &schema,
        &eng,
    )
    .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Alice".into()))));
}

#[test]
fn test_empty_result() {
    let (schema, store) = setup();
    let eng = InMemoryEngine::new(&store);
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
        &eng,
    )
    .unwrap();

    assert_eq!(result.rows.len(), 0);
}

#[test]
fn test_variable_predicate_expansion() {
    let (schema, store) = setup();
    let eng = InMemoryEngine::new(&store);
    // ?s ?p ?o should expand over all fields and return all data
    let result = engine::execute(
        "SELECT ?s ?p ?o WHERE { ?s ?p ?o }",
        &schema,
        &eng,
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
    let eng = InMemoryEngine::new(&store);
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
        &eng,
    )
    .unwrap();

    // Each of 3 people has 3 fields scanned by ?person ?p ?o = 9 rows
    assert_eq!(result.rows.len(), 9);
}

#[test]
fn test_distinct_deduplicates_projected_variables() {
    let mut schema = Schema::new();
    schema.register::<Person>();

    let mut store = ResourceStore::new();
    // Two people share the same name but have different ages
    store.load(&Person { id: "alice1".into(), name: "Alice".into(), age: 30 });
    store.load(&Person { id: "alice2".into(), name: "Alice".into(), age: 25 });
    store.load(&Person { id: "bob".into(), name: "Bob".into(), age: 40 });

    let eng = InMemoryEngine::new(&store);
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT DISTINCT ?name
        WHERE { ?p ex:name ?name }
        ORDER BY ?name
        "#,
        &schema,
        &eng,
    )
    .unwrap();

    // Without DISTINCT we'd get 3 rows (Alice, Alice, Bob).
    // DISTINCT should collapse the duplicate Alice.
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Alice".into()))));
    assert_eq!(result.rows[1][0], Some(Term::Literal(Literal::String("Bob".into()))));
}

// ---------------------------------------------------------------------------
// VALUES clause
// ---------------------------------------------------------------------------

#[test]
fn test_values_filters_results() {
    let (schema, store) = setup();
    let eng = InMemoryEngine::new(&store);
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name ?age
        WHERE { ?p ex:name ?name . ?p ex:age ?age }
        ORDER BY ?name
        VALUES ?name { "Alice" "Carol" }
        "#,
        &schema,
        &eng,
    )
    .unwrap();

    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Alice".into()))));
    assert_eq!(result.rows[1][0], Some(Term::Literal(Literal::String("Carol".into()))));
}

#[test]
fn test_values_introduces_new_variable() {
    let (schema, store) = setup();
    let eng = InMemoryEngine::new(&store);
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name ?label
        WHERE { ?p ex:name ?name }
        ORDER BY ?name
        VALUES ?label { "tagged" }
        "#,
        &schema,
        &eng,
    )
    .unwrap();

    // Each of 3 people gets joined with the single VALUES row
    assert_eq!(result.rows.len(), 3);
    assert!(result.variables.contains(&"label".to_string()));
    let label_col = result.variables.iter().position(|v| v == "label").unwrap();
    for row in &result.rows {
        assert_eq!(row[label_col], Some(Term::Literal(Literal::String("tagged".into()))));
    }
}

#[test]
fn test_values_multi_var() {
    let (schema, store) = setup();
    let eng = InMemoryEngine::new(&store);
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name ?age
        WHERE { ?p ex:name ?name . ?p ex:age ?age }
        ORDER BY ?name
        VALUES (?name ?age) { ("Alice" 30) ("Bob" 25) }
        "#,
        &schema,
        &eng,
    )
    .unwrap();

    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Alice".into()))));
    assert_eq!(result.rows[0][1], Some(Term::Literal(Literal::Integer(30))));
    assert_eq!(result.rows[1][0], Some(Term::Literal(Literal::String("Bob".into()))));
    assert_eq!(result.rows[1][1], Some(Term::Literal(Literal::Integer(25))));
}

#[test]
fn test_values_with_undef() {
    let (schema, store) = setup();
    let eng = InMemoryEngine::new(&store);
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name ?age
        WHERE { ?p ex:name ?name . ?p ex:age ?age }
        ORDER BY ?name
        VALUES (?name ?age) { ("Alice" 30) ("Bob" UNDEF) }
        "#,
        &schema,
        &eng,
    )
    .unwrap();

    // Alice matches (name=Alice, age=30). Bob matches (name=Bob, age=UNDEF means any age).
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Alice".into()))));
    assert_eq!(result.rows[1][0], Some(Term::Literal(Literal::String("Bob".into()))));
}

#[test]
fn test_values_empty_produces_no_results() {
    let (schema, store) = setup();
    let eng = InMemoryEngine::new(&store);
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name
        WHERE { ?p ex:name ?name }
        VALUES ?name { }
        "#,
        &schema,
        &eng,
    )
    .unwrap();

    assert_eq!(result.rows.len(), 0);
}

#[test]
fn test_values_select_star_includes_values_vars() {
    let (schema, store) = setup();
    let eng = InMemoryEngine::new(&store);
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT *
        WHERE { ?p ex:name ?name }
        ORDER BY ?name
        VALUES ?tag { "x" }
        "#,
        &schema,
        &eng,
    )
    .unwrap();

    assert!(result.variables.contains(&"tag".to_string()));
    assert_eq!(result.rows.len(), 3);
}

#[test]
fn test_values_with_iri() {
    let (schema, store) = setup();
    let eng = InMemoryEngine::new(&store);
    let result = engine::execute(
        r#"
        PREFIX ex: <http://example.org/>
        SELECT ?name
        WHERE { ?p a ex:Person . ?p ex:name ?name }
        VALUES ?p { <http://example.org/person/alice> }
        "#,
        &schema,
        &eng,
    )
    .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Some(Term::Literal(Literal::String("Alice".into()))));
}

#[test]
fn test_cpmeta_not_exists_rewrite_to_null_checks() {
    let input = std::fs::read_to_string("queries/objects.rq").unwrap();
    let query = darq::sparql::parser::parse(&input).unwrap();

    let schema = darq::cpmeta_schema::cpmeta_schema();
    let plans = darq::lower::lower(&query, &schema).unwrap();

    // The query has two FILTER NOT EXISTS:
    // 1. {?spec cpmeta:hasAssociatedProject ?pro . ?pro cpmeta:hasHideFromSearchPolicy true}
    //    → stays as NotExistsFilter (not a single blank-node pattern)
    // 2. {[] cpmeta:isNextVersionOf ?dobj}
    //    → rewritten to NullCheckFilter (blank-node subject, schema rewrite registered)
    assert_eq!(plans.len(), 1);
    let plan = &plans[0];
    assert_eq!(plan.filters.len(), 1, "first NOT EXISTS should remain as filter");
    assert_eq!(plan.null_checks.len(), 1);
    assert_eq!(plan.null_checks[0].variable, "dobj");
    assert_eq!(
        plan.null_checks[0].field_names,
        vec!["deprecated_by_object", "deprecated_by_collection"],
    );

    // Verify the SQL uses IS NULL for the rewritten null-check filter
    let sql = darq::sql::to_sql(plan, &schema, "rdf_subject", "id").unwrap();
    assert!(sql.contains("\"deprecated_by_object\" IS NULL"), "SQL should use IS NULL:\n{}", sql);
    assert!(sql.contains("\"deprecated_by_collection\" IS NULL"), "SQL should use IS NULL:\n{}", sql);
    // The first NOT EXISTS (hasAssociatedProject/hasHideFromSearchPolicy) remains as SQL NOT EXISTS
    // The second NOT EXISTS (isNextVersionOf) was rewritten to IS NULL checks
    assert!(!sql.contains("NOT IN"), "SQL should not contain NOT IN:\n{}", sql);
}
