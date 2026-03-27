use std::collections::HashMap;

use crate::rdf::{Iri, Term};
use crate::schema::Resource;

/// A stored resource instance with its type, identity, and field values.
#[derive(Debug, Clone)]
pub struct ResourceInstance {
    pub type_iri: Iri,
    pub subject: Iri,
    pub fields: HashMap<String, Term>,
}

/// Stores resource instances indexed by type and subject IRI.
pub struct ResourceStore {
    by_type: HashMap<Iri, Vec<ResourceInstance>>,
}

impl ResourceStore {
    pub fn new() -> Self {
        ResourceStore {
            by_type: HashMap::new(),
        }
    }

    /// Load a resource instance into the store.
    pub fn load<R: Resource>(&mut self, resource: &R) {
        let type_iri = R::rdf_type();
        let subject = resource.subject_iri();
        let fields: HashMap<String, Term> = R::field_descriptors()
            .into_iter()
            .zip(resource.field_values())
            .map(|(fd, val)| (fd.name.to_string(), val))
            .collect();

        self.by_type
            .entry(type_iri.clone())
            .or_default()
            .push(ResourceInstance {
                type_iri,
                subject,
                fields,
            });
    }

    /// Get all instances of a given type.
    pub fn instances_of(&self, type_iri: &Iri) -> &[ResourceInstance] {
        self.by_type
            .get(type_iri)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Find a resource instance by its subject IRI (searches all types).
    pub fn find_by_subject(&self, subject: &Iri) -> Option<&ResourceInstance> {
        for instances in self.by_type.values() {
            for inst in instances {
                if inst.subject == *subject {
                    return Some(inst);
                }
            }
        }
        None
    }

    /// Iterate over all registered type IRIs.
    pub fn all_types(&self) -> impl Iterator<Item = &Iri> {
        self.by_type.keys()
    }

    /// Iterate over all instances across all types.
    pub fn all_instances(&self) -> impl Iterator<Item = &ResourceInstance> {
        self.by_type.values().flat_map(|v| v.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::{Iri, Literal, Term};
    use crate::schema::{FieldDescriptor, FieldType};

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

    #[test]
    fn test_load_and_query() {
        let mut store = ResourceStore::new();
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

        let person_type = Iri::new("http://example.org/Person");
        let instances = store.instances_of(&person_type);
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].subject, Iri::new("http://example.org/person/alice"));
        assert_eq!(
            instances[0].fields.get("name"),
            Some(&Term::Literal(Literal::String("Alice".into())))
        );
        assert_eq!(
            instances[0].fields.get("age"),
            Some(&Term::Literal(Literal::Integer(30)))
        );
    }

    #[test]
    fn test_find_by_subject() {
        let mut store = ResourceStore::new();
        store.load(&Person {
            id: "alice".into(),
            name: "Alice".into(),
            age: 30,
        });

        let alice = store
            .find_by_subject(&Iri::new("http://example.org/person/alice"))
            .unwrap();
        assert_eq!(
            alice.fields.get("name"),
            Some(&Term::Literal(Literal::String("Alice".into())))
        );

        assert!(store
            .find_by_subject(&Iri::new("http://example.org/person/nobody"))
            .is_none());
    }
}
