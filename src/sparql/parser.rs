use crate::error::DarqError;
use crate::rdf::Iri;
use super::ast::*;

/// Parser state: wraps the input string with a cursor position.
struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Parser { input, pos: 0 }
    }

    fn remaining(&self) -> &'a str {
        &self.input[self.pos..]
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.input.len()
    }

    /// Peek at the next character without consuming it.
    fn peek(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    /// Advance past one character.
    fn advance(&mut self, n: usize) {
        self.pos += n;
    }

    /// Skip ASCII whitespace.
    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.advance(c.len_utf8());
            } else {
                break;
            }
        }
    }

    /// Expect and consume at least one whitespace character.
    fn expect_ws(&mut self) -> Result<(), DarqError> {
        if self.peek().map_or(false, |c| c.is_ascii_whitespace()) {
            self.skip_ws();
            Ok(())
        } else {
            Err(self.error("expected whitespace"))
        }
    }

    /// Expect and consume a specific character.
    fn expect_char(&mut self, expected: char) -> Result<(), DarqError> {
        match self.peek() {
            Some(c) if c == expected => {
                self.advance(c.len_utf8());
                Ok(())
            }
            _ => Err(self.error(&format!("expected '{}'", expected))),
        }
    }

    /// Try to consume a specific character. Returns true if consumed.
    fn try_char(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance(expected.len_utf8());
            true
        } else {
            false
        }
    }

    /// Try to consume a case-insensitive keyword followed by a word boundary.
    /// Returns true if consumed.
    fn try_keyword(&mut self, keyword: &str) -> bool {
        let rem = self.remaining();
        if rem.len() < keyword.len() {
            return false;
        }
        if !rem[..keyword.len()].eq_ignore_ascii_case(keyword) {
            return false;
        }
        // Must be at a word boundary: end of input or followed by non-alphanumeric
        let after = &rem[keyword.len()..];
        if let Some(c) = after.chars().next() {
            if c.is_alphanumeric() || c == '_' {
                return false;
            }
        }
        self.advance(keyword.len());
        true
    }

    /// Expect a case-insensitive keyword (error if not found).
    fn expect_keyword(&mut self, keyword: &str) -> Result<(), DarqError> {
        if self.try_keyword(keyword) {
            Ok(())
        } else {
            Err(self.error(&format!("expected '{}'", keyword)))
        }
    }

    /// Consume characters while the predicate holds. Returns the consumed slice.
    fn take_while(&mut self, pred: impl Fn(char) -> bool) -> &'a str {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if pred(c) {
                self.advance(c.len_utf8());
            } else {
                break;
            }
        }
        &self.input[start..self.pos]
    }

    /// Save the current position for backtracking.
    fn save(&self) -> usize {
        self.pos
    }

    /// Restore a previously saved position.
    fn restore(&mut self, pos: usize) {
        self.pos = pos;
    }

    /// Create an error at the current position.
    fn error(&self, msg: &str) -> DarqError {
        let remaining = self.remaining();
        let context = if remaining.len() > 40 {
            &remaining[..40]
        } else {
            remaining
        };
        DarqError::ParseError(format!("{} at: {:?}", msg, context))
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn parse(input: &str) -> Result<SelectQuery, DarqError> {
    let mut p = Parser::new(input.trim());
    let query = parse_select_query(&mut p)?;
    p.skip_ws();
    if !p.is_empty() {
        return Err(p.error("unexpected trailing input"));
    }
    Ok(query)
}

// ---------------------------------------------------------------------------
// Top-level: SELECT query
// ---------------------------------------------------------------------------

fn parse_select_query(p: &mut Parser) -> Result<SelectQuery, DarqError> {
    let (base, prefixes) = parse_prologue(p)?;
    p.skip_ws();
    let (select, distinct) = parse_select_clause(p)?;
    p.skip_ws();
    let where_pattern = parse_where_clause(p)?;
    p.skip_ws();
    let mut modifier = parse_solution_modifier(p)?;
    modifier.distinct = distinct;
    p.skip_ws();
    let values = parse_values_clause(p)?;

    Ok(SelectQuery {
        prefixes,
        base,
        select,
        where_pattern,
        modifier,
        values,
    })
}

// ---------------------------------------------------------------------------
// Prologue: BASE and PREFIX declarations
// ---------------------------------------------------------------------------

fn parse_prologue(p: &mut Parser) -> Result<(Option<Iri>, Vec<PrefixDecl>), DarqError> {
    let mut base = None;
    let mut prefixes = Vec::new();

    loop {
        p.skip_ws();
        let saved = p.save();
        if p.try_keyword("BASE") {
            p.expect_ws()?;
            base = Some(parse_iri_ref(p)?);
        } else if p.try_keyword("PREFIX") {
            p.expect_ws()?;
            let prefix = parse_pname_ns_prefix(p)?;
            p.skip_ws();
            let iri = parse_iri_ref(p)?;
            prefixes.push(PrefixDecl { prefix, iri });
        } else {
            p.restore(saved);
            break;
        }
    }

    Ok((base, prefixes))
}

/// Parse the prefix part of PNAME_NS: `foo:` returns "foo", `:` returns "".
fn parse_pname_ns_prefix(p: &mut Parser) -> Result<String, DarqError> {
    let prefix = p.take_while(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.');
    let prefix = prefix.to_string();
    p.expect_char(':')?;
    Ok(prefix)
}

// ---------------------------------------------------------------------------
// SELECT clause
// ---------------------------------------------------------------------------

fn parse_select_clause(p: &mut Parser) -> Result<(SelectClause, bool), DarqError> {
    p.expect_keyword("SELECT")?;
    p.expect_ws()?;

    // Optional DISTINCT / REDUCED
    let distinct = if p.try_keyword("DISTINCT") {
        p.expect_ws()?;
        true
    } else if p.try_keyword("REDUCED") {
        p.expect_ws()?;
        false
    } else {
        false
    };

    // Star or variable list
    let clause = if p.try_char('*') {
        SelectClause::Star
    } else {
        let mut vars = vec![parse_variable(p)?];
        loop {
            let saved = p.save();
            p.skip_ws();
            match parse_variable(p) {
                Ok(v) => vars.push(v),
                Err(_) => {
                    p.restore(saved);
                    break;
                }
            }
        }
        SelectClause::Variables(vars)
    };

    Ok((clause, distinct))
}

// ---------------------------------------------------------------------------
// WHERE clause
// ---------------------------------------------------------------------------

fn parse_where_clause(p: &mut Parser) -> Result<GroupGraphPattern, DarqError> {
    // WHERE keyword is optional
    let saved = p.save();
    if p.try_keyword("WHERE") {
        p.skip_ws();
    } else {
        p.restore(saved);
    }
    parse_group_graph_pattern(p)
}

fn parse_group_graph_pattern(p: &mut Parser) -> Result<GroupGraphPattern, DarqError> {
    p.expect_char('{')?;
    p.skip_ws();
    let patterns = parse_triples_block(p)?;
    p.skip_ws();
    p.expect_char('}')?;
    Ok(GroupGraphPattern { patterns })
}

// ---------------------------------------------------------------------------
// Triples block
// ---------------------------------------------------------------------------

fn parse_triples_block(p: &mut Parser) -> Result<Vec<TriplePattern>, DarqError> {
    let mut all_patterns = Vec::new();

    // Try to parse the first triple group
    let saved = p.save();
    match parse_triples_same_subject(p) {
        Ok(patterns) => {
            all_patterns.extend(patterns);
        }
        Err(_) => {
            p.restore(saved);
            return Ok(all_patterns);
        }
    }

    // Parse subsequent '. TriplesSameSubject' groups
    loop {
        p.skip_ws();
        if !p.try_char('.') {
            break;
        }
        p.skip_ws();
        // After a dot, there might be another triple group or the closing '}'
        let saved = p.save();
        match parse_triples_same_subject(p) {
            Ok(patterns) => all_patterns.extend(patterns),
            Err(_) => {
                // Trailing dot is fine
                p.restore(saved);
                break;
            }
        }
    }

    Ok(all_patterns)
}

/// Parse subject + property-list, desugaring ';' and ',' into flat triple patterns.
fn parse_triples_same_subject(p: &mut Parser) -> Result<Vec<TriplePattern>, DarqError> {
    let subject = parse_term_or_variable(p)?;
    p.expect_ws()?;
    let pred_obj_pairs = parse_property_list_not_empty(p)?;

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

    Ok(patterns)
}

/// Parse: Verb ObjectList (';' Verb ObjectList)*
fn parse_property_list_not_empty(
    p: &mut Parser,
) -> Result<Vec<(TermOrVariable, Vec<TermOrVariable>)>, DarqError> {
    let first_pred = parse_verb(p)?;
    p.expect_ws()?;
    let first_objs = parse_object_list(p)?;

    let mut pairs = vec![(first_pred, first_objs)];

    loop {
        let saved = p.save();
        p.skip_ws();
        if !p.try_char(';') {
            p.restore(saved);
            break;
        }
        p.skip_ws();
        // After ';', there might be another verb or just end (trailing ';' is valid)
        match parse_verb(p) {
            Ok(pred) => {
                p.expect_ws()?;
                let objs = parse_object_list(p)?;
                pairs.push((pred, objs));
            }
            Err(_) => break,
        }
    }

    Ok(pairs)
}

/// Parse: Object (',' Object)*
fn parse_object_list(p: &mut Parser) -> Result<Vec<TermOrVariable>, DarqError> {
    let mut objects = vec![parse_term_or_variable(p)?];

    loop {
        let saved = p.save();
        p.skip_ws();
        if !p.try_char(',') {
            p.restore(saved);
            break;
        }
        p.skip_ws();
        objects.push(parse_term_or_variable(p)?);
    }

    Ok(objects)
}

/// Verb: VarOrIRIref | 'a'
fn parse_verb(p: &mut Parser) -> Result<TermOrVariable, DarqError> {
    // Try 'a' keyword first (must be followed by whitespace, not part of a prefixed name)
    if let Some(tov) = try_rdf_type_keyword(p) {
        return Ok(tov);
    }
    // Try variable
    if let Some(c) = p.peek() {
        if c == '?' || c == '$' {
            return Ok(TermOrVariable::Variable(parse_variable(p)?));
        }
    }
    // Try IRI ref
    if p.peek() == Some('<') {
        return Ok(TermOrVariable::Iri(parse_iri_ref(p)?));
    }
    // Try prefixed name
    parse_prefixed_name(p)
}

/// Try to parse the 'a' keyword (rdf:type shorthand).
fn try_rdf_type_keyword(p: &mut Parser) -> Option<TermOrVariable> {
    let saved = p.save();
    if p.peek() != Some('a') {
        return None;
    }
    p.advance(1);
    // 'a' must be followed by whitespace or end of input (not part of a longer token)
    match p.peek() {
        None => Some(TermOrVariable::RdfType),
        Some(c) if c.is_ascii_whitespace() => Some(TermOrVariable::RdfType),
        _ => {
            p.restore(saved);
            None
        }
    }
}

// ---------------------------------------------------------------------------
// VALUES clause
// ---------------------------------------------------------------------------

fn parse_values_clause(p: &mut Parser) -> Result<Option<ValuesClause>, DarqError> {
    let saved = p.save();
    if p.try_keyword("VALUES") {
        p.skip_ws();
        Ok(Some(parse_data_block(p)?))
    } else {
        p.restore(saved);
        Ok(None)
    }
}

fn parse_data_block(p: &mut Parser) -> Result<ValuesClause, DarqError> {
    // Single-variable form: VALUES ?x { ... }
    // Multi-variable form:  VALUES (?x ?y) { ... }
    if p.peek() == Some('(') {
        parse_inline_data_full(p)
    } else {
        parse_inline_data_one_var(p)
    }
}

/// Parse `Var '{' DataBlockValue* '}'`
fn parse_inline_data_one_var(p: &mut Parser) -> Result<ValuesClause, DarqError> {
    let var = parse_variable(p)?;
    p.skip_ws();
    p.expect_char('{')?;

    let mut bindings = Vec::new();
    loop {
        p.skip_ws();
        if p.try_char('}') {
            break;
        }
        let val = parse_data_block_value(p)?;
        bindings.push(vec![val]);
    }

    Ok(ValuesClause {
        variables: vec![var],
        bindings,
    })
}

/// Parse `'(' Var* ')' '{' ( '(' DataBlockValue* ')' )* '}'`
fn parse_inline_data_full(p: &mut Parser) -> Result<ValuesClause, DarqError> {
    p.expect_char('(')?;
    let mut variables = Vec::new();
    loop {
        p.skip_ws();
        if p.try_char(')') {
            break;
        }
        variables.push(parse_variable(p)?);
    }

    p.skip_ws();
    p.expect_char('{')?;

    let num_vars = variables.len();
    let mut bindings = Vec::new();
    loop {
        p.skip_ws();
        if p.try_char('}') {
            break;
        }
        p.expect_char('(')?;
        let mut row = Vec::new();
        for _ in 0..num_vars {
            p.skip_ws();
            row.push(parse_data_block_value(p)?);
        }
        p.skip_ws();
        p.expect_char(')')?;
        bindings.push(row);
    }

    Ok(ValuesClause {
        variables,
        bindings,
    })
}

/// Parse a single data block value: IRI, prefixed name, literal, or UNDEF.
fn parse_data_block_value(p: &mut Parser) -> Result<DataBlockValue, DarqError> {
    // UNDEF keyword
    let saved = p.save();
    if p.try_keyword("UNDEF") {
        return Ok(DataBlockValue::Undef);
    }
    p.restore(saved);

    // IRI ref: <...>
    if p.peek() == Some('<') {
        return Ok(DataBlockValue::Iri(parse_iri_ref(p)?));
    }

    // String literal: "..." or '...'
    if let Some(c) = p.peek() {
        if c == '"' || c == '\'' {
            return Ok(DataBlockValue::Literal(parse_string_literal(p)?));
        }
    }

    // Boolean literal: true/false
    let saved = p.save();
    if p.try_keyword("true") {
        return Ok(DataBlockValue::Literal(AstLiteral::Boolean(true)));
    }
    p.restore(saved);
    if p.try_keyword("false") {
        return Ok(DataBlockValue::Literal(AstLiteral::Boolean(false)));
    }
    p.restore(saved);

    // Integer literal (possibly negative)
    if let Some(c) = p.peek() {
        if c.is_ascii_digit() || c == '-' {
            return Ok(DataBlockValue::Literal(parse_integer_literal(p)?));
        }
    }

    // Prefixed name: prefix:local
    let pn = parse_prefixed_name(p)?;
    match pn {
        TermOrVariable::PrefixedName { prefix, local } => {
            Ok(DataBlockValue::PrefixedName { prefix, local })
        }
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// Solution modifiers
// ---------------------------------------------------------------------------

fn parse_solution_modifier(p: &mut Parser) -> Result<SolutionModifier, DarqError> {
    let mut modifier = SolutionModifier::default();

    // ORDER BY
    p.skip_ws();
    let saved = p.save();
    if p.try_keyword("ORDER") {
        p.expect_ws()?;
        p.expect_keyword("BY")?;
        p.expect_ws()?;
        modifier.order_by = parse_order_conditions(p)?;
    } else {
        p.restore(saved);
    }

    // LIMIT and OFFSET can appear in either order
    for _ in 0..2 {
        p.skip_ws();
        let saved = p.save();
        if modifier.limit.is_none() && p.try_keyword("LIMIT") {
            p.expect_ws()?;
            modifier.limit = Some(parse_usize(p)?);
        } else {
            p.restore(saved);
            let saved = p.save();
            if modifier.offset.is_none() && p.try_keyword("OFFSET") {
                p.expect_ws()?;
                modifier.offset = Some(parse_usize(p)?);
            } else {
                p.restore(saved);
            }
        }
    }

    Ok(modifier)
}

fn parse_order_conditions(p: &mut Parser) -> Result<Vec<OrderCondition>, DarqError> {
    let mut conditions = vec![parse_order_condition(p)?];
    loop {
        let saved = p.save();
        p.skip_ws();
        match parse_order_condition(p) {
            Ok(cond) => conditions.push(cond),
            Err(_) => {
                p.restore(saved);
                break;
            }
        }
    }
    Ok(conditions)
}

fn parse_order_condition(p: &mut Parser) -> Result<OrderCondition, DarqError> {
    // ASC(?var) or DESC(?var)
    let saved = p.save();
    if p.try_keyword("ASC") {
        p.skip_ws();
        p.expect_char('(')?;
        p.skip_ws();
        let var = parse_variable(p)?;
        p.skip_ws();
        p.expect_char(')')?;
        return Ok(OrderCondition {
            variable: var,
            direction: OrderDirection::Ascending,
        });
    }
    p.restore(saved);

    let saved = p.save();
    if p.try_keyword("DESC") {
        p.skip_ws();
        p.expect_char('(')?;
        p.skip_ws();
        let var = parse_variable(p)?;
        p.skip_ws();
        p.expect_char(')')?;
        return Ok(OrderCondition {
            variable: var,
            direction: OrderDirection::Descending,
        });
    }
    p.restore(saved);

    // Bare ?var (defaults to ascending)
    let var = parse_variable(p)?;
    Ok(OrderCondition {
        variable: var,
        direction: OrderDirection::Ascending,
    })
}

fn parse_usize(p: &mut Parser) -> Result<usize, DarqError> {
    let digits = p.take_while(|c| c.is_ascii_digit());
    if digits.is_empty() {
        return Err(p.error("expected integer"));
    }
    digits
        .parse()
        .map_err(|_| p.error("invalid integer"))
}

// ---------------------------------------------------------------------------
// Terms and variables
// ---------------------------------------------------------------------------

fn parse_term_or_variable(p: &mut Parser) -> Result<TermOrVariable, DarqError> {
    // Variable: ?x or $x
    if let Some(c) = p.peek() {
        if c == '?' || c == '$' {
            return Ok(TermOrVariable::Variable(parse_variable(p)?));
        }
    }
    // 'a' keyword
    if let Some(tov) = try_rdf_type_keyword(p) {
        return Ok(tov);
    }
    // IRI ref: <...>
    if p.peek() == Some('<') {
        return Ok(TermOrVariable::Iri(parse_iri_ref(p)?));
    }
    // String literal: "..." or '...'
    if let Some(c) = p.peek() {
        if c == '"' || c == '\'' {
            return Ok(TermOrVariable::Literal(parse_string_literal(p)?));
        }
    }
    // Boolean literal: true/false
    let saved = p.save();
    if p.try_keyword("true") {
        return Ok(TermOrVariable::Literal(AstLiteral::Boolean(true)));
    }
    p.restore(saved);
    if p.try_keyword("false") {
        return Ok(TermOrVariable::Literal(AstLiteral::Boolean(false)));
    }
    p.restore(saved);
    // Integer literal (possibly negative)
    if let Some(c) = p.peek() {
        if c.is_ascii_digit() || c == '-' {
            return Ok(TermOrVariable::Literal(parse_integer_literal(p)?));
        }
    }
    // Prefixed name: prefix:local
    parse_prefixed_name(p)
}

fn parse_variable(p: &mut Parser) -> Result<Variable, DarqError> {
    let c = p.peek().ok_or_else(|| p.error("expected variable"))?;
    if c != '?' && c != '$' {
        return Err(p.error("expected '?' or '$'"));
    }
    p.advance(1);
    let name = p.take_while(|c| c.is_alphanumeric() || c == '_');
    if name.is_empty() {
        return Err(p.error("expected variable name"));
    }
    Ok(Variable(name.to_string()))
}

/// Parse `<...>` IRI reference.
fn parse_iri_ref(p: &mut Parser) -> Result<Iri, DarqError> {
    p.expect_char('<')?;
    let iri_str = p.take_while(|c| c != '>');
    p.expect_char('>')?;
    Ok(Iri::new(iri_str))
}

/// Parse prefixed name like `foaf:name` or `ex:` or `:localName`.
fn parse_prefixed_name(p: &mut Parser) -> Result<TermOrVariable, DarqError> {
    let prefix = p.take_while(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.');
    let prefix = prefix.to_string();
    p.expect_char(':')?;
    let local = p.take_while(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.');
    let local = local.to_string();
    Ok(TermOrVariable::PrefixedName { prefix, local })
}

// ---------------------------------------------------------------------------
// Literals
// ---------------------------------------------------------------------------

fn parse_string_literal(p: &mut Parser) -> Result<AstLiteral, DarqError> {
    let quote = p.peek().ok_or_else(|| p.error("expected string literal"))?;

    // Check for long strings (""" or ''')
    let rem = p.remaining();
    let long_delim = if quote == '"' { "\"\"\"" } else { "'''" };
    if rem.starts_with(long_delim) {
        let s = parse_long_string(p, quote)?;
        return Ok(AstLiteral::String(s));
    }

    // Short string
    p.advance(1); // consume opening quote
    let mut result = String::new();
    loop {
        let c = p.peek().ok_or_else(|| p.error("unterminated string"))?;
        if c == quote {
            p.advance(1);
            return Ok(AstLiteral::String(result));
        }
        if c == '\\' {
            p.advance(1);
            let esc = p.peek().ok_or_else(|| p.error("unterminated escape"))?;
            p.advance(esc.len_utf8());
            result.push(match esc {
                't' => '\t',
                'n' => '\n',
                'r' => '\r',
                '\\' | '"' | '\'' => esc,
                _ => return Err(p.error(&format!("invalid escape: \\{}", esc))),
            });
        } else if c == '\n' || c == '\r' {
            return Err(p.error("newline in short string"));
        } else {
            p.advance(c.len_utf8());
            result.push(c);
        }
    }
}

fn parse_long_string(p: &mut Parser, quote: char) -> Result<String, DarqError> {
    // Skip opening triple-quote
    p.advance(3);
    let close = if quote == '"' { "\"\"\"" } else { "'''" };

    let mut result = String::new();
    loop {
        if p.remaining().starts_with(close) {
            p.advance(3);
            return Ok(result);
        }
        let c = p.peek().ok_or_else(|| p.error("unterminated long string"))?;
        if c == '\\' {
            p.advance(1);
            let esc = p.peek().ok_or_else(|| p.error("unterminated escape"))?;
            p.advance(esc.len_utf8());
            result.push(match esc {
                't' => '\t',
                'n' => '\n',
                'r' => '\r',
                '\\' | '"' | '\'' => esc,
                _ => return Err(p.error(&format!("invalid escape: \\{}", esc))),
            });
        } else {
            p.advance(c.len_utf8());
            result.push(c);
        }
    }
}

fn parse_integer_literal(p: &mut Parser) -> Result<AstLiteral, DarqError> {
    let neg = p.try_char('-');
    let digits = p.take_while(|c| c.is_ascii_digit());
    if digits.is_empty() {
        return Err(p.error("expected digits"));
    }
    let n: i64 = digits
        .parse()
        .map_err(|_| p.error("invalid integer"))?;
    Ok(AstLiteral::Integer(if neg { -n } else { n }))
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
        let q = parse(
            "SELECT * WHERE { ?s a ?type ; <http://example.org/name> ?name }",
        )
        .unwrap();
        assert_eq!(q.where_pattern.patterns.len(), 2);
        let s1 = &q.where_pattern.patterns[0].subject;
        let s2 = &q.where_pattern.patterns[1].subject;
        assert!(matches!(s1, TermOrVariable::Variable(Variable(v)) if v == "s"));
        assert!(matches!(s2, TermOrVariable::Variable(Variable(v)) if v == "s"));
    }

    #[test]
    fn test_comma_shorthand() {
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

    #[test]
    fn test_no_values_clause() {
        let q = parse("SELECT * WHERE { ?s ?p ?o }").unwrap();
        assert!(q.values.is_none());
    }

    #[test]
    fn test_values_single_var() {
        let q = parse("SELECT ?x WHERE { ?x ?p ?o } VALUES ?x { 1 2 3 }").unwrap();
        let vc = q.values.as_ref().unwrap();
        assert_eq!(vc.variables.len(), 1);
        assert_eq!(vc.variables[0].0, "x");
        assert_eq!(vc.bindings.len(), 3);
        assert!(matches!(&vc.bindings[0][0], DataBlockValue::Literal(AstLiteral::Integer(1))));
        assert!(matches!(&vc.bindings[1][0], DataBlockValue::Literal(AstLiteral::Integer(2))));
        assert!(matches!(&vc.bindings[2][0], DataBlockValue::Literal(AstLiteral::Integer(3))));
    }

    #[test]
    fn test_values_multi_var() {
        let q = parse(r#"SELECT * WHERE { ?s ?p ?o } VALUES (?x ?y) { (1 "a") (2 "b") }"#).unwrap();
        let vc = q.values.as_ref().unwrap();
        assert_eq!(vc.variables.len(), 2);
        assert_eq!(vc.variables[0].0, "x");
        assert_eq!(vc.variables[1].0, "y");
        assert_eq!(vc.bindings.len(), 2);
    }

    #[test]
    fn test_values_undef() {
        let q = parse(r#"SELECT * WHERE { ?s ?p ?o } VALUES (?x ?y) { (1 UNDEF) }"#).unwrap();
        let vc = q.values.as_ref().unwrap();
        assert!(matches!(&vc.bindings[0][0], DataBlockValue::Literal(AstLiteral::Integer(1))));
        assert!(matches!(&vc.bindings[0][1], DataBlockValue::Undef));
    }

    #[test]
    fn test_values_with_iri() {
        let q = parse("SELECT * WHERE { ?s ?p ?o } VALUES ?x { <http://example.org/a> }").unwrap();
        let vc = q.values.as_ref().unwrap();
        assert!(matches!(&vc.bindings[0][0], DataBlockValue::Iri(iri) if iri.0 == "http://example.org/a"));
    }

    #[test]
    fn test_values_with_prefixed_name() {
        let q = parse("PREFIX ex: <http://example.org/> SELECT * WHERE { ?s ?p ?o } VALUES ?x { ex:Alice }").unwrap();
        let vc = q.values.as_ref().unwrap();
        assert!(matches!(&vc.bindings[0][0], DataBlockValue::PrefixedName { prefix, local } if prefix == "ex" && local == "Alice"));
    }

    #[test]
    fn test_values_empty() {
        let q = parse("SELECT * WHERE { ?s ?p ?o } VALUES ?x { }").unwrap();
        let vc = q.values.as_ref().unwrap();
        assert_eq!(vc.variables.len(), 1);
        assert!(vc.bindings.is_empty());
    }
}
