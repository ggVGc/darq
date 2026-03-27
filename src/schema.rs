use std::collections::{HashMap, HashSet};

use crate::rdf::{Float64, Iri, Literal, Term, RDF_TYPE};

/// What kind of value a field holds, following XSD type vocabulary.
#[derive(Debug, Clone)]
pub enum FieldType {
    String,
    Integer,
    Boolean,
    Float,
    Double,
    Decimal,
    Date,
    DateTime,
    /// IRI reference to one of the listed resource types.
    Reference(Vec<Iri>),
}

/// Describes one field on a Resource: its predicate IRI, Rust field name, and value type.
#[derive(Debug, Clone)]
pub struct FieldDescriptor {
    pub predicate: Iri,
    pub name: &'static str,
    pub field_type: FieldType,
    pub indexed: bool,
}

/// Implemented by any Rust type that can be stored and queried as a resource.
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
}

/// Static information about a registered resource type.
pub struct TypeInfo {
    pub type_iri: Iri,
    pub fields: Vec<FieldDescriptor>,
}

/// The schema knows every registered resource type and its fields.
/// Used to validate queries and map predicates to field names.
pub struct Schema {
    types: HashMap<Iri, TypeInfo>,
    predicate_to_types: HashMap<Iri, Vec<Iri>>,
}

impl Schema {
    pub fn new() -> Self {
        Schema {
            types: HashMap::new(),
            predicate_to_types: HashMap::new(),
        }
    }

    /// Register a Resource type and all its predicates.
    pub fn register<R: Resource>(&mut self) {
        let type_iri = R::rdf_type();
        let fields = R::field_descriptors();

        for fd in &fields {
            self.predicate_to_types
                .entry(fd.predicate.clone())
                .or_default()
                .push(type_iri.clone());
        }

        self.types.insert(
            type_iri.clone(),
            TypeInfo {
                type_iri,
                fields,
            },
        );
    }

    /// Check whether a predicate IRI is known.
    pub fn is_known_predicate(&self, pred: &Iri) -> bool {
        *pred == Iri::new(RDF_TYPE) || self.predicate_to_types.contains_key(pred)
    }

    /// Return all known predicate IRIs (including rdf:type).
    pub fn known_predicates(&self) -> HashSet<Iri> {
        let mut preds: HashSet<Iri> = self.predicate_to_types.keys().cloned().collect();
        preds.insert(Iri::new(RDF_TYPE));
        preds
    }

    /// Look up the field name for a predicate on a given type.
    pub fn field_name(&self, type_iri: &Iri, predicate: &Iri) -> Option<&str> {
        self.types.get(type_iri).and_then(|info| {
            info.fields
                .iter()
                .find(|fd| fd.predicate == *predicate)
                .map(|fd| fd.name)
        })
    }

    /// Look up the predicate IRI for a field name on a given type.
    pub fn predicate_for_field(&self, type_iri: &Iri, field_name: &str) -> Option<&Iri> {
        self.types.get(type_iri).and_then(|info| {
            info.fields
                .iter()
                .find(|fd| fd.name == field_name)
                .map(|fd| &fd.predicate)
        })
    }

    /// Return which types have a given predicate.
    pub fn types_for_predicate(&self, predicate: &Iri) -> &[Iri] {
        self.predicate_to_types
            .get(predicate)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Return the field descriptors for a type.
    pub fn fields_for_type(&self, type_iri: &Iri) -> Option<&[FieldDescriptor]> {
        self.types.get(type_iri).map(|info| info.fields.as_slice())
    }

    /// Iterate over all registered type IRIs.
    pub fn known_types(&self) -> impl Iterator<Item = &Iri> {
        self.types.keys()
    }

    /// Return the resource types that a reference-typed predicate can point to.
    /// Collects targets from all types that declare this predicate as a Reference.
    /// Returns an empty vec for literal fields or unknown predicates.
    pub fn range_types(&self, predicate: &Iri) -> Vec<Iri> {
        let mut targets = Vec::new();
        for info in self.types.values() {
            for fd in &info.fields {
                if fd.predicate == *predicate {
                    if let FieldType::Reference(ref iris) = fd.field_type {
                        targets.extend(iris.iter().cloned());
                    }
                }
            }
        }
        targets
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

impl From<f64> for Term {
    fn from(v: f64) -> Self {
        Term::Literal(Literal::Double(Float64(v)))
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

    #[test]
    fn test_schema_register() {
        let mut schema = Schema::new();
        schema.register::<Person>();

        assert!(schema.is_known_predicate(&Iri::new(RDF_TYPE)));
        assert!(schema.is_known_predicate(&Iri::new("http://example.org/name")));
        assert!(schema.is_known_predicate(&Iri::new("http://example.org/age")));
        assert!(!schema.is_known_predicate(&Iri::new("http://example.org/email")));
    }

    #[test]
    fn test_schema_type_lookups() {
        let mut schema = Schema::new();
        schema.register::<Person>();

        let person_type = Iri::new("http://example.org/Person");
        let name_pred = Iri::new("http://example.org/name");

        assert_eq!(schema.field_name(&person_type, &name_pred), Some("name"));
        assert_eq!(
            schema.predicate_for_field(&person_type, "name"),
            Some(&name_pred)
        );
        assert_eq!(schema.types_for_predicate(&name_pred), &[person_type.clone()]);
        assert!(schema.fields_for_type(&person_type).is_some());
        assert_eq!(schema.fields_for_type(&person_type).unwrap().len(), 2);
    }
}
