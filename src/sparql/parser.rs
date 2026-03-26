use nom::{
    IResult,
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while, take_while1},
    character::complete::{char, digit1},
    combinator::{map, map_res, opt, value},
    multi::separated_list1,
    sequence::{delimited, pair, terminated, tuple},
};

use crate::error::DarqError;
use crate::rdf::Iri;
use super::ast::*;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn parse(input: &str) -> Result<SelectQuery, DarqError> {
    let input = input.trim();
    match select_query(input) {
        Ok(("", query)) => Ok(query),
        Ok((rest, _)) => Err(DarqError::ParseError(format!(
            "unexpected trailing input: {:?}",
            truncate(rest, 50)
        ))),
        Err(e) => Err(DarqError::ParseError(format!("{}", e))),
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}

// ---------------------------------------------------------------------------
// Whitespace / comments
// ---------------------------------------------------------------------------

fn ws(input: &str) -> IResult<&str, &str> {
    take_while(|c: char| c.is_ascii_whitespace())(input)
}

/// Consume at least some whitespace (required separator).
fn ws1(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_ascii_whitespace())(input)
}

// ---------------------------------------------------------------------------
// Top-level: SELECT query
// ---------------------------------------------------------------------------

fn select_query(input: &str) -> IResult<&str, SelectQuery> {
    let (input, (base, prefixes)) = prologue(input)?;
    let (input, _) = ws(input)?;
    let (input, (select, distinct)) = select_clause(input)?;
    let (input, _) = ws(input)?;
    let (input, where_pattern) = where_clause(input)?;
    let (input, _) = ws(input)?;
    let (input, mut modifier) = solution_modifier(input)?;
    modifier.distinct = distinct;

    Ok((
        input,
        SelectQuery {
            prefixes,
            base,
            select,
            where_pattern,
            modifier,
        },
    ))
}

// ---------------------------------------------------------------------------
// Prologue: BASE and PREFIX declarations
// ---------------------------------------------------------------------------

fn prologue(input: &str) -> IResult<&str, (Option<Iri>, Vec<PrefixDecl>)> {
    let mut base = None;
    let mut prefixes = Vec::new();
    let mut input = input;

    loop {
        let (rest, _) = ws(input)?;
        if let Ok((rest2, iri)) = base_decl(rest) {
            base = Some(iri);
            input = rest2;
        } else if let Ok((rest2, pd)) = prefix_decl(rest) {
            prefixes.push(pd);
            input = rest2;
        } else {
            return Ok((rest, (base, prefixes)));
        }
    }
}

fn base_decl(input: &str) -> IResult<&str, Iri> {
    let (input, _) = tag_no_case("BASE")(input)?;
    let (input, _) = ws1(input)?;
    iri_ref(input)
}

fn prefix_decl(input: &str) -> IResult<&str, PrefixDecl> {
    let (input, _) = tag_no_case("PREFIX")(input)?;
    let (input, _) = ws1(input)?;
    let (input, prefix) = pname_ns_prefix(input)?;
    let (input, _) = ws(input)?;
    let (input, iri) = iri_ref(input)?;
    Ok((input, PrefixDecl { prefix, iri }))
}

/// Parse the prefix part of PNAME_NS: `foo:` returns "foo", `:` returns "".
fn pname_ns_prefix(input: &str) -> IResult<&str, String> {
    let (input, prefix) = take_while(|c: char| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')(input)?;
    let (input, _) = char(':')(input)?;
    Ok((input, prefix.to_string()))
}

// ---------------------------------------------------------------------------
// SELECT clause
// ---------------------------------------------------------------------------

fn select_clause(input: &str) -> IResult<&str, (SelectClause, bool)> {
    let (input, _) = tag_no_case("SELECT")(input)?;
    let (input, _) = ws1(input)?;

    // Optional DISTINCT / REDUCED
    let (input, distinct) = opt(alt((
        value(true, terminated(tag_no_case("DISTINCT"), ws1)),
        value(false, terminated(tag_no_case("REDUCED"), ws1)),
    )))(input)?;
    let distinct = distinct.unwrap_or(false);

    // Star or variable list
    let (input, clause) = alt((
        value(SelectClause::Star, char('*')),
        map(separated_list1(ws1, variable), SelectClause::Variables),
    ))(input)?;

    Ok((input, (clause, distinct)))
}

// ---------------------------------------------------------------------------
// WHERE clause
// ---------------------------------------------------------------------------

fn where_clause(input: &str) -> IResult<&str, GroupGraphPattern> {
    let (input, _) = opt(pair(tag_no_case("WHERE"), ws))(input)?;
    group_graph_pattern(input)
}

fn group_graph_pattern(input: &str) -> IResult<&str, GroupGraphPattern> {
    let (input, _) = char('{')(input)?;
    let (input, _) = ws(input)?;
    let (input, patterns) = triples_block(input)?;
    let (input, _) = ws(input)?;
    let (input, _) = char('}')(input)?;
    Ok((input, GroupGraphPattern { patterns }))
}

// ---------------------------------------------------------------------------
// Triples block: TriplesSameSubject separated by '.'
// ---------------------------------------------------------------------------

fn triples_block(input: &str) -> IResult<&str, Vec<TriplePattern>> {
    let mut all_patterns = Vec::new();
    let mut input = input;

    // Parse first triple group
    if let Ok((rest, patterns)) = triples_same_subject(input) {
        all_patterns.extend(patterns);
        input = rest;

        // Parse subsequent '. TriplesSameSubject' groups
        loop {
            let (rest, _) = ws(input)?;
            if let Ok((rest2, _)) = char::<&str, nom::error::Error<&str>>('.')(rest) {
                let (rest2, _) = ws(rest2)?;
                // After a dot, there might be another triple group or the closing '}'
                if let Ok((rest3, patterns)) = triples_same_subject(rest2) {
                    all_patterns.extend(patterns);
                    input = rest3;
                } else {
                    // Trailing dot is fine
                    input = rest2;
                    break;
                }
            } else {
                break;
            }
        }
    }

    Ok((input, all_patterns))
}

/// Parse subject + property-list, desugaring ';' and ',' into flat triple patterns.
fn triples_same_subject(input: &str) -> IResult<&str, Vec<TriplePattern>> {
    let (input, subject) = term_or_variable(input)?;
    let (input, _) = ws1(input)?;
    let (input, pred_obj_pairs) = property_list_not_empty(input)?;

    let mut patterns = Vec::new();
    for (predicate, objects) in pred_obj_pairs {
        for object in objects {
            patterns.push(TriplePattern {
                subject: subject.clone(),
                predicate: predicate.clone(),
                object,
            });
        }
    }

    Ok((input, patterns))
}

/// Parse: Verb ObjectList (';' Verb ObjectList)*
/// Returns list of (predicate, objects) pairs.
fn property_list_not_empty(
    input: &str,
) -> IResult<&str, Vec<(TermOrVariable, Vec<TermOrVariable>)>> {
    let (input, first_pred) = verb(input)?;
    let (input, _) = ws1(input)?;
    let (input, first_objs) = object_list(input)?;

    let mut pairs = vec![(first_pred, first_objs)];
    let mut input = input;

    // Optional ';' separated additional predicate-object groups
    loop {
        let (rest, _) = ws(input)?;
        if let Ok((rest2, _)) = char::<&str, nom::error::Error<&str>>(';')(rest) {
            let (rest2, _) = ws(rest2)?;
            // After ';', there might be another verb or just end (trailing ';' is valid)
            if let Ok((rest3, pred)) = verb(rest2) {
                let (rest3, _) = ws1(rest3)?;
                let (rest3, objs) = object_list(rest3)?;
                pairs.push((pred, objs));
                input = rest3;
            } else {
                input = rest2;
                break;
            }
        } else {
            break;
        }
    }

    Ok((input, pairs))
}

/// Parse: Object (',' Object)*
fn object_list(input: &str) -> IResult<&str, Vec<TermOrVariable>> {
    let (input, first) = term_or_variable(input)?;
    let mut objects = vec![first];
    let mut input = input;

    loop {
        let (rest, _) = ws(input)?;
        if let Ok((rest2, _)) = char::<&str, nom::error::Error<&str>>(',')(rest) {
            let (rest2, _) = ws(rest2)?;
            let (rest2, obj) = term_or_variable(rest2)?;
            objects.push(obj);
            input = rest2;
        } else {
            break;
        }
    }

    Ok((input, objects))
}

/// Verb: VarOrIRIref | 'a'
fn verb(input: &str) -> IResult<&str, TermOrVariable> {
    alt((
        rdf_type_keyword,
        map(variable, TermOrVariable::Variable),
        map(iri_ref, TermOrVariable::Iri),
        prefixed_name,
    ))(input)
}

/// The 'a' keyword (shorthand for rdf:type).
/// Must be followed by whitespace or specific delimiters to avoid matching
/// prefixed names that start with 'a'.
fn rdf_type_keyword(input: &str) -> IResult<&str, TermOrVariable> {
    let (input, _) = char('a')(input)?;
    // 'a' must be followed by whitespace (not part of a longer token)
    if input.is_empty() || input.starts_with(|c: char| c.is_ascii_whitespace()) {
        Ok((input, TermOrVariable::RdfType))
    } else {
        Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )))
    }
}

// ---------------------------------------------------------------------------
// Solution modifiers: ORDER BY, LIMIT, OFFSET
// ---------------------------------------------------------------------------

fn solution_modifier(input: &str) -> IResult<&str, SolutionModifier> {
    let mut modifier = SolutionModifier::default();
    let mut input = input;

    // ORDER BY
    let (rest, _) = ws(input)?;
    if let Ok((rest2, conditions)) = order_clause(rest) {
        modifier.order_by = conditions;
        input = rest2;
    } else {
        input = rest;
    }

    // LIMIT and OFFSET can appear in either order
    loop {
        let (rest, _) = ws(input)?;
        if modifier.limit.is_none() {
            if let Ok((rest2, n)) = limit_clause(rest) {
                modifier.limit = Some(n);
                input = rest2;
                continue;
            }
        }
        if modifier.offset.is_none() {
            if let Ok((rest2, n)) = offset_clause(rest) {
                modifier.offset = Some(n);
                input = rest2;
                continue;
            }
        }
        input = rest;
        break;
    }

    Ok((input, modifier))
}

fn order_clause(input: &str) -> IResult<&str, Vec<OrderCondition>> {
    let (input, _) = tag_no_case("ORDER")(input)?;
    let (input, _) = ws1(input)?;
    let (input, _) = tag_no_case("BY")(input)?;
    let (input, _) = ws1(input)?;
    separated_list1(ws1, order_condition)(input)
}

fn order_condition(input: &str) -> IResult<&str, OrderCondition> {
    alt((
        // ASC(?var) or DESC(?var)
        map(
            pair(
                alt((
                    value(OrderDirection::Ascending, tag_no_case("ASC")),
                    value(OrderDirection::Descending, tag_no_case("DESC")),
                )),
                delimited(
                    tuple((ws, char('('), ws)),
                    variable,
                    tuple((ws, char(')'))),
                ),
            ),
            |(direction, var)| OrderCondition {
                variable: var,
                direction,
            },
        ),
        // Bare ?var (defaults to ascending)
        map(variable, |var| OrderCondition {
            variable: var,
            direction: OrderDirection::Ascending,
        }),
    ))(input)
}

fn limit_clause(input: &str) -> IResult<&str, usize> {
    let (input, _) = tag_no_case("LIMIT")(input)?;
    let (input, _) = ws1(input)?;
    map_res(digit1, |s: &str| s.parse::<usize>())(input)
}

fn offset_clause(input: &str) -> IResult<&str, usize> {
    let (input, _) = tag_no_case("OFFSET")(input)?;
    let (input, _) = ws1(input)?;
    map_res(digit1, |s: &str| s.parse::<usize>())(input)
}

// ---------------------------------------------------------------------------
// Terms and variables
// ---------------------------------------------------------------------------

fn term_or_variable(input: &str) -> IResult<&str, TermOrVariable> {
    alt((
        map(variable, TermOrVariable::Variable),
        rdf_type_keyword,
        map(iri_ref, TermOrVariable::Iri),
        prefixed_name,
        map(ast_literal, TermOrVariable::Literal),
    ))(input)
}

fn variable(input: &str) -> IResult<&str, Variable> {
    let (input, _) = alt((char('?'), char('$')))(input)?;
    let (input, name) = take_while1(|c: char| c.is_alphanumeric() || c == '_')(input)?;
    Ok((input, Variable(name.to_string())))
}

/// Parse `<...>` IRI reference.
fn iri_ref(input: &str) -> IResult<&str, Iri> {
    let (input, _) = char('<')(input)?;
    let (input, iri_str) = take_while(|c: char| c != '>')(input)?;
    let (input, _) = char('>')(input)?;
    Ok((input, Iri::new(iri_str)))
}

/// Parse prefixed name like `foaf:name` or `ex:` or `:localName`.
fn prefixed_name(input: &str) -> IResult<&str, TermOrVariable> {
    let (input, prefix) = take_while(|c: char| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')(input)?;
    let (input, _) = char(':')(input)?;
    let (input, local) = take_while(|c: char| {
        c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
    })(input)?;

    Ok((
        input,
        TermOrVariable::PrefixedName {
            prefix: prefix.to_string(),
            local: local.to_string(),
        },
    ))
}

// ---------------------------------------------------------------------------
// Literals
// ---------------------------------------------------------------------------

fn ast_literal(input: &str) -> IResult<&str, AstLiteral> {
    alt((boolean_literal, string_literal, integer_literal))(input)
}

fn boolean_literal(input: &str) -> IResult<&str, AstLiteral> {
    alt((
        value(AstLiteral::Boolean(true), tag("true")),
        value(AstLiteral::Boolean(false), tag("false")),
    ))(input)
}

fn string_literal(input: &str) -> IResult<&str, AstLiteral> {
    let (input, s) = alt((
        string_literal_long2,
        string_literal2,
        string_literal_long1,
        string_literal1,
    ))(input)?;
    Ok((input, AstLiteral::String(s)))
}

/// Double-quoted string: "..."
fn string_literal2(input: &str) -> IResult<&str, String> {
    let (input, _) = char('"')(input)?;
    let mut result = String::new();
    let mut chars = input.chars();
    let mut consumed = 0;

    loop {
        match chars.next() {
            Some('"') => {
                consumed += 1;
                return Ok((&input[consumed..], result));
            }
            Some('\\') => {
                consumed += 1;
                match chars.next() {
                    Some(c @ ('t' | 'n' | 'r' | '\\' | '"' | '\'')) => {
                        consumed += c.len_utf8();
                        result.push(match c {
                            't' => '\t',
                            'n' => '\n',
                            'r' => '\r',
                            _ => c,
                        });
                    }
                    _ => {
                        return Err(nom::Err::Error(nom::error::Error::new(
                            input,
                            nom::error::ErrorKind::Char,
                        )));
                    }
                }
            }
            Some(c) if c != '\n' && c != '\r' => {
                consumed += c.len_utf8();
                result.push(c);
            }
            _ => {
                return Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Char,
                )));
            }
        }
    }
}

/// Single-quoted string: '...'
fn string_literal1(input: &str) -> IResult<&str, String> {
    let (input, _) = char('\'')(input)?;
    let mut result = String::new();
    let mut chars = input.chars();
    let mut consumed = 0;

    loop {
        match chars.next() {
            Some('\'') => {
                consumed += 1;
                return Ok((&input[consumed..], result));
            }
            Some('\\') => {
                consumed += 1;
                match chars.next() {
                    Some(c @ ('t' | 'n' | 'r' | '\\' | '"' | '\'')) => {
                        consumed += c.len_utf8();
                        result.push(match c {
                            't' => '\t',
                            'n' => '\n',
                            'r' => '\r',
                            _ => c,
                        });
                    }
                    _ => {
                        return Err(nom::Err::Error(nom::error::Error::new(
                            input,
                            nom::error::ErrorKind::Char,
                        )));
                    }
                }
            }
            Some(c) if c != '\n' && c != '\r' => {
                consumed += c.len_utf8();
                result.push(c);
            }
            _ => {
                return Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Char,
                )));
            }
        }
    }
}

/// Long double-quoted string: """..."""
fn string_literal_long2(input: &str) -> IResult<&str, String> {
    let (input, _) = tag("\"\"\"")(input)?;
    let mut result = String::new();
    let mut i = 0;
    let bytes = input.as_bytes();

    while i < bytes.len() {
        if i + 2 < bytes.len() && bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"'
        {
            return Ok((&input[i + 3..], result));
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let c = bytes[i + 1] as char;
            match c {
                't' => { result.push('\t'); i += 2; }
                'n' => { result.push('\n'); i += 2; }
                'r' => { result.push('\r'); i += 2; }
                '\\' | '"' | '\'' => { result.push(c); i += 2; }
                _ => {
                    return Err(nom::Err::Error(nom::error::Error::new(
                        input,
                        nom::error::ErrorKind::Char,
                    )));
                }
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input,
        nom::error::ErrorKind::Tag,
    )))
}

/// Long single-quoted string: '''...'''
fn string_literal_long1(input: &str) -> IResult<&str, String> {
    let (input, _) = tag("'''")(input)?;
    let mut result = String::new();
    let mut i = 0;
    let bytes = input.as_bytes();

    while i < bytes.len() {
        if i + 2 < bytes.len()
            && bytes[i] == b'\''
            && bytes[i + 1] == b'\''
            && bytes[i + 2] == b'\''
        {
            return Ok((&input[i + 3..], result));
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let c = bytes[i + 1] as char;
            match c {
                't' => { result.push('\t'); i += 2; }
                'n' => { result.push('\n'); i += 2; }
                'r' => { result.push('\r'); i += 2; }
                '\\' | '"' | '\'' => { result.push(c); i += 2; }
                _ => {
                    return Err(nom::Err::Error(nom::error::Error::new(
                        input,
                        nom::error::ErrorKind::Char,
                    )));
                }
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input,
        nom::error::ErrorKind::Tag,
    )))
}

fn integer_literal(input: &str) -> IResult<&str, AstLiteral> {
    let (input, neg) = opt(char('-'))(input)?;
    let (input, digits) = digit1(input)?;
    let n: i64 = digits.parse().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Digit))
    })?;
    Ok((input, AstLiteral::Integer(if neg.is_some() { -n } else { n })))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_select_star() {
        let q = parse("SELECT * WHERE { ?s ?p ?o }").unwrap();
        assert!(matches!(q.select, SelectClause::Star));
        assert_eq!(q.where_pattern.patterns.len(), 1);
    }

    #[test]
    fn test_select_variables() {
        let q = parse("SELECT ?name ?age WHERE { ?s ?p ?o }").unwrap();
        match &q.select {
            SelectClause::Variables(vars) => {
                assert_eq!(vars.len(), 2);
                assert_eq!(vars[0].0, "name");
                assert_eq!(vars[1].0, "age");
            }
            _ => panic!("expected Variables"),
        }
    }

    #[test]
    fn test_prefix_and_prefixed_names() {
        let q = parse(
            "PREFIX ex: <http://example.org/> SELECT * WHERE { ?s ex:name ?o }",
        )
        .unwrap();
        assert_eq!(q.prefixes.len(), 1);
        assert_eq!(q.prefixes[0].prefix, "ex");
        assert_eq!(q.prefixes[0].iri.0, "http://example.org/");

        let pred = &q.where_pattern.patterns[0].predicate;
        match pred {
            TermOrVariable::PrefixedName { prefix, local } => {
                assert_eq!(prefix, "ex");
                assert_eq!(local, "name");
            }
            _ => panic!("expected PrefixedName"),
        }
    }

    #[test]
    fn test_rdf_type_keyword() {
        let q = parse("SELECT * WHERE { ?s a ?type }").unwrap();
        assert!(matches!(
            q.where_pattern.patterns[0].predicate,
            TermOrVariable::RdfType
        ));
    }

    #[test]
    fn test_multiple_triple_patterns() {
        let q = parse(
            "SELECT * WHERE { ?s a ?type . ?s <http://example.org/name> ?name }",
        )
        .unwrap();
        assert_eq!(q.where_pattern.patterns.len(), 2);
    }

    #[test]
    fn test_semicolon_shorthand() {
        // Same subject, two predicates
        let q = parse(
            "SELECT * WHERE { ?s a ?type ; <http://example.org/name> ?name }",
        )
        .unwrap();
        assert_eq!(q.where_pattern.patterns.len(), 2);
        // Both should have the same subject
        let s1 = &q.where_pattern.patterns[0].subject;
        let s2 = &q.where_pattern.patterns[1].subject;
        assert!(matches!(s1, TermOrVariable::Variable(Variable(v)) if v == "s"));
        assert!(matches!(s2, TermOrVariable::Variable(Variable(v)) if v == "s"));
    }

    #[test]
    fn test_comma_shorthand() {
        // Same subject+predicate, two objects
        let q = parse(
            "SELECT * WHERE { ?s <http://example.org/knows> ?a , ?b }",
        )
        .unwrap();
        assert_eq!(q.where_pattern.patterns.len(), 2);
    }

    #[test]
    fn test_solution_modifiers() {
        let q = parse(
            "SELECT * WHERE { ?s ?p ?o } ORDER BY ?s DESC(?o) LIMIT 10 OFFSET 5",
        )
        .unwrap();
        assert_eq!(q.modifier.order_by.len(), 2);
        assert!(matches!(
            q.modifier.order_by[0].direction,
            OrderDirection::Ascending
        ));
        assert!(matches!(
            q.modifier.order_by[1].direction,
            OrderDirection::Descending
        ));
        assert_eq!(q.modifier.limit, Some(10));
        assert_eq!(q.modifier.offset, Some(5));
    }

    #[test]
    fn test_distinct() {
        let q = parse("SELECT DISTINCT ?s WHERE { ?s ?p ?o }").unwrap();
        assert!(q.modifier.distinct);
    }

    #[test]
    fn test_string_literal() {
        let q = parse(r#"SELECT * WHERE { ?s ?p "hello" }"#).unwrap();
        match &q.where_pattern.patterns[0].object {
            TermOrVariable::Literal(AstLiteral::String(s)) => assert_eq!(s, "hello"),
            other => panic!("expected string literal, got {:?}", other),
        }
    }

    #[test]
    fn test_integer_literal() {
        let q = parse("SELECT * WHERE { ?s ?p 42 }").unwrap();
        match &q.where_pattern.patterns[0].object {
            TermOrVariable::Literal(AstLiteral::Integer(n)) => assert_eq!(*n, 42),
            other => panic!("expected integer literal, got {:?}", other),
        }
    }

    #[test]
    fn test_boolean_literal() {
        let q = parse("SELECT * WHERE { ?s ?p true }").unwrap();
        match &q.where_pattern.patterns[0].object {
            TermOrVariable::Literal(AstLiteral::Boolean(b)) => assert!(*b),
            other => panic!("expected boolean literal, got {:?}", other),
        }
    }

    #[test]
    fn test_trailing_dot() {
        // Trailing dot should be fine
        let q = parse("SELECT * WHERE { ?s ?p ?o . }").unwrap();
        assert_eq!(q.where_pattern.patterns.len(), 1);
    }

    #[test]
    fn test_optional_where_keyword() {
        let q = parse("SELECT * { ?s ?p ?o }").unwrap();
        assert_eq!(q.where_pattern.patterns.len(), 1);
    }

    #[test]
    fn test_full_query() {
        let query = r#"
            PREFIX ex: <http://example.org/>

            SELECT ?name ?age
            WHERE {
                ?person a ex:Person .
                ?person ex:name ?name .
                ?person ex:age ?age .
            }
            ORDER BY ?name
            LIMIT 10
        "#;
        let q = parse(query).unwrap();
        assert_eq!(q.prefixes.len(), 1);
        assert_eq!(q.where_pattern.patterns.len(), 3);
        assert_eq!(q.modifier.order_by.len(), 1);
        assert_eq!(q.modifier.limit, Some(10));
        match &q.select {
            SelectClause::Variables(vars) => {
                assert_eq!(vars.len(), 2);
                assert_eq!(vars[0].0, "name");
                assert_eq!(vars[1].0, "age");
            }
            _ => panic!("expected Variables"),
        }
    }
}
