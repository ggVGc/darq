use crate::rdf::{Iri, Term};
use crate::schema::{FieldDescriptor, FieldType, Resource, Schema};

pub struct Person {
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
        vec![self._name.clone().into(), self._age.into()]
    }
}

pub fn test_schema() -> Schema {
    let mut schema = Schema::new();
    schema.register::<Person>();
    schema
}
