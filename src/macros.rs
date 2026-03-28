// ---------------------------------------------------------------------------
// Reusable schema macros
// ---------------------------------------------------------------------------

/// Standard RDF namespace helpers.

#[macro_export]
macro_rules! rdfs {
    ($p:literal) => {
        concat!("http://www.w3.org/2000/01/rdf-schema#", $p)
    };
}

#[macro_export]
macro_rules! dcterms {
    ($p:literal) => {
        concat!("http://purl.org/dc/terms/", $p)
    };
}

#[macro_export]
macro_rules! prov {
    ($p:literal) => {
        concat!("http://www.w3.org/ns/prov#", $p)
    };
}

#[macro_export]
macro_rules! skos {
    ($p:literal) => {
        concat!("http://www.w3.org/2004/02/skos/core#", $p)
    };
}

/// Create a `FieldType::Reference` from one or more IRI expressions.
#[macro_export]
macro_rules! ref_to {
    ($($iri:expr),+) => {
        $crate::schema::FieldType::Reference(vec![$($crate::rdf::Iri::new($iri)),+])
    };
}

/// Define a SQL-backed `Resource` implementation from a declarative schema.
///
/// ```rust,ignore
/// define_resource!(
///     /// Optional doc comment
///     MyResource, "http://example.org/MyResource", [
///         ("column_name", "http://example.org/predicate", FieldType::String),
///     ]
/// );
/// ```
#[macro_export]
macro_rules! define_resource {
    (
        $(#[doc = $doc:expr])*
        $name:ident, $type_iri:expr,
        [ $( ($col:expr, $pred:expr, $ftype:expr) ),* $(,)? ]
    ) => {
        $(#[doc = $doc])*
        pub struct $name;

        impl $crate::schema::Resource for $name {
            fn rdf_type() -> $crate::rdf::Iri {
                $crate::rdf::Iri::new($type_iri)
            }

            fn subject_iri(&self) -> $crate::rdf::Iri {
                unimplemented!("SQL-backed resource")
            }

            fn field_descriptors() -> Vec<$crate::schema::FieldDescriptor> {
                vec![
                    $($crate::schema::FieldDescriptor {
                        predicate: $crate::rdf::Iri::new($pred),
                        name: $col,
                        field_type: $ftype,
                        indexed: false,
                    }),*
                ]
            }

            fn field_values(&self) -> Vec<$crate::rdf::Term> {
                unimplemented!("SQL-backed resource")
            }
        }
    };
}
