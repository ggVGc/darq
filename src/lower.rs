use std::collections::{HashMap, HashSet};

use crate::error::DarqError;
use crate::ir::{FieldConstraint, InlineData, NotExistsFilter, NullCheckFilter, QueryPattern, QueryPlan, Subject, Value};
use crate::rdf::{Iri, Literal, Term, RDF_TYPE};
use crate::schema::Schema;
use crate::sparql::ast::*;

/// Lower a parsed (and prefix-expanded, predicate-validated) SPARQL query
/// into one or more resource-level query plans.
///
/// When a subject's type is ambiguous (multiple candidate types), produces one
/// plan per candidate so the caller can evaluate all plans and union the results.
pub fn lower(query: &SelectQuery, schema: &Schema) -> Result<Vec<QueryPlan>, DarqError> {
    let values = query.values.as_ref().map(lower_values_clause);
    let pattern_sets = lower_bgp_alternatives(&query.where_pattern, schema)?;

    let mut plans = Vec::new();
    for patterns in pattern_sets {
        // Collect known variable types from the main BGP for propagation into filters.
        let mut outer_var_types: HashMap<String, Iri> = HashMap::new();
        for pattern in &patterns {
            if let QueryPattern::Resource { subject: crate::ir::Subject::Variable(name), type_iri: Some(ti), .. } = pattern {
                outer_var_types.insert(name.clone(), ti.clone());
            }
        }

        let (filters, null_checks) = lower_filters(&query.where_pattern.filters, schema, &outer_var_types)?;

        check_pattern_connectivity(&patterns, &filters, values.as_ref())?;

        plans.push(QueryPlan {
            patterns,
            filters,
            null_checks,
            select: query.select.clone(),
            modifier: query.modifier.clone(),
            values: values.clone(),
        });
    }
    Ok(plans)
}

/// Like [`lower_bgp`] but handles ambiguous types by producing multiple
/// alternative pattern sets — one per candidate type. Recurses to handle
/// multiple ambiguous subjects in the same BGP.
fn lower_bgp_alternatives(
    ggp: &GroupGraphPattern,
    schema: &Schema,
) -> Result<Vec<Vec<QueryPattern>>, DarqError> {
    match lower_bgp(ggp, schema) {
        Ok(patterns) => Ok(vec![patterns]),
        Err(DarqError::AmbiguousType { ref subject, ref candidates }) if candidates.len() > 1 => {
            let mut all = Vec::new();
            for candidate in candidates {
                let augmented = augment_with_type(ggp, subject, candidate);
                let alternatives = lower_bgp_alternatives(&augmented, schema)?;
                all.extend(alternatives);
            }
            Ok(all)
        }
        Err(e) => Err(e),
    }
}

/// Collect variables from a single QueryPattern, including anonymous variables.
fn pattern_variables(pattern: &QueryPattern) -> HashSet<String> {
    let mut vars = HashSet::new();
    let mut add = |name: &str| {
        vars.insert(name.to_string());
    };
    match pattern {
        QueryPattern::Resource {
            subject,
            constraints,
            type_variable,
            ..
        } => {
            if let Subject::Variable(v) = subject {
                add(v);
            }
            if let Some(tv) = type_variable {
                add(tv);
            }
            for c in constraints {
                if let Value::Variable(v) = &c.value {
                    add(v);
                }
            }
        }
        QueryPattern::FieldScan {
            subject,
            predicate_var,
            object,
            ..
        } => {
            if let Subject::Variable(v) = subject {
                add(v);
            }
            add(predicate_var);
            if let Value::Variable(v) = object {
                add(v);
            }
        }
    }
    vars
}

/// Check that all patterns form a single connected variable graph.
/// Patterns that share no variables with any other pattern would produce
/// a cartesian product, which is almost certainly a query error.
fn check_pattern_connectivity(
    patterns: &[QueryPattern],
    filters: &[NotExistsFilter],
    values: Option<&InlineData>,
) -> Result<(), DarqError> {
    // Collect variable sets per pattern, skipping patterns with no variables
    // (fully-bound patterns like `<s> <p> <o>` are boolean checks).
    let var_sets: Vec<HashSet<String>> = patterns
        .iter()
        .map(pattern_variables)
        .filter(|vs| !vs.is_empty())
        .collect();

    if var_sets.len() <= 1 {
        return Ok(());
    }

    // Collect additional "connecting" variables from filters and VALUES.
    // Filter inner patterns reference outer variables, creating connections.
    let mut extra_vars: HashSet<String> = HashSet::new();
    for filter in filters {
        for inner_pattern in &filter.inner_patterns {
            for v in pattern_variables(inner_pattern) {
                extra_vars.insert(v);
            }
        }
    }
    if let Some(vd) = values {
        for v in &vd.variables {
            if !v.starts_with("__anon_") {
                extra_vars.insert(v.clone());
            }
        }
    }

    // Union-find to group patterns by shared variables.
    let n = var_sets.len();
    let mut parent: Vec<usize> = (0..n).collect();

    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    fn union(parent: &mut [usize], a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent[ra] = rb;
        }
    }

    // Map variable -> first pattern index that uses it.
    let mut var_to_pattern: HashMap<&str, usize> = HashMap::new();
    for (i, vs) in var_sets.iter().enumerate() {
        for v in vs {
            if let Some(&prev) = var_to_pattern.get(v.as_str()) {
                union(&mut parent, prev, i);
            } else {
                var_to_pattern.insert(v, i);
            }
        }
    }

    // Filters/VALUES variables connect patterns indirectly:
    // if variable X appears in pattern A and in a filter, and variable Y
    // appears in pattern B and the same filter, then A and B are connected
    // through the filter. We handle this by merging all patterns that share
    // any variable present in extra_vars.
    // Simpler approach: collect all variables from filters/VALUES as one bag,
    // then merge all patterns that contain any of those variables.
    let mut first_with_extra: Option<usize> = None;
    for (i, vs) in var_sets.iter().enumerate() {
        if vs.iter().any(|v| extra_vars.contains(v)) {
            if let Some(prev) = first_with_extra {
                union(&mut parent, prev, i);
            } else {
                first_with_extra = Some(i);
            }
        }
    }

    // Check if all patterns are in the same component.
    let root = find(&mut parent, 0);
    let all_connected = (1..n).all(|i| find(&mut parent, i) == root);

    if all_connected {
        return Ok(());
    }

    // Build groups for error message, excluding anonymous variables from the display
    // (they exist for connectivity but are not user-visible)
    let mut components: HashMap<usize, Vec<String>> = HashMap::new();
    for (i, vs) in var_sets.iter().enumerate() {
        let r = find(&mut parent, i);
        let entry = components.entry(r).or_default();
        for v in vs {
            if !v.starts_with("__anon_") && !entry.contains(v) {
                entry.push(v.clone());
            }
        }
    }
    let mut groups: Vec<Vec<String>> = components.into_values().collect();
    for g in &mut groups {
        g.sort();
    }
    groups.sort();

    Err(DarqError::DisconnectedPatterns { groups })
}

fn lower_values_clause(vc: &ValuesClause) -> InlineData {
    let variables = vc.variables.iter().map(|v| v.as_str().to_owned()).collect();
    let rows = vc
        .bindings
        .iter()
        .map(|row| row.iter().map(ground_term_to_term).collect())
        .collect();
    InlineData { variables, rows }
}

fn ground_term_to_term(val: &Option<GroundTerm>) -> Option<Term> {
    match val {
        None => None,
        Some(GroundTerm::NamedNode(nn)) => Some(Term::Iri(Iri::new(nn.as_str().to_owned()))),
        Some(GroundTerm::Literal(lit)) => Some(sparql_lit_to_term(lit)),
    }
}

/// Lower FILTER NOT EXISTS clauses into NotExistsFilters or NullCheckFilters.
/// When the schema has a registered rewrite for `NOT EXISTS {[] pred ?var}`,
/// emits a NullCheckFilter instead of the expensive subquery.
/// When the inner pattern has an ambiguous type, produce one filter per candidate type,
/// narrowing candidates using known types from the outer query.
fn lower_filters(
    filters: &[Filter],
    schema: &Schema,
    outer_var_types: &HashMap<String, Iri>,
) -> Result<(Vec<NotExistsFilter>, Vec<NullCheckFilter>), DarqError> {
    let mut not_exists = Vec::new();
    let mut null_checks = Vec::new();
    for filter in filters {
        match filter {
            Filter::NotExists(ggp) => {
                if let Some(nc) = try_not_exists_rewrite(ggp, schema, outer_var_types) {
                    null_checks.push(nc);
                    continue;
                }
                match lower_bgp(ggp, schema) {
                    Ok(patterns) => {
                        not_exists.push(NotExistsFilter { inner_patterns: patterns });
                    }
                    Err(DarqError::AmbiguousType { ref subject, ref candidates })
                        if candidates.len() > 1 =>
                    {
                        let narrowed = narrow_candidates_by_outer_types(
                            candidates, ggp, schema, outer_var_types,
                        );
                        for candidate in &narrowed {
                            let augmented = augment_with_type(ggp, subject, candidate);
                            let patterns = lower_bgp(&augmented, schema)?;
                            not_exists.push(NotExistsFilter { inner_patterns: patterns });
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
        }
    }
    Ok((not_exists, null_checks))
}

/// Try to rewrite `FILTER NOT EXISTS {[] pred ?var}` as a null-check on the
/// outer variable's resource fields, using a schema-registered rewrite.
fn try_not_exists_rewrite(
    ggp: &GroupGraphPattern,
    schema: &Schema,
    outer_var_types: &HashMap<String, Iri>,
) -> Option<NullCheckFilter> {
    if ggp.patterns.len() != 1 || !ggp.filters.is_empty() {
        return None;
    }
    let tp = &ggp.patterns[0];

    // Subject must be anonymous (blank node / [])
    match &tp.subject {
        TermOrVariable::Variable(v) if v.as_str().starts_with("__anon_") => {}
        _ => return None,
    }

    // Predicate must be a concrete IRI
    let pred_iri = match &tp.predicate {
        TermOrVariable::Iri(iri) => iri,
        _ => return None,
    };

    // Object must be a variable with a known type in the outer query
    let obj_name = match &tp.object {
        TermOrVariable::Variable(v) => v.as_str(),
        _ => return None,
    };

    let obj_type = outer_var_types.get(obj_name)?;
    let null_fields = schema.not_exists_rewrite(pred_iri, obj_type)?;

    Some(NullCheckFilter {
        variable: obj_name.to_owned(),
        field_names: null_fields.to_vec(),
    })
}

/// Narrow ambiguous subject type candidates using known types of variables
/// from the outer query. For each candidate type, check whether its field
/// definitions are compatible with the known types of correlated variables.
fn narrow_candidates_by_outer_types(
    candidates: &[Iri],
    ggp: &GroupGraphPattern,
    schema: &Schema,
    outer_var_types: &HashMap<String, Iri>,
) -> Vec<Iri> {
    let mut narrowed = candidates.to_vec();

    for tp in &ggp.patterns {
        if let TermOrVariable::Iri(pred_iri) = &tp.predicate {
            if let TermOrVariable::Variable(obj_var) = &tp.object {
                if let Some(obj_type) = outer_var_types.get(obj_var.as_str()) {
                    narrowed.retain(|candidate| {
                        match schema.field_range_for_type(candidate, pred_iri) {
                            Some(range) => range.contains(obj_type),
                            None => true, // No range info → can't narrow → keep
                        }
                    });
                }
            }
        }
    }

    narrowed
}

/// Prepend `?subject a <type_iri>` to a GGP to disambiguate the subject's type.
fn augment_with_type(ggp: &GroupGraphPattern, subject_name: &str, type_iri: &Iri) -> GroupGraphPattern {
    let mut patterns = vec![TriplePattern {
        subject: TermOrVariable::Variable(Variable::new_unchecked(subject_name)),
        predicate: TermOrVariable::Iri(Iri::new(RDF_TYPE)),
        object: TermOrVariable::Iri(type_iri.clone()),
    }];
    patterns.extend(ggp.patterns.clone());
    GroupGraphPattern {
        patterns,
        filters: ggp.filters.clone(),
    }
}

/// Lower a basic graph pattern into a list of QueryPatterns.
fn lower_bgp(
    bgp: &GroupGraphPattern,
    schema: &Schema,
) -> Result<Vec<QueryPattern>, DarqError> {
    let groups = group_by_subject(&bgp.patterns);
    let ref_constraints = collect_reference_constraints(&bgp.patterns, schema);
    let mut result = Vec::new();

    for group in groups {
        lower_subject_group(&group, schema, &ref_constraints, &mut result)?;
    }

    Ok(result)
}

/// Collect type constraints for variables that appear as objects of Reference-typed predicates.
/// For example, if `?subm` is the object of `cpmeta:wasSubmittedBy` which is
/// `Reference([DataSubmission])`, then `?subm` is constrained to `DataSubmission`.
fn collect_reference_constraints(
    patterns: &[TriplePattern],
    schema: &Schema,
) -> HashMap<String, Vec<Iri>> {
    let mut constraints: HashMap<String, Vec<Iri>> = HashMap::new();
    for tp in patterns {
        if let (TermOrVariable::Iri(pred_iri), TermOrVariable::Variable(obj_var)) =
            (&tp.predicate, &tp.object)
        {
            let range = schema.range_types(pred_iri);
            if !range.is_empty() {
                let entry = constraints.entry(obj_var.as_str().to_owned()).or_default();
                if entry.is_empty() {
                    *entry = range;
                } else {
                    entry.retain(|t| range.contains(t));
                }
            }
        }
    }
    constraints
}

/// A group of triple patterns sharing the same subject.
struct SubjectGroup {
    subject: SubjectKey,
    patterns: Vec<(PredicateKind, ObjectInfo)>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum SubjectKey {
    Variable(String),
    Iri(Iri),
}

enum PredicateKind {
    RdfType,
    Concrete(Iri),
    Variable(String),
}

enum ObjectInfo {
    Variable(String),
    Iri(Iri),
    Literal(spargebra::term::Literal),
}

/// Group AST triple patterns by subject, preserving first-appearance order.
fn group_by_subject(patterns: &[TriplePattern]) -> Vec<SubjectGroup> {
    let mut groups: Vec<SubjectGroup> = Vec::new();
    let mut index: HashMap<SubjectKey, usize> = HashMap::new();

    for tp in patterns {
        let key = subject_key(&tp.subject);
        let pred = predicate_kind(&tp.predicate);
        let obj = object_info(&tp.object);

        if let Some(&idx) = index.get(&key) {
            groups[idx].patterns.push((pred, obj));
        } else {
            let idx = groups.len();
            index.insert(key.clone(), idx);
            groups.push(SubjectGroup {
                subject: key,
                patterns: vec![(pred, obj)],
            });
        }
    }

    groups
}

fn subject_key(tov: &TermOrVariable) -> SubjectKey {
    match tov {
        TermOrVariable::Variable(v) => SubjectKey::Variable(v.as_str().to_owned()),
        TermOrVariable::Iri(iri) => SubjectKey::Iri(iri.clone()),
        TermOrVariable::Literal(_) => unreachable!("literals cannot appear in subject position"),
    }
}

fn predicate_kind(tov: &TermOrVariable) -> PredicateKind {
    match tov {
        TermOrVariable::Iri(iri) if iri.0 == RDF_TYPE => PredicateKind::RdfType,
        TermOrVariable::Iri(iri) => PredicateKind::Concrete(iri.clone()),
        TermOrVariable::Variable(v) => PredicateKind::Variable(v.as_str().to_owned()),
        TermOrVariable::Literal(_) => unreachable!("literals cannot appear in predicate position"),
    }
}

fn object_info(tov: &TermOrVariable) -> ObjectInfo {
    match tov {
        TermOrVariable::Variable(v) => ObjectInfo::Variable(v.as_str().to_owned()),
        TermOrVariable::Iri(iri) => ObjectInfo::Iri(iri.clone()),
        TermOrVariable::Literal(lit) => ObjectInfo::Literal(lit.clone()),
    }
}

/// Lower a single subject group into QueryPatterns.
fn lower_subject_group(
    group: &SubjectGroup,
    schema: &Schema,
    ref_constraints: &HashMap<String, Vec<Iri>>,
    result: &mut Vec<QueryPattern>,
) -> Result<(), DarqError> {
    let subject = to_ir_subject(&group.subject);

    // Classify patterns within the group
    let mut explicit_type: Option<Iri> = None;
    let mut type_variable: Option<String> = None;
    let mut concrete_predicates: Vec<(&Iri, &ObjectInfo)> = Vec::new();
    let mut variable_predicates: Vec<(&String, &ObjectInfo)> = Vec::new();

    for (pred, obj) in &group.patterns {
        match pred {
            PredicateKind::RdfType => match obj {
                ObjectInfo::Iri(iri) => {
                    // Validate the type is known
                    if schema.fields_for_type(iri).is_none() {
                        return Err(DarqError::UnknownType(iri.clone()));
                    }
                    explicit_type = Some(iri.clone());
                }
                ObjectInfo::Variable(name) => {
                    type_variable = Some(name.clone());
                }
                ObjectInfo::Literal(_) => {
                    // rdf:type with a literal object — won't match anything,
                    // but we can still produce a valid plan
                }
            },
            PredicateKind::Concrete(iri) => {
                concrete_predicates.push((iri, obj));
            }
            PredicateKind::Variable(name) => {
                variable_predicates.push((name, obj));
            }
        }
    }

    // Determine the resource type
    let type_iri = if let Some(t) = explicit_type {
        Some(t)
    } else if !concrete_predicates.is_empty() {
        Some(infer_type(&group.subject, &concrete_predicates, schema, ref_constraints)?)
    } else {
        None
    };

    // Build field constraints from concrete predicates
    let constraints = if let Some(ref ti) = type_iri {
        build_field_constraints(&concrete_predicates, ti, schema)?
    } else {
        vec![]
    };

    // Emit Resource pattern if there's a type or constraints
    let need_resource = type_iri.is_some() || type_variable.is_some();
    if need_resource {
        result.push(QueryPattern::Resource {
            subject: subject.clone(),
            type_iri: type_iri.clone(),
            constraints,
            type_variable: type_variable.clone(),
        });
    }

    // Emit FieldScan for each variable-predicate pattern
    for (pred_var, obj) in &variable_predicates {
        result.push(QueryPattern::FieldScan {
            subject: subject.clone(),
            predicate_var: (*pred_var).clone(),
            object: to_ir_value(obj),
            type_iri: type_iri.clone(),
        });
    }

    Ok(())
}

/// Infer the resource type from concrete predicates by intersecting type sets.
fn infer_type(
    subject: &SubjectKey,
    predicates: &[(&Iri, &ObjectInfo)],
    schema: &Schema,
    ref_constraints: &HashMap<String, Vec<Iri>>,
) -> Result<Iri, DarqError> {
    let mut candidate_types: Option<Vec<Iri>> = None;

    for (pred_iri, _) in predicates {
        let types = schema.types_for_predicate(pred_iri);
        match &mut candidate_types {
            None => {
                candidate_types = Some(types.to_vec());
            }
            Some(candidates) => {
                candidates.retain(|t| types.contains(t));
            }
        }
    }

    // Narrow candidates using reference constraints from other subject groups.
    if let SubjectKey::Variable(name) = subject {
        if let Some(ref_types) = ref_constraints.get(name) {
            match &mut candidate_types {
                None => candidate_types = Some(ref_types.clone()),
                Some(candidates) => candidates.retain(|t| ref_types.contains(t)),
            }
        }
    }

    let candidates = candidate_types.unwrap_or_default();
    let subject_name = match subject {
        SubjectKey::Variable(name) => name.clone(),
        SubjectKey::Iri(iri) => iri.0.clone(),
    };

    match candidates.len() {
        0 => Err(DarqError::AmbiguousType {
            subject: subject_name,
            candidates: vec![],
        }),
        1 => Ok(candidates.into_iter().next().unwrap()),
        _ => Err(DarqError::AmbiguousType {
            subject: subject_name,
            candidates,
        }),
    }
}

/// Build field constraints from concrete predicate patterns.
fn build_field_constraints(
    predicates: &[(&Iri, &ObjectInfo)],
    type_iri: &Iri,
    schema: &Schema,
) -> Result<Vec<FieldConstraint>, DarqError> {
    let mut constraints = Vec::new();

    for (pred_iri, obj) in predicates {
        let field_name = schema
            .field_name(type_iri, pred_iri)
            .ok_or_else(|| DarqError::UnknownPredicate((*pred_iri).clone()))?;

        constraints.push(FieldConstraint {
            field_name: field_name.to_string(),
            value: to_ir_value(obj),
        });
    }

    Ok(constraints)
}

fn to_ir_subject(key: &SubjectKey) -> Subject {
    match key {
        SubjectKey::Variable(name) => Subject::Variable(name.clone()),
        SubjectKey::Iri(iri) => Subject::Bound(iri.clone()),
    }
}

fn to_ir_value(obj: &ObjectInfo) -> Value {
    match obj {
        ObjectInfo::Variable(name) => Value::Variable(name.clone()),
        ObjectInfo::Iri(iri) => Value::Bound(Term::Iri(iri.clone())),
        ObjectInfo::Literal(lit) => Value::Bound(sparql_lit_to_term(lit)),
    }
}

fn sparql_lit_to_term(lit: &spargebra::term::Literal) -> Term {
    let datatype = lit.datatype().as_str();
    let value = lit.value();
    match datatype {
        "http://www.w3.org/2001/XMLSchema#integer" => {
            value.parse::<i64>()
                .map(|n| Term::Literal(Literal::Integer(n)))
                .unwrap_or_else(|_| Term::Literal(Literal::String(value.to_owned())))
        }
        "http://www.w3.org/2001/XMLSchema#boolean" => match value {
            "true" | "1" => Term::Literal(Literal::Boolean(true)),
            "false" | "0" => Term::Literal(Literal::Boolean(false)),
            _ => Term::Literal(Literal::String(value.to_owned())),
        },
        _ => Term::Literal(Literal::String(value.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::Iri;
    use crate::schema::{FieldDescriptor, FieldType, Resource, Schema};

    struct Person {
        id: String,
        name: String,
        age: i64,
    }

    impl Resource for Person {
        fn rdf_type() -> Iri {
            Iri::new("http://example.org/Person")
        }

        fn subject_iri(&self) -> Iri {
            Iri::new(format!("http://example.org/person/{}", self.id))
        }

        fn field_descriptors() -> Vec<FieldDescriptor> {
            vec![
                FieldDescriptor {
                    predicate: Iri::new("http://example.org/name"),
                    name: "name",
                    field_type: FieldType::String,
                    indexed: false,
                },
                FieldDescriptor {
                    predicate: Iri::new("http://example.org/age"),
                    name: "age",
                    field_type: FieldType::Integer,
                    indexed: false,
                },
            ]
        }

        fn field_values(&self) -> Vec<Term> {
            vec![self.name.clone().into(), self.age.into()]
        }
    }

    fn test_schema() -> Schema {
        let mut schema = Schema::new();
        schema.register::<Person>();
        schema
    }

    #[test]
    fn test_lower_typed_with_fields() {
        let schema = test_schema();

        // ?p a ex:Person . ?p ex:name ?name . ?p ex:age ?age
        let bgp = GroupGraphPattern {
            patterns: vec![
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("p")),
                    predicate: TermOrVariable::Iri(Iri::new(RDF_TYPE)),
                    object: TermOrVariable::Iri(Iri::new("http://example.org/Person")),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("p")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("name")),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("p")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/age")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("age")),
                },
            ],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 1);
        match &patterns[0] {
            QueryPattern::Resource {
                subject,
                type_iri,
                constraints,
                type_variable,
            } => {
                assert!(matches!(subject, Subject::Variable(v) if v == "p"));
                assert_eq!(type_iri.as_ref().unwrap().0, "http://example.org/Person");
                assert_eq!(constraints.len(), 2);
                assert_eq!(constraints[0].field_name, "name");
                assert!(matches!(&constraints[0].value, Value::Variable(v) if v == "name"));
                assert_eq!(constraints[1].field_name, "age");
                assert!(matches!(&constraints[1].value, Value::Variable(v) if v == "age"));
                assert!(type_variable.is_none());
            }
            _ => panic!("expected Resource pattern"),
        }
    }

    #[test]
    fn test_lower_inferred_type() {
        let schema = test_schema();

        // ?p ex:name ?name (no explicit type — infer Person from predicate)
        let bgp = GroupGraphPattern {
            patterns: vec![TriplePattern {
                subject: TermOrVariable::Variable(Variable::new_unchecked("p")),
                predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                object: TermOrVariable::Variable(Variable::new_unchecked("name")),
            }],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 1);
        match &patterns[0] {
            QueryPattern::Resource { type_iri, .. } => {
                assert_eq!(type_iri.as_ref().unwrap().0, "http://example.org/Person");
            }
            _ => panic!("expected Resource pattern"),
        }
    }

    #[test]
    fn test_lower_variable_predicate() {
        let schema = test_schema();

        // ?s ?p ?o
        let bgp = GroupGraphPattern {
            patterns: vec![TriplePattern {
                subject: TermOrVariable::Variable(Variable::new_unchecked("s")),
                predicate: TermOrVariable::Variable(Variable::new_unchecked("p")),
                object: TermOrVariable::Variable(Variable::new_unchecked("o")),
            }],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 1);
        match &patterns[0] {
            QueryPattern::FieldScan {
                subject,
                predicate_var,
                object,
                type_iri,
            } => {
                assert!(matches!(subject, Subject::Variable(v) if v == "s"));
                assert_eq!(predicate_var, "p");
                assert!(matches!(object, Value::Variable(v) if v == "o"));
                assert!(type_iri.is_none());
            }
            _ => panic!("expected FieldScan pattern"),
        }
    }

    #[test]
    fn test_lower_mixed_concrete_and_variable_predicates() {
        let schema = test_schema();

        // ?person ex:name ?name . ?person ?p ?o
        let bgp = GroupGraphPattern {
            patterns: vec![
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("person")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("name")),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("person")),
                    predicate: TermOrVariable::Variable(Variable::new_unchecked("p")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("o")),
                },
            ],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 2);
        assert!(matches!(&patterns[0], QueryPattern::Resource { .. }));
        assert!(matches!(&patterns[1], QueryPattern::FieldScan { type_iri: Some(_), .. }));
    }

    #[test]
    fn test_lower_type_variable() {
        let schema = test_schema();

        // ?s a ?type . ?s ex:name ?name
        let bgp = GroupGraphPattern {
            patterns: vec![
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("s")),
                    predicate: TermOrVariable::Iri(Iri::new(RDF_TYPE)),
                    object: TermOrVariable::Variable(Variable::new_unchecked("type")),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("s")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("name")),
                },
            ],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 1);
        match &patterns[0] {
            QueryPattern::Resource {
                type_iri,
                type_variable,
                constraints,
                ..
            } => {
                // Type inferred from ex:name
                assert_eq!(type_iri.as_ref().unwrap().0, "http://example.org/Person");
                assert_eq!(type_variable.as_ref().unwrap(), "type");
                assert_eq!(constraints.len(), 1);
            }
            _ => panic!("expected Resource pattern"),
        }
    }

    #[test]
    fn test_lower_unknown_type_errors() {
        let schema = test_schema();

        // ?s a ex:Unknown
        let bgp = GroupGraphPattern {
            patterns: vec![TriplePattern {
                subject: TermOrVariable::Variable(Variable::new_unchecked("s")),
                predicate: TermOrVariable::Iri(Iri::new(RDF_TYPE)),
                object: TermOrVariable::Iri(Iri::new("http://example.org/Unknown")),
            }],
        filters: vec![],
            };

        let err = lower_bgp(&bgp, &schema).unwrap_err();
        assert!(matches!(err, DarqError::UnknownType(_)));
    }

    // --- Group 1: Value/Subject variants ---

    #[test]
    fn test_lower_bound_literal_object() {
        let schema = test_schema();

        // ?p ex:name "Alice"
        let bgp = GroupGraphPattern {
            patterns: vec![TriplePattern {
                subject: TermOrVariable::Variable(Variable::new_unchecked("p")),
                predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                object: TermOrVariable::Literal(spargebra::term::Literal::new_simple_literal("Alice")),
            }],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 1);
        match &patterns[0] {
            QueryPattern::Resource { constraints, .. } => {
                assert_eq!(constraints.len(), 1);
                assert_eq!(constraints[0].field_name, "name");
                assert!(matches!(
                    &constraints[0].value,
                    Value::Bound(Term::Literal(Literal::String(s))) if s == "Alice"
                ));
            }
            _ => panic!("expected Resource pattern"),
        }
    }

    #[test]
    fn test_lower_bound_iri_object() {
        // Need a type with an IRI-valued field
        struct Thing {
            id: String,
        }

        impl Resource for Thing {
            fn rdf_type() -> Iri {
                Iri::new("http://example.org/Thing")
            }
            fn subject_iri(&self) -> Iri {
                Iri::new(format!("http://example.org/thing/{}", self.id))
            }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![FieldDescriptor {
                    predicate: Iri::new("http://example.org/owner"),
                    name: "owner",
                    field_type: FieldType::String,
                    indexed: false,
                }]
            }
            fn field_values(&self) -> Vec<Term> {
                vec![Term::Iri(Iri::new("http://example.org/person/alice"))]
            }
        }

        let mut schema = Schema::new();
        schema.register::<Thing>();

        // ?t ex:owner <http://example.org/person/bob>
        let bgp = GroupGraphPattern {
            patterns: vec![TriplePattern {
                subject: TermOrVariable::Variable(Variable::new_unchecked("t")),
                predicate: TermOrVariable::Iri(Iri::new("http://example.org/owner")),
                object: TermOrVariable::Iri(Iri::new("http://example.org/person/bob")),
            }],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 1);
        match &patterns[0] {
            QueryPattern::Resource { constraints, .. } => {
                assert_eq!(constraints[0].field_name, "owner");
                assert!(matches!(
                    &constraints[0].value,
                    Value::Bound(Term::Iri(iri)) if iri.0 == "http://example.org/person/bob"
                ));
            }
            _ => panic!("expected Resource pattern"),
        }
    }

    #[test]
    fn test_lower_bound_subject() {
        let schema = test_schema();

        // <http://example.org/person/alice> ex:name ?name
        let bgp = GroupGraphPattern {
            patterns: vec![TriplePattern {
                subject: TermOrVariable::Iri(Iri::new("http://example.org/person/alice")),
                predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                object: TermOrVariable::Variable(Variable::new_unchecked("name")),
            }],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 1);
        match &patterns[0] {
            QueryPattern::Resource { subject, type_iri, constraints, .. } => {
                assert!(matches!(
                    subject,
                    Subject::Bound(iri) if iri.0 == "http://example.org/person/alice"
                ));
                assert_eq!(type_iri.as_ref().unwrap().0, "http://example.org/Person");
                assert_eq!(constraints.len(), 1);
            }
            _ => panic!("expected Resource pattern"),
        }
    }

    // --- Group 2: Grouping and ordering ---

    #[test]
    fn test_lower_multiple_subject_groups() {
        let schema = test_schema();

        // ?p ex:name ?name . ?q ex:age ?age
        let bgp = GroupGraphPattern {
            patterns: vec![
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("p")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("name")),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("q")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/age")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("age")),
                },
            ],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 2);

        // First group is ?p (appeared first)
        match &patterns[0] {
            QueryPattern::Resource { subject, constraints, .. } => {
                assert!(matches!(subject, Subject::Variable(v) if v == "p"));
                assert_eq!(constraints[0].field_name, "name");
            }
            _ => panic!("expected Resource pattern for ?p"),
        }

        // Second group is ?q
        match &patterns[1] {
            QueryPattern::Resource { subject, constraints, .. } => {
                assert!(matches!(subject, Subject::Variable(v) if v == "q"));
                assert_eq!(constraints[0].field_name, "age");
            }
            _ => panic!("expected Resource pattern for ?q"),
        }
    }

    #[test]
    fn test_lower_non_adjacent_same_subject() {
        let schema = test_schema();

        // ?p ex:name ?name . ?q ex:age ?age . ?p ex:age ?a
        let bgp = GroupGraphPattern {
            patterns: vec![
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("p")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("name")),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("q")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/age")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("age")),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("p")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/age")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("a")),
                },
            ],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 2);

        // ?p group collects patterns 0 and 2
        match &patterns[0] {
            QueryPattern::Resource { subject, constraints, .. } => {
                assert!(matches!(subject, Subject::Variable(v) if v == "p"));
                assert_eq!(constraints.len(), 2);
                assert_eq!(constraints[0].field_name, "name");
                assert_eq!(constraints[1].field_name, "age");
            }
            _ => panic!("expected Resource pattern for ?p"),
        }

        // ?q group
        match &patterns[1] {
            QueryPattern::Resource { subject, constraints, .. } => {
                assert!(matches!(subject, Subject::Variable(v) if v == "q"));
                assert_eq!(constraints.len(), 1);
            }
            _ => panic!("expected Resource pattern for ?q"),
        }
    }

    // --- Group 3: Type handling edge cases ---

    #[test]
    fn test_lower_type_only_no_fields() {
        let schema = test_schema();

        // ?p a ex:Person
        let bgp = GroupGraphPattern {
            patterns: vec![TriplePattern {
                subject: TermOrVariable::Variable(Variable::new_unchecked("p")),
                predicate: TermOrVariable::Iri(Iri::new(RDF_TYPE)),
                object: TermOrVariable::Iri(Iri::new("http://example.org/Person")),
            }],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 1);
        match &patterns[0] {
            QueryPattern::Resource { type_iri, constraints, type_variable, .. } => {
                assert_eq!(type_iri.as_ref().unwrap().0, "http://example.org/Person");
                assert!(constraints.is_empty());
                assert!(type_variable.is_none());
            }
            _ => panic!("expected Resource pattern"),
        }
    }

    #[test]
    fn test_lower_type_variable_alone() {
        let schema = test_schema();

        // ?s a ?type (no concrete predicates to infer type)
        let bgp = GroupGraphPattern {
            patterns: vec![TriplePattern {
                subject: TermOrVariable::Variable(Variable::new_unchecked("s")),
                predicate: TermOrVariable::Iri(Iri::new(RDF_TYPE)),
                object: TermOrVariable::Variable(Variable::new_unchecked("type")),
            }],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 1);
        match &patterns[0] {
            QueryPattern::Resource { type_iri, type_variable, constraints, .. } => {
                assert!(type_iri.is_none());
                assert_eq!(type_variable.as_ref().unwrap(), "type");
                assert!(constraints.is_empty());
            }
            _ => panic!("expected Resource pattern"),
        }
    }

    #[test]
    fn test_lower_rdf_type_as_full_iri() {
        let schema = test_schema();

        // ?s <rdf:type full IRI> ex:Person — same semantics as `?s a ex:Person`
        let bgp = GroupGraphPattern {
            patterns: vec![TriplePattern {
                subject: TermOrVariable::Variable(Variable::new_unchecked("s")),
                predicate: TermOrVariable::Iri(Iri::new(RDF_TYPE)),
                object: TermOrVariable::Iri(Iri::new("http://example.org/Person")),
            }],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 1);
        match &patterns[0] {
            QueryPattern::Resource { type_iri, constraints, .. } => {
                assert_eq!(type_iri.as_ref().unwrap().0, "http://example.org/Person");
                assert!(constraints.is_empty());
            }
            _ => panic!("expected Resource pattern"),
        }
    }

    // --- Group 4: FieldScan variants ---

    #[test]
    fn test_lower_field_scan_bound_object() {
        let schema = test_schema();

        // ?s ?p "Alice"
        let bgp = GroupGraphPattern {
            patterns: vec![TriplePattern {
                subject: TermOrVariable::Variable(Variable::new_unchecked("s")),
                predicate: TermOrVariable::Variable(Variable::new_unchecked("p")),
                object: TermOrVariable::Literal(spargebra::term::Literal::new_simple_literal("Alice")),
            }],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 1);
        match &patterns[0] {
            QueryPattern::FieldScan { predicate_var, object, type_iri, .. } => {
                assert_eq!(predicate_var, "p");
                assert!(matches!(
                    object,
                    Value::Bound(Term::Literal(Literal::String(s))) if s == "Alice"
                ));
                assert!(type_iri.is_none());
            }
            _ => panic!("expected FieldScan pattern"),
        }
    }

    // --- Group 5: Multiple constraints on same field ---

    #[test]
    fn test_lower_duplicate_field_constraints() {
        let schema = test_schema();

        // ?p ex:name "Alice" . ?p ex:name ?name
        let bgp = GroupGraphPattern {
            patterns: vec![
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("p")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Literal(spargebra::term::Literal::new_simple_literal("Alice")),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("p")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("name")),
                },
            ],
        filters: vec![],
            };

        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 1);
        match &patterns[0] {
            QueryPattern::Resource { constraints, .. } => {
                assert_eq!(constraints.len(), 2);
                assert_eq!(constraints[0].field_name, "name");
                assert_eq!(constraints[1].field_name, "name");
                assert!(matches!(
                    &constraints[0].value,
                    Value::Bound(Term::Literal(Literal::String(s))) if s == "Alice"
                ));
                assert!(matches!(&constraints[1].value, Value::Variable(v) if v == "name"));
            }
            _ => panic!("expected Resource pattern"),
        }
    }

    // --- Group 7: Empty input ---

    #[test]
    fn test_lower_empty_bgp() {
        let schema = test_schema();

        let bgp = GroupGraphPattern { patterns: vec![], filters: vec![] };
        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert!(patterns.is_empty());
    }

    // --- Bug reproduction: cross-group reference type disambiguation ---

    // When a variable is the object of a Reference field (constraining it to a
    // single type) and then used as a subject with a predicate shared by multiple
    // types, the type should be disambiguated by the reference constraint.
    #[test]
    fn test_cross_group_reference_type_disambiguation() {
        struct Parent;
        struct Child;
        struct Sibling;

        impl Resource for Parent {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Parent") }
            fn subject_iri(&self) -> Iri { Iri::new("http://example.org/parent/1") }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![FieldDescriptor {
                    predicate: Iri::new("http://example.org/hasChild"),
                    name: "has_child",
                    field_type: FieldType::Reference(vec![Iri::new("http://example.org/Child")]),
                    indexed: false,
                }]
            }
            fn field_values(&self) -> Vec<Term> { vec![] }
        }

        impl Resource for Child {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Child") }
            fn subject_iri(&self) -> Iri { Iri::new("http://example.org/child/1") }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![FieldDescriptor {
                    predicate: Iri::new("http://example.org/timestamp"),
                    name: "timestamp",
                    field_type: FieldType::DateTime,
                    indexed: false,
                }]
            }
            fn field_values(&self) -> Vec<Term> { vec![] }
        }

        impl Resource for Sibling {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Sibling") }
            fn subject_iri(&self) -> Iri { Iri::new("http://example.org/sibling/1") }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![FieldDescriptor {
                    predicate: Iri::new("http://example.org/timestamp"),
                    name: "timestamp",
                    field_type: FieldType::DateTime,
                    indexed: false,
                }]
            }
            fn field_values(&self) -> Vec<Term> { vec![] }
        }

        let mut schema = Schema::new();
        schema.register::<Parent>();
        schema.register::<Child>();
        schema.register::<Sibling>();

        // ?parent ex:hasChild ?child .  (hasChild is Reference([Child]))
        // ?child ex:timestamp ?ts .     (timestamp exists on both Child and Sibling)
        let bgp = GroupGraphPattern {
            patterns: vec![
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("parent")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/hasChild")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("child")),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("child")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/timestamp")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("ts")),
                },
            ],
        filters: vec![],
            };

        // The reference constraint from hasChild should disambiguate ?child to Child,
        // even though ex:timestamp alone is ambiguous (Child vs Sibling).
        let patterns = lower_bgp(&bgp, &schema).unwrap();
        assert_eq!(patterns.len(), 2);

        match &patterns[1] {
            QueryPattern::Resource { type_iri, .. } => {
                assert_eq!(type_iri.as_ref().unwrap().0, "http://example.org/Child");
            }
            _ => panic!("expected Resource pattern for ?child"),
        }
    }

    #[test]
    fn test_filter_not_exists_narrows_by_outer_variable_type() {
        // Three types share predicate "nextVersionOf":
        //   Alpha  → nextVersionOf: ReferenceArray([Alpha])
        //   Beta   → nextVersionOf: ReferenceArray([Beta])
        //   Gamma  → nextVersionOf: Reference([Alpha])
        // Alpha also has a unique predicate "spec".
        // Query: ?x spec ?s . FILTER NOT EXISTS { [] nextVersionOf ?x }
        // Since ?x is constrained to Alpha (via spec), only Alpha and Gamma
        // (whose nextVersionOf targets Alpha) should produce NOT EXISTS filters.
        // Beta should be eliminated.

        struct Alpha;
        impl Resource for Alpha {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Alpha") }
            fn subject_iri(&self) -> Iri { unimplemented!() }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/spec"),
                        name: "spec",
                        field_type: FieldType::String,
                        indexed: false,
                    },
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/nextVersionOf"),
                        name: "next_version_of",
                        field_type: FieldType::ReferenceArray(vec![Iri::new("http://example.org/Alpha")]),
                        indexed: false,
                    },
                ]
            }
            fn field_values(&self) -> Vec<Term> { unimplemented!() }
        }

        struct Beta;
        impl Resource for Beta {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Beta") }
            fn subject_iri(&self) -> Iri { unimplemented!() }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/nextVersionOf"),
                        name: "next_version_of",
                        field_type: FieldType::ReferenceArray(vec![Iri::new("http://example.org/Beta")]),
                        indexed: false,
                    },
                ]
            }
            fn field_values(&self) -> Vec<Term> { unimplemented!() }
        }

        struct Gamma;
        impl Resource for Gamma {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Gamma") }
            fn subject_iri(&self) -> Iri { unimplemented!() }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/nextVersionOf"),
                        name: "next_version_of",
                        field_type: FieldType::Reference(vec![Iri::new("http://example.org/Alpha")]),
                        indexed: false,
                    },
                ]
            }
            fn field_values(&self) -> Vec<Term> { unimplemented!() }
        }

        let mut schema = Schema::new();
        schema.register::<Alpha>();
        schema.register::<Beta>();
        schema.register::<Gamma>();

        let query = crate::sparql::ast::SelectQuery {
            select: SelectClause::Variables(vec![Variable::new_unchecked("x")]),
            where_pattern: GroupGraphPattern {
                patterns: vec![
                    TriplePattern {
                        subject: TermOrVariable::Variable(Variable::new_unchecked("x")),
                        predicate: TermOrVariable::Iri(Iri::new("http://example.org/spec")),
                        object: TermOrVariable::Variable(Variable::new_unchecked("s")),
                    },
                ],
                filters: vec![
                    Filter::NotExists(GroupGraphPattern {
                        patterns: vec![
                            TriplePattern {
                                subject: TermOrVariable::Variable(Variable::new_unchecked("_anon0")),
                                predicate: TermOrVariable::Iri(Iri::new("http://example.org/nextVersionOf")),
                                object: TermOrVariable::Variable(Variable::new_unchecked("x")),
                            },
                        ],
                        filters: vec![],
                    }),
                ],
            },
            modifier: SolutionModifier::default(),
            values: None,
        };

        let plans = lower(&query, &schema).unwrap();
        assert_eq!(plans.len(), 1);
        let plan = &plans[0];

        // Should have 2 NOT EXISTS filters (Alpha and Gamma), not 3.
        assert_eq!(plan.filters.len(), 2, "expected 2 filters (Alpha + Gamma), got {}", plan.filters.len());

        // Verify the filter types are Alpha and Gamma (not Beta).
        let filter_types: Vec<_> = plan.filters.iter().map(|f| {
            match &f.inner_patterns[0] {
                QueryPattern::Resource { type_iri, .. } => type_iri.as_ref().unwrap().0.clone(),
                _ => panic!("expected Resource pattern"),
            }
        }).collect();
        assert!(filter_types.contains(&"http://example.org/Alpha".to_string()));
        assert!(filter_types.contains(&"http://example.org/Gamma".to_string()));
        assert!(!filter_types.contains(&"http://example.org/Beta".to_string()));
    }

    #[test]
    fn test_ambiguous_main_bgp_produces_multiple_plans() {
        // Two types share the same predicate "timestamp":
        //   Child  → timestamp: DateTime
        //   Sibling → timestamp: DateTime
        // Query: ?x timestamp ?ts  (ambiguous — matches both types)
        // Should produce 2 plans, one per candidate type.

        struct Child;
        struct Sibling;

        impl Resource for Child {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Child") }
            fn subject_iri(&self) -> Iri { unimplemented!() }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![FieldDescriptor {
                    predicate: Iri::new("http://example.org/timestamp"),
                    name: "timestamp",
                    field_type: FieldType::DateTime,
                    indexed: false,
                }]
            }
            fn field_values(&self) -> Vec<Term> { unimplemented!() }
        }

        impl Resource for Sibling {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Sibling") }
            fn subject_iri(&self) -> Iri { unimplemented!() }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![FieldDescriptor {
                    predicate: Iri::new("http://example.org/timestamp"),
                    name: "timestamp",
                    field_type: FieldType::DateTime,
                    indexed: false,
                }]
            }
            fn field_values(&self) -> Vec<Term> { unimplemented!() }
        }

        let mut schema = Schema::new();
        schema.register::<Child>();
        schema.register::<Sibling>();

        let query = crate::sparql::ast::SelectQuery {
            select: SelectClause::Variables(vec![Variable::new_unchecked("x"), Variable::new_unchecked("ts")]),
            where_pattern: GroupGraphPattern {
                patterns: vec![
                    TriplePattern {
                        subject: TermOrVariable::Variable(Variable::new_unchecked("x")),
                        predicate: TermOrVariable::Iri(Iri::new("http://example.org/timestamp")),
                        object: TermOrVariable::Variable(Variable::new_unchecked("ts")),
                    },
                ],
                filters: vec![],
            },
            modifier: SolutionModifier::default(),
            values: None,
        };

        let plans = lower(&query, &schema).unwrap();
        assert_eq!(plans.len(), 2, "expected 2 plans for ambiguous type, got {}", plans.len());

        // Each plan should have one Resource pattern with a different type.
        let plan_types: Vec<_> = plans.iter().map(|p| {
            assert_eq!(p.patterns.len(), 1);
            match &p.patterns[0] {
                QueryPattern::Resource { type_iri, .. } => type_iri.as_ref().unwrap().0.clone(),
                _ => panic!("expected Resource pattern"),
            }
        }).collect();

        assert!(plan_types.contains(&"http://example.org/Child".to_string()));
        assert!(plan_types.contains(&"http://example.org/Sibling".to_string()));
    }

    #[test]
    fn test_not_exists_rewrite_to_null_check() {
        // Schema: Item has "deprecated_by" field and "link" predicate.
        // Versioner has "is_next_version_of" which is a Reference to Item.
        // Rewrite: NOT EXISTS {[] isNextVersionOf ?x} on Item → deprecated_by IS NULL.
        use crate::schema::Resource;

        struct Item {
            _id: String,
        }
        impl Resource for Item {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Item") }
            fn subject_iri(&self) -> Iri { Iri::new(format!("http://example.org/item/{}", self._id)) }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/link"),
                        name: "link",
                        field_type: FieldType::String,
                        indexed: false,
                    },
                    FieldDescriptor {
                        predicate: Iri::new("http://example.org/deprecatedBy"),
                        name: "deprecated_by",
                        field_type: FieldType::Reference(vec![Iri::new("http://example.org/Item")]),
                        indexed: false,
                    },
                ]
            }
            fn field_values(&self) -> Vec<Term> { vec!["".into(), "".into()] }
        }

        struct Versioner {
            _id: String,
        }
        impl Resource for Versioner {
            fn rdf_type() -> Iri { Iri::new("http://example.org/Versioner") }
            fn subject_iri(&self) -> Iri { Iri::new(format!("http://example.org/ver/{}", self._id)) }
            fn field_descriptors() -> Vec<FieldDescriptor> {
                vec![FieldDescriptor {
                    predicate: Iri::new("http://example.org/isNextVersionOf"),
                    name: "is_next_version_of",
                    field_type: FieldType::Reference(vec![Iri::new("http://example.org/Item")]),
                    indexed: false,
                }]
            }
            fn field_values(&self) -> Vec<Term> { vec!["".into()] }
        }

        let mut schema = Schema::new();
        schema.register::<Item>();
        schema.register::<Versioner>();
        schema.register_not_exists_rewrite(
            Iri::new("http://example.org/isNextVersionOf"),
            Iri::new("http://example.org/Item"),
            vec!["deprecated_by"],
        );

        // Query: ?x ex:link ?l . FILTER NOT EXISTS {[] ex:isNextVersionOf ?x}
        let query = SelectQuery {
            select: SelectClause::Star,
            where_pattern: GroupGraphPattern {
                patterns: vec![TriplePattern {
                    subject: TermOrVariable::Variable(Variable::new_unchecked("x")),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/link")),
                    object: TermOrVariable::Variable(Variable::new_unchecked("l")),
                }],
                filters: vec![Filter::NotExists(GroupGraphPattern {
                    patterns: vec![TriplePattern {
                        subject: TermOrVariable::Variable(Variable::new_unchecked("__anon_0")),
                        predicate: TermOrVariable::Iri(Iri::new("http://example.org/isNextVersionOf")),
                        object: TermOrVariable::Variable(Variable::new_unchecked("x")),
                    }],
                    filters: vec![],
                })],
            },
            modifier: SolutionModifier::default(),
            values: None,
        };

        let plans = lower(&query, &schema).unwrap();
        assert_eq!(plans.len(), 1);
        let plan = &plans[0];

        // Should have no NOT EXISTS filters (rewritten away)
        assert!(plan.filters.is_empty(), "expected no NOT EXISTS filters, got {:?}", plan.filters);

        // Should have one null-check filter on "x" with field "deprecated_by"
        assert_eq!(plan.null_checks.len(), 1, "expected 1 null check, got {:?}", plan.null_checks);
        assert_eq!(plan.null_checks[0].variable, "x");
        assert_eq!(plan.null_checks[0].field_names, vec!["deprecated_by"]);
    }

    #[test]
    fn test_lower_property_path() {
        // Test that property path sequences are properly lowered
        // Property paths like ?a ex:p1/ex:p2 ?b desugar to:
        //   ?a ex:p1 ?__anon_0 .
        //   ?__anon_0 ex:p2 ?b .
        // These should be recognized as connected through the intermediate blank node

        let query = SelectQuery {
            select: SelectClause::Star,
            where_pattern: GroupGraphPattern {
                patterns: vec![
                    TriplePattern {
                        subject: TermOrVariable::Variable(Variable::new_unchecked("a")),
                        predicate: TermOrVariable::Iri(Iri::new("http://example.org/p1")),
                        object: TermOrVariable::Variable(Variable::new_unchecked("__anon_0")),
                    },
                    TriplePattern {
                        subject: TermOrVariable::Variable(Variable::new_unchecked("__anon_0")),
                        predicate: TermOrVariable::Iri(Iri::new("http://example.org/p2")),
                        object: TermOrVariable::Variable(Variable::new_unchecked("b")),
                    },
                ],
                filters: vec![],
            },
            modifier: SolutionModifier::default(),
            values: None,
        };

        let schema = test_schema();
        // This should not error with "disconnected patterns" despite the __anon_0 variable
        // because __anon_0 is included in connectivity checks
        let result = lower(&query, &schema);
        // It may fail for other reasons (unknown types) but should not fail for connectivity
        if let Err(DarqError::DisconnectedPatterns { .. }) = result {
            panic!("property path patterns should be recognized as connected");
        }
    }
}
