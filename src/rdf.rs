/// A full IRI, stored as a plain String.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Iri(pub String);

impl Iri {
    pub fn new(s: impl Into<String>) -> Self {
        Iri(s.into())
    }
}

impl std::fmt::Display for Iri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{}>", self.0)
    }
}

/// An RDF literal value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Literal {
    String(String),
    Integer(i64),
    Boolean(bool),
}

impl std::fmt::Display for Literal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Literal::String(s) => write!(f, "\"{}\"", s),
            Literal::Integer(n) => write!(f, "{}", n),
            Literal::Boolean(b) => write!(f, "{}", b),
        }
    }
}

/// An RDF term: either an IRI or a literal.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Term {
    Iri(Iri),
    Literal(Literal),
}

impl std::fmt::Display for Term {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Term::Iri(iri) => write!(f, "{}", iri),
            Term::Literal(lit) => write!(f, "{}", lit),
        }
    }
}

/// A single RDF triple. Predicate is always an IRI.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Triple {
    pub subject: Term,
    pub predicate: Iri,
    pub object: Term,
}

pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
