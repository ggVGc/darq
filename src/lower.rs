use std::collections::HashMap;

use crate::error::DarqError;
use crate::ir::{FieldConstraint, QueryPattern, QueryPlan, Subject, Value};
use crate::rdf::{Iri, Literal, Term, RDF_TYPE};
use crate::schema::Schema;
use crate::sparql::ast::*;

/// Lower a parsed (and prefix-expanded, predicate-validated) SPARQL query
/// into a resource-level query plan.
pub fn lower(query: &SelectQuery, schema: &Schema) -> Result<QueryPlan, DarqError> {
    let patterns = lower_bgp(&query.where_pattern, schema)?;
    Ok(QueryPlan {
        patterns,
        select: query.select.clone(),
        modifier: query.modifier.clone(),
    })
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
        if let (TermOrVariable::Iri(pred_iri), TermOrVariable::Variable(Variable(obj_name))) =
            (&tp.predicate, &tp.object)
        {
            let range = schema.range_types(pred_iri);
            if !range.is_empty() {
                let entry = constraints.entry(obj_name.clone()).or_default();
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
    Literal(AstLiteral),
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
        TermOrVariable::Variable(Variable(name)) => SubjectKey::Variable(name.clone()),
        TermOrVariable::Iri(iri) => SubjectKey::Iri(iri.clone()),
        TermOrVariable::RdfType => SubjectKey::Iri(Iri::new(RDF_TYPE)),
        _ => unreachable!("literals and prefixed names cannot appear in subject position"),
    }
}

fn predicate_kind(tov: &TermOrVariable) -> PredicateKind {
    match tov {
        TermOrVariable::RdfType => PredicateKind::RdfType,
        TermOrVariable::Iri(iri) if iri.0 == RDF_TYPE => PredicateKind::RdfType,
        TermOrVariable::Iri(iri) => PredicateKind::Concrete(iri.clone()),
        TermOrVariable::Variable(Variable(name)) => PredicateKind::Variable(name.clone()),
        _ => unreachable!("literals and prefixed names cannot appear in predicate position"),
    }
}

fn object_info(tov: &TermOrVariable) -> ObjectInfo {
    match tov {
        TermOrVariable::Variable(Variable(name)) => ObjectInfo::Variable(name.clone()),
        TermOrVariable::Iri(iri) => ObjectInfo::Iri(iri.clone()),
        TermOrVariable::RdfType => ObjectInfo::Iri(Iri::new(RDF_TYPE)),
        TermOrVariable::Literal(lit) => ObjectInfo::Literal(lit.clone()),
        TermOrVariable::PrefixedName { .. } => {
            unreachable!("prefixed names should be expanded before lowering")
        }
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
        ObjectInfo::Literal(lit) => Value::Bound(ast_lit_to_term(lit)),
    }
}

fn ast_lit_to_term(lit: &AstLiteral) -> Term {
    match lit {
        AstLiteral::String(s) => Term::Literal(Literal::String(s.clone())),
        AstLiteral::Integer(n) => Term::Literal(Literal::Integer(*n)),
        AstLiteral::Boolean(b) => Term::Literal(Literal::Boolean(*b)),
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
                    subject: TermOrVariable::Variable(Variable("p".into())),
                    predicate: TermOrVariable::RdfType,
                    object: TermOrVariable::Iri(Iri::new("http://example.org/Person")),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable("p".into())),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Variable(Variable("name".into())),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable("p".into())),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/age")),
                    object: TermOrVariable::Variable(Variable("age".into())),
                },
            ],
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
                subject: TermOrVariable::Variable(Variable("p".into())),
                predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                object: TermOrVariable::Variable(Variable("name".into())),
            }],
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
                subject: TermOrVariable::Variable(Variable("s".into())),
                predicate: TermOrVariable::Variable(Variable("p".into())),
                object: TermOrVariable::Variable(Variable("o".into())),
            }],
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
                    subject: TermOrVariable::Variable(Variable("person".into())),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Variable(Variable("name".into())),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable("person".into())),
                    predicate: TermOrVariable::Variable(Variable("p".into())),
                    object: TermOrVariable::Variable(Variable("o".into())),
                },
            ],
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
                    subject: TermOrVariable::Variable(Variable("s".into())),
                    predicate: TermOrVariable::RdfType,
                    object: TermOrVariable::Variable(Variable("type".into())),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable("s".into())),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Variable(Variable("name".into())),
                },
            ],
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
                subject: TermOrVariable::Variable(Variable("s".into())),
                predicate: TermOrVariable::RdfType,
                object: TermOrVariable::Iri(Iri::new("http://example.org/Unknown")),
            }],
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
                subject: TermOrVariable::Variable(Variable("p".into())),
                predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                object: TermOrVariable::Literal(AstLiteral::String("Alice".into())),
            }],
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
                subject: TermOrVariable::Variable(Variable("t".into())),
                predicate: TermOrVariable::Iri(Iri::new("http://example.org/owner")),
                object: TermOrVariable::Iri(Iri::new("http://example.org/person/bob")),
            }],
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
                object: TermOrVariable::Variable(Variable("name".into())),
            }],
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
                    subject: TermOrVariable::Variable(Variable("p".into())),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Variable(Variable("name".into())),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable("q".into())),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/age")),
                    object: TermOrVariable::Variable(Variable("age".into())),
                },
            ],
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
                    subject: TermOrVariable::Variable(Variable("p".into())),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Variable(Variable("name".into())),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable("q".into())),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/age")),
                    object: TermOrVariable::Variable(Variable("age".into())),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable("p".into())),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/age")),
                    object: TermOrVariable::Variable(Variable("a".into())),
                },
            ],
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
                subject: TermOrVariable::Variable(Variable("p".into())),
                predicate: TermOrVariable::RdfType,
                object: TermOrVariable::Iri(Iri::new("http://example.org/Person")),
            }],
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
                subject: TermOrVariable::Variable(Variable("s".into())),
                predicate: TermOrVariable::RdfType,
                object: TermOrVariable::Variable(Variable("type".into())),
            }],
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
                subject: TermOrVariable::Variable(Variable("s".into())),
                predicate: TermOrVariable::Iri(Iri::new(RDF_TYPE)),
                object: TermOrVariable::Iri(Iri::new("http://example.org/Person")),
            }],
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
                subject: TermOrVariable::Variable(Variable("s".into())),
                predicate: TermOrVariable::Variable(Variable("p".into())),
                object: TermOrVariable::Literal(AstLiteral::String("Alice".into())),
            }],
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
                    subject: TermOrVariable::Variable(Variable("p".into())),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Literal(AstLiteral::String("Alice".into())),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable("p".into())),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/name")),
                    object: TermOrVariable::Variable(Variable("name".into())),
                },
            ],
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

        let bgp = GroupGraphPattern { patterns: vec![] };
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
                    subject: TermOrVariable::Variable(Variable("parent".into())),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/hasChild")),
                    object: TermOrVariable::Variable(Variable("child".into())),
                },
                TriplePattern {
                    subject: TermOrVariable::Variable(Variable("child".into())),
                    predicate: TermOrVariable::Iri(Iri::new("http://example.org/timestamp")),
                    object: TermOrVariable::Variable(Variable("ts".into())),
                },
            ],
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
}
