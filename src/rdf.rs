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

/// A 64-bit float that supports Eq/Hash/Ord (via total ordering on bits).
/// NaN values are considered equal to each other and sort last.
#[derive(Debug, Clone, Copy)]
pub struct Float64(pub f64);

impl Float64 {
    fn to_bits_canonical(self) -> u64 {
        let bits = self.0.to_bits();
        // Normalize all NaN representations to a single canonical form.
        if self.0.is_nan() {
            f64::NAN.to_bits()
        } else {
            bits
        }
    }
}

impl PartialEq for Float64 {
    fn eq(&self, other: &Self) -> bool {
        self.to_bits_canonical() == other.to_bits_canonical()
    }
}

impl Eq for Float64 {}

impl std::hash::Hash for Float64 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.to_bits_canonical().hash(state);
    }
}

impl PartialOrd for Float64 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Float64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl std::fmt::Display for Float64 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.fract() == 0.0 && self.0.is_finite() {
            write!(f, "{:.1}", self.0)
        } else {
            write!(f, "{}", self.0)
        }
    }
}

/// An RDF literal value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Literal {
    String(String),
    Integer(i64),
    Boolean(bool),
    Float(Float64),
    Double(Float64),
    /// Arbitrary-precision decimal, stored as a string.
    Decimal(String),
    /// Date in YYYY-MM-DD format.
    Date(String),
    /// DateTime in YYYY-MM-DDTHH:MM:SS format (optional timezone).
    DateTime(String),
}

impl std::fmt::Display for Literal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Literal::String(s) => write!(f, "\"{}\"", s),
            Literal::Integer(n) => write!(f, "{}", n),
            Literal::Boolean(b) => write!(f, "{}", b),
            Literal::Float(v) => write!(f, "{}", v),
            Literal::Double(v) => write!(f, "{}", v),
            Literal::Decimal(s) => write!(f, "{}", s),
            Literal::Date(s) => write!(f, "{}", s),
            Literal::DateTime(s) => write!(f, "{}", s),
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

pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
