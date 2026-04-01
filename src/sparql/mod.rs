pub mod ast;
pub mod parser;

use std::collections::HashSet;

use crate::error::DarqError;
use ast::{SelectClause, SelectQuery, TermOrVariable};

/// Check that all variables in the SELECT clause are bound by a WHERE pattern.
pub fn validate_select_variables(query: &SelectQuery) -> Result<(), DarqError> {
    let select_vars = match &query.select {
        SelectClause::Star | SelectClause::Count { .. } => return Ok(()),
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
            bound.insert(v.as_str());
        }
    }

    let unbound: Vec<String> = select_vars
        .iter()
        .filter(|v| !bound.contains(v.as_str()))
        .map(|v| v.as_str().to_owned())
        .collect();

    if unbound.is_empty() {
        Ok(())
    } else {
        Err(DarqError::UnboundVariables(unbound))
    }
}

fn collect_var<'a>(tov: &'a TermOrVariable, set: &mut HashSet<&'a str>) {
    if let TermOrVariable::Variable(v) = tov {
        set.insert(v.as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::parser::parse;

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
