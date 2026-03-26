use std::collections::HashMap;

use crate::rdf::{Iri, Term, Triple};
use crate::schema::Resource;

/// A pattern for IRI positions (subject, predicate): either a concrete IRI or a variable.
#[derive(Debug, Clone)]
pub enum IriPattern {
    Bound(Iri),
    Variable(String),
}

/// A pattern for object position: either a concrete term (IRI or literal) or a variable.
#[derive(Debug, Clone)]
pub enum TermPattern {
    Bound(Term),
    Variable(String),
}

/// A triple pattern used for matching against the store.
#[derive(Debug, Clone)]
pub struct TriplePattern {
    pub subject: IriPattern,
    pub predicate: IriPattern,
    pub object: TermPattern,
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

            let subject_matches =
                match_iri(&pattern.subject, &triple.subject, &mut binding);
            let predicate_matches =
                match_iri(&pattern.predicate, &triple.predicate, &mut binding);
            let object_matches =
                match_term(&pattern.object, &triple.object, &mut binding);

            if subject_matches && predicate_matches && object_matches {
                results.push(binding);
            }
        }

        results
    }
}

/// Match an IRI pattern against an actual IRI.
/// Variables bind as `Term::Iri(actual)` so bindings are always Terms.
fn match_iri(pattern: &IriPattern, actual: &Iri, binding: &mut Binding) -> bool {
    match pattern {
        IriPattern::Bound(expected) => expected == actual,
        IriPattern::Variable(name) => {
            let term = Term::Iri(actual.clone());
            if let Some(existing) = binding.get(name) {
                *existing == term
            } else {
                binding.insert(name.clone(), term);
                true
            }
        }
    }
}

/// Match a term pattern against an actual term.
fn match_term(pattern: &TermPattern, actual: &Term, binding: &mut Binding) -> bool {
    match pattern {
        TermPattern::Bound(expected) => expected == actual,
        TermPattern::Variable(name) => {
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
        let pattern = TriplePattern {
            subject: IriPattern::Variable("s".into()),
            predicate: IriPattern::Variable("p".into()),
            object: TermPattern::Variable("o".into()),
        };
        let results = store.match_pattern(&pattern);
        assert_eq!(results.len(), 6);
    }

    #[test]
    fn test_match_by_predicate() {
        let store = test_store();
        let pattern = TriplePattern {
            subject: IriPattern::Variable("s".into()),
            predicate: IriPattern::Bound(Iri::new("http://example.org/name")),
            object: TermPattern::Variable("name".into()),
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
        let pattern = TriplePattern {
            subject: IriPattern::Variable("s".into()),
            predicate: IriPattern::Bound(Iri::new(RDF_TYPE)),
            object: TermPattern::Bound(Term::Iri(Iri::new("http://example.org/Person"))),
        };
        let results = store.match_pattern(&pattern);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_match_specific_subject() {
        let store = test_store();
        let pattern = TriplePattern {
            subject: IriPattern::Bound(Iri::new("http://example.org/person/alice")),
            predicate: IriPattern::Bound(Iri::new("http://example.org/age")),
            object: TermPattern::Variable("age".into()),
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
        // ?x in subject and predicate (both IRIs) — can match if subject IRI == predicate IRI
        // ?y in object — independent
        // In our data, no triple has subject == predicate, so 0 results
        let pattern = TriplePattern {
            subject: IriPattern::Variable("x".into()),
            predicate: IriPattern::Variable("x".into()),
            object: TermPattern::Variable("y".into()),
        };
        let results = store.match_pattern(&pattern);
        assert_eq!(results.len(), 0);
    }
}
