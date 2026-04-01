use std::sync::atomic::{AtomicUsize, Ordering};

use spargebra::algebra::{AggregateExpression, Expression, GraphPattern, OrderExpression};
use spargebra::term::{NamedNodePattern, TermPattern};
use spargebra::{Query, SparqlParser};

use crate::error::DarqError;
use crate::rdf::Iri;
use super::ast::*;

static BLANK_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Parse a SPARQL SELECT query string into our internal AST.
pub fn parse(input: &str) -> Result<SelectQuery, DarqError> {
    BLANK_COUNTER.store(0, Ordering::Relaxed);

    // Detect SELECT * before parsing (spargebra always wraps in Project)
    let is_select_star = detect_select_star(input);

    let query = SparqlParser::new()
        .parse_query(input)
        .map_err(|e| DarqError::ParseError(e.to_string()))?;

    let Query::Select { pattern, .. } = query else {
        return Err(DarqError::ParseError("only SELECT queries are supported".into()));
    };

    unwrap_algebra(pattern, is_select_star)
}

fn detect_select_star(input: &str) -> bool {
    let upper = input.to_ascii_uppercase();
    if let Some(pos) = upper.find("SELECT") {
        let rest = &upper[pos + 6..];
        let trimmed = rest.trim_start();
        let after_modifier = if trimmed.starts_with("DISTINCT") {
            trimmed[8..].trim_start()
        } else if trimmed.starts_with("REDUCED") {
            trimmed[7..].trim_start()
        } else {
            trimmed
        };
        after_modifier.starts_with('*')
    } else {
        false
    }
}

/// Peel off the nested algebra wrappers and convert the body to our flat AST.
fn unwrap_algebra(mut pattern: GraphPattern, is_select_star: bool) -> Result<SelectQuery, DarqError> {
    let mut distinct = false;
    let mut order_by = Vec::new();
    let mut limit = None;
    let mut offset = None;
    let mut select = SelectClause::Star;
    let mut aggregate_alias: Option<Variable> = None;

    loop {
        match pattern {
            GraphPattern::Slice { inner, start, length } => {
                if start > 0 {
                    offset = Some(start);
                }
                limit = length;
                pattern = *inner;
            }
            GraphPattern::OrderBy { inner, expression } => {
                order_by = convert_order_expressions(expression)?;
                pattern = *inner;
            }
            GraphPattern::Distinct { inner } => {
                distinct = true;
                pattern = *inner;
            }
            GraphPattern::Reduced { inner } => {
                pattern = *inner;
            }
            GraphPattern::Project { inner, variables } => {
                if !is_select_star {
                    select = SelectClause::Variables(variables);
                }
                pattern = *inner;
            }
            GraphPattern::Extend { inner, variable, expression } => {
                if matches!(expression, Expression::Variable(_)) {
                    aggregate_alias = Some(variable);
                    pattern = *inner;
                } else {
                    return Err(DarqError::ParseError(
                        "only aggregate aliases are supported in SELECT expressions".into(),
                    ));
                }
            }
            GraphPattern::Group { inner, variables, aggregates } => {
                if !variables.is_empty() {
                    return Err(DarqError::ParseError("GROUP BY is not supported".into()));
                }
                let (_, agg) = match aggregates.into_iter().next() {
                    Some(pair) => pair,
                    None => return Err(DarqError::ParseError("empty aggregate".into())),
                };
                if !matches!(agg, AggregateExpression::CountSolutions { distinct: false }) {
                    return Err(DarqError::ParseError(
                        "only COUNT(*) aggregate is supported".into(),
                    ));
                }
                let alias = aggregate_alias.take().unwrap_or_else(|| {
                    Variable::new_unchecked("count")
                });
                select = SelectClause::Count { variable: alias };
                pattern = *inner;
            }
            _ => break,
        }
    }

    let (where_pattern, values) = extract_body(pattern)?;

    Ok(SelectQuery {
        select,
        where_pattern,
        modifier: SolutionModifier { distinct, order_by, limit, offset },
        values,
    })
}

fn convert_order_expressions(exprs: Vec<OrderExpression>) -> Result<Vec<OrderCondition>, DarqError> {
    exprs.into_iter().map(|expr| {
        match expr {
            OrderExpression::Asc(Expression::Variable(v)) => Ok(OrderCondition {
                variable: v,
                direction: OrderDirection::Ascending,
            }),
            OrderExpression::Desc(Expression::Variable(v)) => Ok(OrderCondition {
                variable: v,
                direction: OrderDirection::Descending,
            }),
            _ => Err(DarqError::ParseError(
                "only ORDER BY on variables (ASC/DESC) is supported".into(),
            )),
        }
    }).collect()
}

/// Extract the WHERE body and optional VALUES clause from the inner pattern.
fn extract_body(pattern: GraphPattern) -> Result<(GroupGraphPattern, Option<ValuesClause>), DarqError> {
    match pattern {
        GraphPattern::Bgp { patterns } => {
            let triples = patterns.into_iter().map(convert_triple).collect::<Result<_, _>>()?;
            Ok((GroupGraphPattern { patterns: triples, filters: vec![] }, None))
        }

        GraphPattern::Filter { expr, inner } => {
            let filters = extract_filters(expr)?;
            let (mut ggp, values) = extract_body(*inner)?;
            ggp.filters = filters;
            Ok((ggp, values))
        }

        GraphPattern::Join { left, right } => {
            if let GraphPattern::Values { variables, bindings } = *right {
                let (ggp, existing_values) = extract_body(*left)?;
                if existing_values.is_some() {
                    return Err(DarqError::ParseError("multiple VALUES clauses not supported".into()));
                }
                return Ok((ggp, Some(ValuesClause { variables, bindings })));
            }
            if let GraphPattern::Values { variables, bindings } = *left {
                let (ggp, existing_values) = extract_body(*right)?;
                if existing_values.is_some() {
                    return Err(DarqError::ParseError("multiple VALUES clauses not supported".into()));
                }
                return Ok((ggp, Some(ValuesClause { variables, bindings })));
            }
            let (left_ggp, left_values) = extract_body(*left)?;
            let (right_ggp, right_values) = extract_body(*right)?;
            if left_values.is_some() && right_values.is_some() {
                return Err(DarqError::ParseError("multiple VALUES clauses not supported".into()));
            }
            let mut patterns = left_ggp.patterns;
            patterns.extend(right_ggp.patterns);
            let mut filters = left_ggp.filters;
            filters.extend(right_ggp.filters);
            Ok((GroupGraphPattern { patterns, filters }, left_values.or(right_values)))
        }

        GraphPattern::Values { variables, bindings } => {
            Ok((GroupGraphPattern { patterns: vec![], filters: vec![] }, Some(ValuesClause { variables, bindings })))
        }

        ref other => Err(DarqError::ParseError(format!(
            "unsupported graph pattern: {}",
            pattern_name(other),
        ))),
    }
}

fn pattern_name(gp: &GraphPattern) -> &'static str {
    match gp {
        GraphPattern::Bgp { .. } => "Bgp",
        GraphPattern::Path { .. } => "Path (property paths)",
        GraphPattern::Join { .. } => "Join",
        GraphPattern::LeftJoin { .. } => "LeftJoin (OPTIONAL)",
        GraphPattern::Filter { .. } => "Filter",
        GraphPattern::Union { .. } => "Union",
        GraphPattern::Graph { .. } => "Graph",
        GraphPattern::Extend { .. } => "Extend (BIND)",
        GraphPattern::Minus { .. } => "Minus",
        GraphPattern::Values { .. } => "Values",
        GraphPattern::OrderBy { .. } => "OrderBy",
        GraphPattern::Project { .. } => "Project",
        GraphPattern::Distinct { .. } => "Distinct",
        GraphPattern::Reduced { .. } => "Reduced",
        GraphPattern::Slice { .. } => "Slice",
        GraphPattern::Group { .. } => "Group (GROUP BY)",
        GraphPattern::Service { .. } => "Service",
    }
}

fn extract_filters(expr: Expression) -> Result<Vec<Filter>, DarqError> {
    match expr {
        Expression::Not(inner) => {
            if let Expression::Exists(gp) = *inner {
                let (ggp, _) = extract_body(*gp)?;
                Ok(vec![Filter::NotExists(ggp)])
            } else {
                Err(DarqError::ParseError("only FILTER NOT EXISTS is supported".into()))
            }
        }
        Expression::And(left, right) => {
            let mut filters = extract_filters(*left)?;
            filters.extend(extract_filters(*right)?);
            Ok(filters)
        }
        _ => Err(DarqError::ParseError(
            "only FILTER NOT EXISTS is supported".into(),
        )),
    }
}

fn convert_triple(tp: spargebra::term::TriplePattern) -> Result<TriplePattern, DarqError> {
    Ok(TriplePattern {
        subject: convert_term_pattern(tp.subject)?,
        predicate: convert_named_node_pattern(tp.predicate),
        object: convert_term_pattern(tp.object)?,
    })
}

fn convert_term_pattern(tp: TermPattern) -> Result<TermOrVariable, DarqError> {
    match tp {
        TermPattern::Variable(v) => Ok(TermOrVariable::Variable(v)),
        TermPattern::NamedNode(nn) => Ok(TermOrVariable::Iri(Iri::new(nn.into_string()))),
        TermPattern::Literal(lit) => Ok(TermOrVariable::Literal(lit)),
        TermPattern::BlankNode(_bn) => {
            let n = BLANK_COUNTER.fetch_add(1, Ordering::Relaxed);
            Ok(TermOrVariable::Variable(Variable::new_unchecked(format!("__anon_{}", n))))
        }
    }
}

fn convert_named_node_pattern(nnp: NamedNodePattern) -> TermOrVariable {
    match nnp {
        NamedNodePattern::NamedNode(nn) => TermOrVariable::Iri(Iri::new(nn.into_string())),
        NamedNodePattern::Variable(v) => TermOrVariable::Variable(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_select() {
        let q = parse("SELECT ?s ?p ?o WHERE { ?s ?p ?o }").unwrap();
        assert!(matches!(q.select, SelectClause::Variables(ref vars) if vars.len() == 3));
        assert_eq!(q.where_pattern.patterns.len(), 1);
    }

    #[test]
    fn test_parse_select_star() {
        let q = parse("SELECT * WHERE { ?s ?p ?o }").unwrap();
        assert!(matches!(q.select, SelectClause::Star));
    }

    #[test]
    fn test_parse_with_prefix() {
        let q = parse("PREFIX ex: <http://example.org/> SELECT * WHERE { ?s ex:name ?o }").unwrap();
        match &q.where_pattern.patterns[0].predicate {
            TermOrVariable::Iri(iri) => assert_eq!(iri.0, "http://example.org/name"),
            other => panic!("expected Iri, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_rdf_type() {
        let q = parse("PREFIX ex: <http://example.org/> SELECT * WHERE { ?s a ex:Person }").unwrap();
        match &q.where_pattern.patterns[0].predicate {
            TermOrVariable::Iri(iri) => assert_eq!(iri.0, "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
            other => panic!("expected rdf:type IRI, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_filter_not_exists() {
        let q = parse("PREFIX ex: <http://example.org/> SELECT * WHERE { ?s ex:name ?o . FILTER NOT EXISTS { ?s ex:age ?a } }").unwrap();
        assert_eq!(q.where_pattern.filters.len(), 1);
        match &q.where_pattern.filters[0] {
            Filter::NotExists(ggp) => assert_eq!(ggp.patterns.len(), 1),
        }
    }

    #[test]
    fn test_parse_order_by_limit_offset() {
        let q = parse("SELECT ?s WHERE { ?s <http://ex.org/p> ?o } ORDER BY ?s LIMIT 10 OFFSET 5").unwrap();
        assert!(!q.modifier.distinct);
        assert_eq!(q.modifier.order_by.len(), 1);
        assert_eq!(q.modifier.limit, Some(10));
        assert_eq!(q.modifier.offset, Some(5));
    }

    #[test]
    fn test_parse_distinct() {
        let q = parse("SELECT DISTINCT ?s WHERE { ?s <http://ex.org/p> ?o }").unwrap();
        assert!(q.modifier.distinct);
    }

    #[test]
    fn test_parse_values() {
        let q = parse("SELECT * WHERE { ?s <http://ex.org/p> ?o } VALUES ?s { <http://ex.org/a> <http://ex.org/b> }").unwrap();
        assert!(q.values.is_some());
        let vc = q.values.unwrap();
        assert_eq!(vc.variables.len(), 1);
        assert_eq!(vc.bindings.len(), 2);
    }

    #[test]
    fn test_parse_blank_node() {
        let q = parse("PREFIX ex: <http://example.org/> SELECT * WHERE { [] ex:name ?o }").unwrap();
        match &q.where_pattern.patterns[0].subject {
            TermOrVariable::Variable(v) => assert!(v.as_str().starts_with("__anon_")),
            other => panic!("expected anonymous variable, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_count_star() {
        let q = parse("PREFIX ex: <http://example.org/> SELECT (COUNT(*) AS ?cnt) WHERE { ?s ex:name ?o }").unwrap();
        match q.select {
            SelectClause::Count { variable } => assert_eq!(variable.as_str(), "cnt"),
            other => panic!("expected Count, got {:?}", other),
        }
        assert_eq!(q.where_pattern.patterns.len(), 1);
    }

    #[test]
    fn test_unknown_prefix_is_parse_error() {
        let result = parse("SELECT ?x WHERE { ?s foaf:name ?x }");
        assert!(result.is_err());
    }
}
