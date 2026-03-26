use crate::rdf::Iri;

#[derive(Debug)]
pub enum DarqError {
    /// SPARQL syntax error.
    ParseError(String),
    /// A prefix was used but never declared.
    UnknownPrefix(String),
    /// A predicate IRI in the query is not in the schema.
    UnknownPredicate(Iri),
    /// A type IRI in the query is not registered in the schema.
    UnknownType(Iri),
    /// The type for a subject could not be determined unambiguously.
    AmbiguousType {
        subject: String,
        candidates: Vec<Iri>,
    },
}

impl std::fmt::Display for DarqError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DarqError::ParseError(msg) => write!(f, "parse error: {}", msg),
            DarqError::UnknownPrefix(p) => write!(f, "unknown prefix: {}", p),
            DarqError::UnknownPredicate(iri) => write!(f, "unknown predicate: {}", iri),
            DarqError::UnknownType(iri) => write!(f, "unknown type: {}", iri),
            DarqError::AmbiguousType { subject, candidates } => {
                write!(f, "ambiguous type for ?{}: candidates {:?}", subject, candidates)
            }
        }
    }
}

impl std::error::Error for DarqError {}
