pub mod ast;
pub mod parser;

use std::collections::{HashMap, HashSet};

use crate::error::DarqError;
use crate::rdf::Iri;
use ast::{DataBlockValue, SelectClause, SelectQuery, TermOrVariable, TriplePattern};

/// Expand all prefixed names in a parsed query into full IRIs.
pub fn expand_prefixes(query: &mut SelectQuery) -> Result<(), DarqError> {
    let prefix_map: HashMap<&str, &str> = query
        .prefixes
        .iter()
        .map(|p| (p.prefix.as_str(), p.iri.0.as_str()))
        .collect();

    expand_ggp(&prefix_map, &mut query.where_pattern)?;

    if let Some(ref mut vc) = query.values {
        for row in &mut vc.bindings {
            for val in row {
                expand_data_block_value(&prefix_map, val)?;
            }
        }
    }

    Ok(())
}

fn expand_ggp(
    prefix_map: &HashMap<&str, &str>,
    ggp: &mut ast::GroupGraphPattern,
) -> Result<(), DarqError> {
    for pattern in &mut ggp.patterns {
        expand_pattern(prefix_map, pattern)?;
    }
    for filter in &mut ggp.filters {
        match filter {
            ast::Filter::NotExists(inner) => expand_ggp(prefix_map, inner)?,
        }
    }
    Ok(())
}

fn expand_pattern(
    prefix_map: &HashMap<&str, &str>,
    pattern: &mut TriplePattern,
) -> Result<(), DarqError> {
    expand_term(prefix_map, &mut pattern.subject)?;
    expand_term(prefix_map, &mut pattern.predicate)?;
    expand_term(prefix_map, &mut pattern.object)?;
    Ok(())
}

fn expand_term(
    prefix_map: &HashMap<&str, &str>,
    term: &mut TermOrVariable,
) -> Result<(), DarqError> {
    if let TermOrVariable::PrefixedName { prefix, local } = term {
        let base = prefix_map
            .get(prefix.as_str())
            .ok_or_else(|| DarqError::UnknownPrefix(prefix.clone()))?;
        *term = TermOrVariable::Iri(Iri::new(format!("{}{}", base, local)));
    }
    Ok(())
}

fn expand_data_block_value(
    prefix_map: &HashMap<&str, &str>,
    value: &mut DataBlockValue,
) -> Result<(), DarqError> {
    if let DataBlockValue::PrefixedName { prefix, local } = value {
        let base = prefix_map
            .get(prefix.as_str())
            .ok_or_else(|| DarqError::UnknownPrefix(prefix.clone()))?;
        *value = DataBlockValue::Iri(Iri::new(format!("{}{}", base, local)));
    }
    Ok(())
}

/// Check that all variables in the SELECT clause are bound by a WHERE pattern.
pub fn validate_select_variables(query: &SelectQuery) -> Result<(), DarqError> {
    let select_vars = match &query.select {
        SelectClause::Star => return Ok(()),
        SelectClause::Variables(vars) => vars,
    };

    let mut bound: HashSet<&str> = HashSet::new();
    for tp in &query.where_pattern.patterns {
        collect_var(&tp.subject, &mut bound);
        collect_var(&tp.predicate, &mut bound);
        collect_var(&tp.object, &mut bound);
    }
    if let Some(ref vc) = query.values {
        for v in &vc.variables {
            bound.insert(&v.0);
        }
    }

    let unbound: Vec<String> = select_vars
        .iter()
        .filter(|v| !bound.contains(v.0.as_str()))
        .map(|v| v.0.clone())
        .collect();

    if unbound.is_empty() {
        Ok(())
    } else {
        Err(DarqError::UnboundVariables(unbound))
    }
}

fn collect_var<'a>(tov: &'a TermOrVariable, set: &mut HashSet<&'a str>) {
    if let TermOrVariable::Variable(v) = tov {
        set.insert(&v.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::parser::parse;

    #[test]
    fn test_expand_prefixes() {
        let mut q = parse(
            "PREFIX ex: <http://example.org/> SELECT * WHERE { ?s ex:name ?o }",
        )
        .unwrap();
        expand_prefixes(&mut q).unwrap();

        match &q.where_pattern.patterns[0].predicate {
            TermOrVariable::Iri(iri) => {
                assert_eq!(iri.0, "http://example.org/name");
            }
            other => panic!("expected Iri, got {:?}", other),
        }
    }

    #[test]
    fn test_unknown_prefix_errors() {
        let mut q = parse("SELECT * WHERE { ?s foaf:name ?o }").unwrap();
        let result = expand_prefixes(&mut q);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_select_all_bound() {
        let q = parse("SELECT ?s ?o WHERE { ?s <http://ex.org/p> ?o }").unwrap();
        assert!(validate_select_variables(&q).is_ok());
    }

    #[test]
    fn test_validate_select_star_always_ok() {
        let q = parse("SELECT * WHERE { ?s <http://ex.org/p> ?o }").unwrap();
        assert!(validate_select_variables(&q).is_ok());
    }

    #[test]
    fn test_validate_select_unbound_variable() {
        let q = parse("SELECT ?name ?age WHERE { ?p <http://ex.org/name> \"Alice\" . ?p <http://ex.org/age> ?age }").unwrap();
        let err = validate_select_variables(&q).unwrap_err();
        assert!(matches!(err, DarqError::UnboundVariables(vars) if vars == ["name"]));
    }
}
