use std::sync::atomic::{AtomicUsize, Ordering};
use std::cell::RefCell;
use std::collections::HashMap;

use spargebra::algebra::{AggregateExpression, Expression, GraphPattern, OrderExpression, PropertyPathExpression};
use spargebra::term::{NamedNodePattern, TermPattern};
use spargebra::{Query, SparqlParser};

use crate::error::DarqError;
use crate::rdf::Iri;
use super::ast::*;

static BLANK_COUNTER: AtomicUsize = AtomicUsize::new(0);
static PATH_COUNTER: AtomicUsize = AtomicUsize::new(0);

// Thread-local map to track blank node IDs and their mapped variable names
// within a single parse call
thread_local! {
    static BLANK_NODE_MAP: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Parse a SPARQL SELECT query string into our internal AST.
pub fn parse(input: &str) -> Result<SelectQuery, DarqError> {
    BLANK_COUNTER.store(0, Ordering::Relaxed);
    PATH_COUNTER.store(0, Ordering::Relaxed);
    BLANK_NODE_MAP.with(|m| m.borrow_mut().clear());

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

/// Desugar a property path expression into a sequence of triple patterns.
///
/// For example, `?dobj a/b ?obj` becomes:
///   ?dobj a ?__path_0 .
///   ?__path_0 b ?obj .
///
/// Note: spargebra currently desugars property paths to Bgp patterns with blank node
/// intermediates before we see them. This handler is for potential future use if:
/// 1. The parser produces GraphPattern::Path nodes
/// 2. We need explicit control over path desugaring
fn desugar_path(
    subject: TermOrVariable,
    path: &PropertyPathExpression,
    object: TermOrVariable,
    triples: &mut Vec<TriplePattern>,
) -> Result<(), DarqError> {
    match path {
        PropertyPathExpression::NamedNode(nn) => {
            let iri = Iri::new(nn.as_str().to_string());
            triples.push(TriplePattern {
                subject,
                predicate: TermOrVariable::Iri(iri),
                object,
            });
            Ok(())
        }
        PropertyPathExpression::Sequence(left, right) => {
            let n = PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
            let intermediate = TermOrVariable::Variable(Variable::new_unchecked(format!("__path_{}", n)));
            desugar_path(subject, left, intermediate.clone(), triples)?;
            desugar_path(intermediate, right, object, triples)?;
            Ok(())
        }
        PropertyPathExpression::Reverse(..) => {
            Err(DarqError::ParseError("property path operator Reverse (^) is not supported".into()))
        }
        PropertyPathExpression::Alternative(..) => {
            Err(DarqError::ParseError("property path operator Alternative (|) is not supported".into()))
        }
        PropertyPathExpression::ZeroOrMore(..) => {
            Err(DarqError::ParseError("property path operator ZeroOrMore (*) is not supported".into()))
        }
        PropertyPathExpression::OneOrMore(..) => {
            Err(DarqError::ParseError("property path operator OneOrMore (+) is not supported".into()))
        }
        PropertyPathExpression::ZeroOrOne(..) => {
            Err(DarqError::ParseError("property path operator ZeroOrOne (?) is not supported".into()))
        }
        PropertyPathExpression::NegatedPropertySet(..) => {
            Err(DarqError::ParseError("property path operator NegatedPropertySet is not supported".into()))
        }
    }
}

/// Extract the WHERE body and optional VALUES clause from the inner pattern.
fn extract_body(pattern: GraphPattern) -> Result<(GroupGraphPattern, Option<ValuesClause>), DarqError> {
    match pattern {
        GraphPattern::Bgp { patterns } => {
            let triples = patterns.into_iter().map(convert_triple).collect::<Result<_, _>>()?;
            Ok((GroupGraphPattern { patterns: triples, filters: vec![] }, None))
        }

        GraphPattern::Path { subject, path, object } => {
            let subject_tv = convert_term_pattern(subject)?;
            let object_tv = convert_term_pattern(object)?;
            let mut triples = Vec::new();
            desugar_path(subject_tv, &path, object_tv, &mut triples)?;
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
                Ok(vec![Filter::Expression(FilterExpr::Not(
                    Box::new(convert_expression(*inner)?),
                ))])
            }
        }
        Expression::And(left, right) => {
            let mut filters = extract_filters(*left)?;
            filters.extend(extract_filters(*right)?);
            Ok(filters)
        }
        other => {
            Ok(vec![Filter::Expression(convert_expression(other)?)])
        }
    }
}

fn convert_expression(expr: Expression) -> Result<FilterExpr, DarqError> {
    match expr {
        Expression::Variable(v) => Ok(FilterExpr::Variable(v)),
        Expression::NamedNode(nn) => Ok(FilterExpr::Iri(Iri::new(nn.into_string()))),
        Expression::Literal(lit) => Ok(FilterExpr::Literal(lit)),
        Expression::Equal(left, right) => Ok(FilterExpr::Equal(
            Box::new(convert_expression(*left)?),
            Box::new(convert_expression(*right)?),
        )),
        Expression::SameTerm(left, right) => Ok(FilterExpr::Equal(
            Box::new(convert_expression(*left)?),
            Box::new(convert_expression(*right)?),
        )),
        Expression::Greater(left, right) => Ok(FilterExpr::Greater(
            Box::new(convert_expression(*left)?),
            Box::new(convert_expression(*right)?),
        )),
        Expression::GreaterOrEqual(left, right) => Ok(FilterExpr::GreaterOrEqual(
            Box::new(convert_expression(*left)?),
            Box::new(convert_expression(*right)?),
        )),
        Expression::Less(left, right) => Ok(FilterExpr::Less(
            Box::new(convert_expression(*left)?),
            Box::new(convert_expression(*right)?),
        )),
        Expression::LessOrEqual(left, right) => Ok(FilterExpr::LessOrEqual(
            Box::new(convert_expression(*left)?),
            Box::new(convert_expression(*right)?),
        )),
        Expression::Or(left, right) => Ok(FilterExpr::Or(
            Box::new(convert_expression(*left)?),
            Box::new(convert_expression(*right)?),
        )),
        Expression::And(left, right) => Ok(FilterExpr::And(
            Box::new(convert_expression(*left)?),
            Box::new(convert_expression(*right)?),
        )),
        Expression::Not(inner) => {
            if let Expression::Exists(gp) = *inner {
                let (ggp, _) = extract_body(*gp)?;
                Ok(FilterExpr::Not(Box::new(FilterExpr::Exists(ggp))))
            } else {
                Ok(FilterExpr::Not(Box::new(convert_expression(*inner)?)))
            }
        }
        Expression::Bound(v) => Ok(FilterExpr::Bound(v)),
        Expression::Exists(gp) => {
            let (ggp, _) = extract_body(*gp)?;
            Ok(FilterExpr::Exists(ggp))
        }
        Expression::FunctionCall(func, args) => {
            convert_function_call(func, args)
        }
        _ => Err(DarqError::ParseError(format!(
            "unsupported FILTER expression: {:?}", expr
        ))),
    }
}

fn convert_function_call(
    func: spargebra::algebra::Function,
    args: Vec<Expression>,
) -> Result<FilterExpr, DarqError> {
    use spargebra::algebra::Function;
    match func {
        Function::Str => {
            if args.len() != 1 {
                return Err(DarqError::ParseError("str() takes exactly 1 argument".into()));
            }
            Ok(FilterExpr::Str(Box::new(convert_expression(args.into_iter().next().unwrap())?)))
        }
        Function::Contains => {
            if args.len() != 2 {
                return Err(DarqError::ParseError("CONTAINS() takes exactly 2 arguments".into()));
            }
            let mut iter = args.into_iter();
            Ok(FilterExpr::Contains(
                Box::new(convert_expression(iter.next().unwrap())?),
                Box::new(convert_expression(iter.next().unwrap())?),
            ))
        }
        _ => Err(DarqError::ParseError(format!(
            "unsupported function: {:?}", func
        ))),
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
        TermPattern::BlankNode(bn) => {
            let bn_str = bn.as_str().to_string();
            let var_name = BLANK_NODE_MAP.with(|m| {
                let mut map = m.borrow_mut();
                map.entry(bn_str).or_insert_with(|| {
                    let n = BLANK_COUNTER.fetch_add(1, Ordering::Relaxed);
                    format!("__anon_{}", n)
                }).clone()
            });
            Ok(TermOrVariable::Variable(Variable::new_unchecked(var_name)))
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
            _ => panic!("expected NotExists filter"),
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

    #[test]
    fn test_parse_property_path_sequence() {
        // Test parsing a property path sequence with correct blank node mapping
        let query = "prefix cpmeta: <http://meta.icos-cp.eu/ontologies/cpmeta/> \
                     prefix prov: <http://www.w3.org/ns/prov#> \
                     select * where { ?dobj cpmeta:wasSubmittedBy/prov:endedAtTime ?submTime . }";
        let q = parse(query).unwrap();

        // Should desugar into 2 triple patterns with an intermediate variable
        assert_eq!(q.where_pattern.patterns.len(), 2);

        // Check first predicate
        if let TermOrVariable::Iri(p1) = &q.where_pattern.patterns[0].predicate {
            assert_eq!(p1.0, "http://meta.icos-cp.eu/ontologies/cpmeta/wasSubmittedBy");
        } else {
            panic!("expected Iri predicate for first triple");
        }

        // Check second predicate
        if let TermOrVariable::Iri(p2) = &q.where_pattern.patterns[1].predicate {
            assert_eq!(p2.0, "http://www.w3.org/ns/prov#endedAtTime");
        } else {
            panic!("expected Iri predicate for second triple");
        }

        // The subject of the second triple should match the object of the first
        let first_object = &q.where_pattern.patterns[0].object;
        let second_subject = &q.where_pattern.patterns[1].subject;
        assert_eq!(
            format!("{:?}", first_object),
            format!("{:?}", second_subject),
            "intermediate variable should be same"
        );
    }

    #[test]
    fn test_parse_simple_property_path() {
        // Test with simpler property path syntax
        let query = "prefix ex: <http://example.org/> SELECT ?x WHERE { ?s ex:a/ex:b ?x }";
        let q = parse(query).unwrap();

        assert_eq!(q.where_pattern.patterns.len(), 2);

        // Check both predicates
        if let TermOrVariable::Iri(p1) = &q.where_pattern.patterns[0].predicate {
            assert_eq!(p1.0, "http://example.org/a");
        } else {
            panic!("expected Iri predicate for first triple");
        }

        if let TermOrVariable::Iri(p2) = &q.where_pattern.patterns[1].predicate {
            assert_eq!(p2.0, "http://example.org/b");
        } else {
            panic!("expected Iri predicate for second triple");
        }

        // Check that the intermediate variable is consistent
        let first_object = &q.where_pattern.patterns[0].object;
        let second_subject = &q.where_pattern.patterns[1].subject;
        assert_eq!(
            format!("{:?}", first_object),
            format!("{:?}", second_subject),
            "intermediate variable should be same"
        );
    }

    #[test]
    fn test_parse_submission_times_query() {
        // Test parsing the actual submission_times.rq query with property paths
        let query = "prefix cpmeta: <http://meta.icos-cp.eu/ontologies/cpmeta/>
prefix prov: <http://www.w3.org/ns/prov#>
select *
where {
  ?dobj cpmeta:wasSubmittedBy/prov:endedAtTime ?submTime .
}
  offset 0 limit 20";
        let q = parse(query).unwrap();

        // Check that we got the right number of triple patterns
        assert_eq!(q.where_pattern.patterns.len(), 2);

        // Verify the predicates
        assert!(matches!(&q.where_pattern.patterns[0].predicate, TermOrVariable::Iri(iri) if iri.0.contains("wasSubmittedBy")));
        assert!(matches!(&q.where_pattern.patterns[1].predicate, TermOrVariable::Iri(iri) if iri.0.contains("endedAtTime")));

        // Verify the variables
        assert!(matches!(&q.where_pattern.patterns[0].subject, TermOrVariable::Variable(_)));
        assert!(matches!(&q.where_pattern.patterns[1].object, TermOrVariable::Variable(_)));

        // Verify limit and offset
        assert_eq!(q.modifier.limit, Some(20));
        // offset 0 is semantically equivalent to no offset
        assert!(q.modifier.offset.is_none() || q.modifier.offset == Some(0));
    }
}
