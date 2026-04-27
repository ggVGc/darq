use crate::rdf::Iri;

pub use spargebra::term::Variable;
pub use spargebra::term::GroundTerm;

#[derive(Debug, Clone)]
pub struct SelectQuery {
    pub select: SelectClause,
    pub where_pattern: GroupGraphPattern,
    pub modifier: SolutionModifier,
    pub values: Option<ValuesClause>,
}

#[derive(Debug, Clone)]
pub enum SelectClause {
    Variables(Vec<Variable>),
    Star,
    Count { variable: Variable },
}

#[derive(Debug, Clone)]
pub struct GroupGraphPattern {
    pub patterns: Vec<TriplePattern>,
    pub filters: Vec<Filter>,
    pub optionals: Vec<OptionalPattern>,
}

#[derive(Debug, Clone)]
pub struct OptionalPattern {
    pub patterns: Vec<TriplePattern>,
    pub filters: Vec<Filter>,
}

#[derive(Debug, Clone)]
pub enum Filter {
    NotExists(GroupGraphPattern),
    Expression(FilterExpr),
}

#[derive(Debug, Clone)]
pub enum FilterExpr {
    Equal(Box<FilterExpr>, Box<FilterExpr>),
    NotEqual(Box<FilterExpr>, Box<FilterExpr>),
    Less(Box<FilterExpr>, Box<FilterExpr>),
    Greater(Box<FilterExpr>, Box<FilterExpr>),
    LessOrEqual(Box<FilterExpr>, Box<FilterExpr>),
    GreaterOrEqual(Box<FilterExpr>, Box<FilterExpr>),
    Or(Box<FilterExpr>, Box<FilterExpr>),
    And(Box<FilterExpr>, Box<FilterExpr>),
    Not(Box<FilterExpr>),
    Variable(Variable),
    Iri(Iri),
    Literal(spargebra::term::Literal),
    Bound(Variable),
    Str(Box<FilterExpr>),
    Contains(Box<FilterExpr>, Box<FilterExpr>),
    Exists(GroupGraphPattern),
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
    Literal(spargebra::term::Literal),
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

/// A VALUES clause: list of variables and rows of values.
/// Each inner Vec in `bindings` has the same length as `variables`.
/// None entries represent UNDEF.
#[derive(Debug, Clone)]
pub struct ValuesClause {
    pub variables: Vec<Variable>,
    pub bindings: Vec<Vec<Option<GroundTerm>>>,
}
