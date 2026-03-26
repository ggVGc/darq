use std::collections::HashMap;

use crate::rdf::{Term, Triple};
use crate::schema::Resource;

/// A pattern component: either a concrete term to match or a variable to bind.
#[derive(Debug, Clone)]
pub enum PatternTerm {
    Bound(Term),
    Variable(String),
}

/// A triple pattern used for matching against the store.
#[derive(Debug, Clone)]
pub struct TriplePattern {
    pub subject: PatternTerm,
    pub predicate: PatternTerm,
    pub object: PatternTerm,
}

/// One solution row: a mapping from variable names to bound terms.
pub type Binding = HashMap<String, Term>;

/// Simple triple store backed by a Vec.
pub struct TripleStore {
    triples: Vec<Triple>,
}

impl TripleStore {
    pub fn new() -> Self {
        TripleStore {
            triples: Vec::new(),
        }
    }

    /// Add triples from a Resource instance.
    pub fn load<R: Resource>(&mut self, resource: &R) {
        self.triples.extend(resource.to_triples());
    }

    /// Find all triples matching a pattern, returning variable bindings.
    pub fn match_pattern(&self, pattern: &TriplePattern) -> Vec<Binding> {
        let mut results = Vec::new();

        for triple in &self.triples {
            let mut binding = Binding::new();

            let subject_matches = match_component(
                &pattern.subject,
                &triple.subject,
                &mut binding,
            );
            let predicate_matches = match_component(
                &pattern.predicate,
                &Term::Iri(triple.predicate.clone()),
                &mut binding,
            );
            let object_matches = match_component(
                &pattern.object,
                &triple.object,
                &mut binding,
            );

            if subject_matches && predicate_matches && object_matches {
                results.push(binding);
            }
        }

        results
    }
}

/// Try to match a pattern component against an actual term.
/// If the pattern is a variable, bind it (or check consistency if already bound).
/// If the pattern is a bound term, check equality.
fn match_component(
    pattern: &PatternTerm,
    actual: &Term,
    binding: &mut Binding,
) -> bool {
    match pattern {
        PatternTerm::Bound(expected) => expected == actual,
        PatternTerm::Variable(name) => {
            if let Some(existing) = binding.get(name) {
                existing == actual
            } else {
                binding.insert(name.clone(), actual.clone());
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::{Iri, Literal, Term, RDF_TYPE};
    use crate::schema::{FieldDescriptor, Resource};

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
                },
                FieldDescriptor {
                    predicate: Iri::new("http://example.org/age"),
                    name: "age",
                },
            ]
        }

        fn field_values(&self) -> Vec<Term> {
            vec![self.name.clone().into(), self.age.into()]
        }
    }

    fn test_store() -> TripleStore {
        let mut store = TripleStore::new();
        store.load(&Person {
            id: "alice".into(),
            name: "Alice".into(),
            age: 30,
        });
        store.load(&Person {
            id: "bob".into(),
            name: "Bob".into(),
            age: 25,
        });
        store
    }

    #[test]
    fn test_match_all() {
        let store = test_store();
        // ?s ?p ?o — matches all 6 triples
        let pattern = TriplePattern {
            subject: PatternTerm::Variable("s".into()),
            predicate: PatternTerm::Variable("p".into()),
            object: PatternTerm::Variable("o".into()),
        };
        let results = store.match_pattern(&pattern);
        assert_eq!(results.len(), 6);
    }

    #[test]
    fn test_match_by_predicate() {
        let store = test_store();
        // ?s <http://example.org/name> ?name
        let pattern = TriplePattern {
            subject: PatternTerm::Variable("s".into()),
            predicate: PatternTerm::Bound(Term::Iri(Iri::new("http://example.org/name"))),
            object: PatternTerm::Variable("name".into()),
        };
        let results = store.match_pattern(&pattern);
        assert_eq!(results.len(), 2);

        let names: Vec<_> = results
            .iter()
            .map(|b| b.get("name").unwrap().clone())
            .collect();
        assert!(names.contains(&Term::Literal(Literal::String("Alice".into()))));
        assert!(names.contains(&Term::Literal(Literal::String("Bob".into()))));
    }

    #[test]
    fn test_match_by_type() {
        let store = test_store();
        // ?s rdf:type <http://example.org/Person>
        let pattern = TriplePattern {
            subject: PatternTerm::Variable("s".into()),
            predicate: PatternTerm::Bound(Term::Iri(Iri::new(RDF_TYPE))),
            object: PatternTerm::Bound(Term::Iri(Iri::new("http://example.org/Person"))),
        };
        let results = store.match_pattern(&pattern);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_match_specific_subject() {
        let store = test_store();
        // <http://example.org/person/alice> <http://example.org/age> ?age
        let pattern = TriplePattern {
            subject: PatternTerm::Bound(Term::Iri(Iri::new(
                "http://example.org/person/alice",
            ))),
            predicate: PatternTerm::Bound(Term::Iri(Iri::new("http://example.org/age"))),
            object: PatternTerm::Variable("age".into()),
        };
        let results = store.match_pattern(&pattern);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].get("age").unwrap(),
            &Term::Literal(Literal::Integer(30))
        );
    }

    #[test]
    fn test_same_variable_consistency() {
        let store = test_store();
        // ?x ?x ?x — subject=predicate=object, should match nothing in our data
        let pattern = TriplePattern {
            subject: PatternTerm::Variable("x".into()),
            predicate: PatternTerm::Variable("x".into()),
            object: PatternTerm::Variable("x".into()),
        };
        let results = store.match_pattern(&pattern);
        assert_eq!(results.len(), 0);
    }
}
