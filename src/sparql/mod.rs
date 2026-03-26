pub mod ast;
pub mod parser;

use std::collections::HashMap;

use crate::error::DarqError;
use crate::rdf::Iri;
use ast::{SelectQuery, TermOrVariable, TriplePattern};

/// Expand all prefixed names in a parsed query into full IRIs.
pub fn expand_prefixes(query: &mut SelectQuery) -> Result<(), DarqError> {
    let prefix_map: HashMap<&str, &str> = query
        .prefixes
        .iter()
        .map(|p| (p.prefix.as_str(), p.iri.0.as_str()))
        .collect();

    for pattern in &mut query.where_pattern.patterns {
        expand_pattern(&prefix_map, pattern)?;
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
}
