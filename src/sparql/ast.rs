use crate::rdf::Iri;

#[derive(Debug, Clone)]
pub struct SelectQuery {
    pub prefixes: Vec<PrefixDecl>,
    pub base: Option<Iri>,
    pub select: SelectClause,
    pub where_pattern: GroupGraphPattern,
    pub modifier: SolutionModifier,
    pub values: Option<ValuesClause>,
}

#[derive(Debug, Clone)]
pub struct PrefixDecl {
    pub prefix: String, // e.g. "foaf" (without the colon)
    pub iri: Iri,
}

#[derive(Debug, Clone)]
pub enum SelectClause {
    Variables(Vec<Variable>),
    Star,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Variable(pub String); // name without ? or $

#[derive(Debug, Clone)]
pub struct GroupGraphPattern {
    pub patterns: Vec<TriplePattern>,
}

#[derive(Debug, Clone)]
pub struct TriplePattern {
    pub subject: TermOrVariable,
    pub predicate: TermOrVariable,
    pub object: TermOrVariable,
}

#[derive(Debug, Clone)]
pub enum TermOrVariable {
    Variable(Variable),
    Iri(Iri),
    PrefixedName { prefix: String, local: String },
    RdfType, // the 'a' keyword
    Literal(AstLiteral),
}

#[derive(Debug, Clone)]
pub enum AstLiteral {
    String(String),
    Integer(i64),
    Boolean(bool),
}

#[derive(Debug, Clone, Default)]
pub struct SolutionModifier {
    pub distinct: bool,
    pub order_by: Vec<OrderCondition>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct OrderCondition {
    pub variable: Variable,
    pub direction: OrderDirection,
}

#[derive(Debug, Clone)]
pub enum OrderDirection {
    Ascending,
    Descending,
}

/// A single data value in a VALUES block.
#[derive(Debug, Clone)]
pub enum DataBlockValue {
    Iri(Iri),
    PrefixedName { prefix: String, local: String },
    Literal(AstLiteral),
    Undef,
}

/// A VALUES clause: list of variables and rows of values.
/// Each inner Vec in `bindings` has the same length as `variables`.
#[derive(Debug, Clone)]
pub struct ValuesClause {
    pub variables: Vec<Variable>,
    pub bindings: Vec<Vec<DataBlockValue>>,
}
