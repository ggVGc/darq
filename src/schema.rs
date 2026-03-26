use std::collections::HashSet;

use crate::rdf::{Iri, Literal, Term, Triple, RDF_TYPE};

/// Describes one field on a Resource: its predicate IRI and Rust field name.
pub struct FieldDescriptor {
    pub predicate: Iri,
    pub name: &'static str,
}

/// Implemented by any Rust type that can be projected into RDF triples.
pub trait Resource {
    /// The rdf:type IRI for this resource (e.g. `<http://example.org/Person>`).
    fn rdf_type() -> Iri;

    /// The IRI that identifies this particular instance (the subject).
    fn subject_iri(&self) -> Iri;

    /// All field descriptors for this type. This is the static schema:
    /// it tells the system which predicates are valid.
    fn field_descriptors() -> Vec<FieldDescriptor>;

    /// Return the object Term for each field, in the same order as
    /// `field_descriptors()`.
    fn field_values(&self) -> Vec<Term>;

    /// Convert this instance into a set of triples.
    fn to_triples(&self) -> Vec<Triple> {
        let subject = Term::Iri(self.subject_iri());
        let mut triples = Vec::new();

        // rdf:type triple
        triples.push(Triple {
            subject: subject.clone(),
            predicate: Iri::new(RDF_TYPE),
            object: Term::Iri(Self::rdf_type()),
        });

        // One triple per field
        for (descriptor, value) in Self::field_descriptors()
            .into_iter()
            .zip(self.field_values())
        {
            triples.push(Triple {
                subject: subject.clone(),
                predicate: descriptor.predicate,
                object: value,
            });
        }

        triples
    }
}

/// The schema knows every valid predicate across all registered resource types.
/// Used to validate queries before execution.
pub struct Schema {
    known_predicates: HashSet<Iri>,
}

impl Schema {
    pub fn new() -> Self {
        let mut known_predicates = HashSet::new();
        // rdf:type is always valid
        known_predicates.insert(Iri::new(RDF_TYPE));
        Schema { known_predicates }
    }

    /// Register a Resource type's predicates.
    pub fn register<R: Resource>(&mut self) {
        for fd in R::field_descriptors() {
            self.known_predicates.insert(fd.predicate);
        }
    }

    /// Check whether a predicate IRI is known.
    pub fn is_known_predicate(&self, pred: &Iri) -> bool {
        self.known_predicates.contains(pred)
    }

    /// Return all known predicate IRIs.
    pub fn known_predicates(&self) -> &HashSet<Iri> {
        &self.known_predicates
    }
}

// Convenience constructors for Term from common Rust types.
impl From<&str> for Term {
    fn from(s: &str) -> Self {
        Term::Literal(Literal::String(s.to_string()))
    }
}

impl From<String> for Term {
    fn from(s: String) -> Self {
        Term::Literal(Literal::String(s))
    }
}

impl From<i64> for Term {
    fn from(n: i64) -> Self {
        Term::Literal(Literal::Integer(n))
    }
}

impl From<bool> for Term {
    fn from(b: bool) -> Self {
        Term::Literal(Literal::Boolean(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_to_triples() {
        let alice = Person {
            id: "alice".into(),
            name: "Alice".into(),
            age: 30,
        };
        let triples = alice.to_triples();
        assert_eq!(triples.len(), 3);

        // rdf:type triple
        assert_eq!(triples[0].predicate, Iri::new(RDF_TYPE));
        assert_eq!(
            triples[0].object,
            Term::Iri(Iri::new("http://example.org/Person"))
        );

        // name triple
        assert_eq!(
            triples[1].predicate,
            Iri::new("http://example.org/name")
        );
        assert_eq!(triples[1].object, Term::Literal(Literal::String("Alice".into())));

        // age triple
        assert_eq!(triples[2].predicate, Iri::new("http://example.org/age"));
        assert_eq!(triples[2].object, Term::Literal(Literal::Integer(30)));
    }

    #[test]
    fn test_schema_register() {
        let mut schema = Schema::new();
        schema.register::<Person>();

        assert!(schema.is_known_predicate(&Iri::new(RDF_TYPE)));
        assert!(schema.is_known_predicate(&Iri::new("http://example.org/name")));
        assert!(schema.is_known_predicate(&Iri::new("http://example.org/age")));
        assert!(!schema.is_known_predicate(&Iri::new("http://example.org/email")));
    }
}
