use std::collections::{HashMap, HashSet};

use crate::rdf::{Float64, Iri, Literal, Term, RDF_TYPE};

/// What kind of value a field holds, following XSD type vocabulary.
#[derive(Debug, Clone)]
pub enum FieldType {
    String,
    StringArray,
    Integer,
    Boolean,
    Float,
    Double,
    Decimal,
    Date,
    DateTime,
    /// IRI reference to one of the listed resource types.
    Reference(Vec<Iri>),
    /// Array of ID references to one of the listed resource types.
    ReferenceArray(Vec<Iri>),
}

/// Describes one field on a Resource: its predicate IRI, Rust field name, and value type.
#[derive(Debug, Clone)]
pub struct FieldDescriptor {
    pub predicate: Iri,
    pub name: &'static str,
    pub field_type: FieldType,
    pub indexed: bool,
}

/// Implemented by any Rust type that can be stored and queried as a resource.
pub trait Resource {
    /// The rdf:type IRI for this resource (e.g. `<http://example.org/Person>`).
    fn rdf_type() -> Iri;

    /// The IRI that identifies this particular instance (the subject).
    fn subject_iri(&self) -> Iri;

    /// All field descriptors for this type. This is the static schema:
    /// it tells the system which predicates are valid.
    fn field_descriptors() -> Vec<FieldDescriptor>;

    /// Return the object Term for each field, in the same order as
    /// `field_descriptors()`.
    fn field_values(&self) -> Vec<Term>;

    /// The SQL table name for this resource type.
    /// Defaults to the local name of the `rdf_type()` IRI (after the last `#` or `/`).
    fn sql_table_name() -> String {
        let iri = Self::rdf_type();
        let s = &iri.0;
        if let Some(pos) = s.rfind('#') {
            s[pos + 1..].to_string()
        } else if let Some(pos) = s.rfind('/') {
            s[pos + 1..].to_string()
        } else {
            s.clone()
        }
    }
}

/// Static information about a registered resource type.
pub struct TypeInfo {
    pub type_iri: Iri,
    pub fields: Vec<FieldDescriptor>,
    pub table_name: String,
}

/// The schema knows every registered resource type and its fields.
/// Used to validate queries and map predicates to field names.
pub struct Schema {
    types: HashMap<Iri, TypeInfo>,
    predicate_to_types: HashMap<Iri, Vec<Iri>>,
    /// Registered rewrites for `FILTER NOT EXISTS {[] predicate ?var}` patterns.
    /// Key: (predicate IRI, target type IRI of ?var).
    /// Value: field names on the target type that must all be NULL.
    not_exists_rewrites: HashMap<(Iri, Iri), Vec<String>>,
    type_aliases: HashMap<Iri, Iri>,
    /// rdfs:subClassOf — direct parent classes of each class IRI.
    subclass_of: HashMap<Iri, HashSet<Iri>>,
}

impl Schema {
    pub fn new() -> Self {
        Schema {
            types: HashMap::new(),
            predicate_to_types: HashMap::new(),
            not_exists_rewrites: HashMap::new(),
            type_aliases: HashMap::new(),
            subclass_of: HashMap::new(),
        }
    }

    fn resolve_alias<'a>(&'a self, iri: &'a Iri) -> &'a Iri {
        self.type_aliases.get(iri).unwrap_or(iri)
    }

    pub fn register_type_alias(&mut self, alias: Iri, canonical: Iri) {
        self.type_aliases.insert(alias, canonical);
    }

    pub fn resolve_type(&self, iri: &Iri) -> Iri {
        self.type_aliases.get(iri).cloned().unwrap_or_else(|| iri.clone())
    }

    /// Register a Resource type and all its predicates.
    pub fn register<R: Resource>(&mut self) {
        let type_iri = R::rdf_type();
        let fields = R::field_descriptors();
        let table_name = R::sql_table_name();

        for fd in &fields {
            self.predicate_to_types
                .entry(fd.predicate.clone())
                .or_default()
                .push(type_iri.clone());
        }

        self.types.insert(
            type_iri.clone(),
            TypeInfo {
                type_iri,
                fields,
                table_name,
            },
        );
    }

    /// Check whether a predicate IRI is known.
    pub fn is_known_predicate(&self, pred: &Iri) -> bool {
        *pred == Iri::new(RDF_TYPE) || self.predicate_to_types.contains_key(pred)
    }

    /// Return all known predicate IRIs (including rdf:type).
    pub fn known_predicates(&self) -> HashSet<Iri> {
        let mut preds: HashSet<Iri> = self.predicate_to_types.keys().cloned().collect();
        preds.insert(Iri::new(RDF_TYPE));
        preds
    }

    /// Look up the field name for a predicate on a given type.
    pub fn field_name(&self, type_iri: &Iri, predicate: &Iri) -> Option<&str> {
        self.types.get(self.resolve_alias(type_iri)).and_then(|info| {
            info.fields
                .iter()
                .find(|fd| fd.predicate == *predicate)
                .map(|fd| fd.name)
        })
    }

    /// Look up the predicate IRI for a field name on a given type.
    pub fn predicate_for_field(&self, type_iri: &Iri, field_name: &str) -> Option<&Iri> {
        self.types.get(self.resolve_alias(type_iri)).and_then(|info| {
            info.fields
                .iter()
                .find(|fd| fd.name == field_name)
                .map(|fd| &fd.predicate)
        })
    }

    /// Return which types have a given predicate.
    pub fn types_for_predicate(&self, predicate: &Iri) -> &[Iri] {
        self.predicate_to_types
            .get(predicate)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Return the field descriptors for a type.
    pub fn fields_for_type(&self, type_iri: &Iri) -> Option<&[FieldDescriptor]> {
        self.types.get(self.resolve_alias(type_iri)).map(|info| info.fields.as_slice())
    }

    /// Return the SQL table name for a registered type.
    pub fn table_name(&self, type_iri: &Iri) -> Option<&str> {
        self.types.get(self.resolve_alias(type_iri)).map(|info| info.table_name.as_str())
    }

    /// Iterate over all registered type IRIs.
    pub fn known_types(&self) -> impl Iterator<Item = &Iri> {
        self.types.keys()
    }

    /// Return the target types for a specific type's field (Reference or ReferenceArray).
    /// Returns None if the field is not a reference type or is unknown.
    pub fn field_range_for_type(&self, type_iri: &Iri, predicate: &Iri) -> Option<&[Iri]> {
        self.types.get(self.resolve_alias(type_iri)).and_then(|info| {
            info.fields.iter().find(|fd| fd.predicate == *predicate).and_then(|fd| {
                match &fd.field_type {
                    FieldType::Reference(iris) | FieldType::ReferenceArray(iris) => Some(iris.as_slice()),
                    _ => None,
                }
            })
        })
    }

    /// Return the resource types that a reference-typed predicate can point to.
    /// Collects targets from all types that declare this predicate as a Reference.
    /// Returns an empty vec for literal fields or unknown predicates.
    pub fn range_types(&self, predicate: &Iri) -> Vec<Iri> {
        let mut targets = Vec::new();
        for info in self.types.values() {
            for fd in &info.fields {
                if fd.predicate == *predicate {
                    match &fd.field_type {
                        FieldType::Reference(iris) | FieldType::ReferenceArray(iris) => {
                            targets.extend(iris.iter().cloned());
                        }
                        _ => {}
                    }
                }
            }
        }
        targets
    }

    /// Register a rewrite rule: `FILTER NOT EXISTS {[] predicate ?var}` where
    /// `?var` has type `target_type` can be replaced by checking that the listed
    /// fields on the target type are all NULL.
    pub fn register_not_exists_rewrite(
        &mut self,
        predicate: Iri,
        target_type: Iri,
        null_fields: Vec<&'static str>,
    ) {
        self.not_exists_rewrites.insert(
            (predicate, target_type),
            null_fields.into_iter().map(String::from).collect(),
        );
    }

    /// Look up a NOT EXISTS rewrite for the given predicate and target type.
    /// Returns the field names that must all be NULL, or None if no rewrite is registered.
    pub fn not_exists_rewrite(&self, predicate: &Iri, target_type: &Iri) -> Option<&[String]> {
        self.not_exists_rewrites
            .get(&(predicate.clone(), target_type.clone()))
            .map(|v| v.as_slice())
    }

    /// Register an `rdfs:subClassOf` relationship: `child` is a direct subclass
    /// of `parent`. Multiple parents per child are allowed.
    pub fn register_subclass_of(&mut self, child: Iri, parent: Iri) {
        self.subclass_of.entry(child).or_default().insert(parent);
    }

    /// Direct parent classes of `iri` (one hop, not transitive).
    pub fn direct_superclasses(&self, iri: &Iri) -> Vec<Iri> {
        self.subclass_of
            .get(iri)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// All transitive ancestor classes of `iri` (excluding `iri` itself).
    pub fn ancestors_of(&self, iri: &Iri) -> HashSet<Iri> {
        let mut out = HashSet::new();
        let mut stack: Vec<Iri> = self.direct_superclasses(iri);
        while let Some(p) = stack.pop() {
            if out.insert(p.clone()) {
                stack.extend(self.direct_superclasses(&p));
            }
        }
        out
    }

    /// Direct child classes of `iri` (one hop, not transitive).
    pub fn direct_subclasses(&self, iri: &Iri) -> Vec<Iri> {
        self.subclass_of
            .iter()
            .filter_map(|(child, parents)| {
                if parents.contains(iri) {
                    Some(child.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// All transitive descendant classes of `iri`, INCLUDING `iri` itself.
    /// Useful for SPARQL `?s a SuperClass` semantics under RDFS entailment.
    pub fn descendants_inclusive(&self, iri: &Iri) -> HashSet<Iri> {
        let mut out = HashSet::new();
        out.insert(iri.clone());
        let mut stack: Vec<Iri> = self.direct_subclasses(iri);
        while let Some(c) = stack.pop() {
            if out.insert(c.clone()) {
                stack.extend(self.direct_subclasses(&c));
            }
        }
        out
    }

    /// Look up the canonical table type for `iri`: the registered table type
    /// itself if `iri` is one, the alias target if `iri` is aliased, or `None`.
    fn canonical_table(&self, iri: &Iri) -> Option<Iri> {
        if self.types.contains_key(iri) {
            return Some(iri.clone());
        }
        if let Some(target) = self.type_aliases.get(iri) {
            if self.types.contains_key(target) {
                return Some(target.clone());
            }
        }
        None
    }

    /// After all classes and `subClassOf` relationships are registered, walk
    /// every class and try to alias it to a registered table type, using two
    /// rules in priority order:
    ///
    /// 1. **Descendant rule:** if every concrete descendant of `C` (including
    ///    `C` itself) resolves to a single canonical table type, alias `C` to
    ///    that table. This handles superclasses such as
    ///    `cpmeta:DataObjectSpecifyingThing` only when its descendants are all
    ///    in one table, and subsumes the trivial case where `C` is itself a
    ///    table.
    /// 2. **Ancestor rule:** if `C` has no descendants that resolve to a
    ///    table, walk ancestors level-by-level. The first level that contains
    ///    one or more aliased/registered ancestor classes determines the
    ///    canonical table — provided all such ancestors at that level agree.
    ///    This makes leaf classes such as `cpmeta:AS` (subclass of
    ///    `cpmeta:IcosStation` ⊂ `cpmeta:Station`) resolve to `ct_stations`.
    ///
    /// Already-registered or already-aliased classes are left untouched.
    pub fn finalize_subclass_aliases(&mut self) {
        let mut all_classes: HashSet<Iri> = HashSet::new();
        for (child, parents) in &self.subclass_of {
            all_classes.insert(child.clone());
            all_classes.extend(parents.iter().cloned());
        }
        all_classes.extend(self.types.keys().cloned());
        all_classes.extend(self.type_aliases.keys().cloned());

        let mut to_alias: Vec<(Iri, Iri)> = Vec::new();
        for class in &all_classes {
            if self.types.contains_key(class) || self.type_aliases.contains_key(class) {
                continue;
            }

            // Descendant rule: union over descendants (incl. self).
            let mut tables: HashSet<Iri> = HashSet::new();
            for desc in self.descendants_inclusive(class) {
                if let Some(t) = self.canonical_table(&desc) {
                    tables.insert(t);
                }
            }
            if tables.len() == 1 {
                to_alias.push((class.clone(), tables.into_iter().next().unwrap()));
                continue;
            }
            if tables.len() > 1 {
                // Genuinely ambiguous: spans multiple tables.
                continue;
            }

            // Ancestor rule: BFS upward, level by level.
            let mut frontier = self.direct_superclasses(class);
            let mut visited: HashSet<Iri> = HashSet::new();
            visited.insert(class.clone());
            while !frontier.is_empty() {
                let level_tables: HashSet<Iri> = frontier
                    .iter()
                    .filter_map(|a| self.canonical_table(a))
                    .collect();
                if level_tables.len() == 1 {
                    to_alias.push((
                        class.clone(),
                        level_tables.into_iter().next().unwrap(),
                    ));
                    break;
                }
                if level_tables.len() > 1 {
                    break; // ambiguous
                }
                let mut next: Vec<Iri> = Vec::new();
                for a in frontier {
                    if visited.insert(a.clone()) {
                        next.extend(self.direct_superclasses(&a));
                    }
                }
                frontier = next;
            }
        }

        for (alias, canonical) in to_alias {
            self.type_aliases.insert(alias, canonical);
        }
    }
}

// Convenience constructors for Term from common Rust types.
impl From<&str> for Term {
    fn from(s: &str) -> Self {
        Term::Literal(Literal::String(s.to_string()))
    }
}

impl From<String> for Term {
    fn from(s: String) -> Self {
        Term::Literal(Literal::String(s))
    }
}

impl From<i64> for Term {
    fn from(n: i64) -> Self {
        Term::Literal(Literal::Integer(n))
    }
}

impl From<bool> for Term {
    fn from(b: bool) -> Self {
        Term::Literal(Literal::Boolean(b))
    }
}

impl From<f64> for Term {
    fn from(v: f64) -> Self {
        Term::Literal(Literal::Double(Float64(v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_schema_register() {
        let mut schema = Schema::new();
        schema.register::<Person>();

        assert!(schema.is_known_predicate(&Iri::new(RDF_TYPE)));
        assert!(schema.is_known_predicate(&Iri::new("http://example.org/name")));
        assert!(schema.is_known_predicate(&Iri::new("http://example.org/age")));
        assert!(!schema.is_known_predicate(&Iri::new("http://example.org/email")));
    }

    #[test]
    fn test_subclass_of_ancestors_descendants() {
        let mut schema = Schema::new();
        let a = Iri::new("urn:A");
        let b = Iri::new("urn:B");
        let c = Iri::new("urn:C");
        let d = Iri::new("urn:D");
        // D ⊂ C ⊂ B ⊂ A
        schema.register_subclass_of(d.clone(), c.clone());
        schema.register_subclass_of(c.clone(), b.clone());
        schema.register_subclass_of(b.clone(), a.clone());

        assert_eq!(schema.direct_superclasses(&d), vec![c.clone()]);
        let ancestors = schema.ancestors_of(&d);
        assert!(ancestors.contains(&a));
        assert!(ancestors.contains(&b));
        assert!(ancestors.contains(&c));
        assert!(!ancestors.contains(&d));

        let descendants = schema.descendants_inclusive(&a);
        assert!(descendants.contains(&a));
        assert!(descendants.contains(&b));
        assert!(descendants.contains(&c));
        assert!(descendants.contains(&d));
    }

    #[test]
    fn test_finalize_descendant_rule() {
        // Single-table hierarchy: every class in {A, B, C} should resolve to
        // the same table.
        struct Tbl;
        impl Resource for Tbl {
            fn rdf_type() -> Iri { Iri::new("urn:tbl") }
            fn subject_iri(&self) -> Iri { unreachable!() }
            fn field_descriptors() -> Vec<FieldDescriptor> { vec![] }
            fn field_values(&self) -> Vec<Term> { unreachable!() }
        }
        let mut schema = Schema::new();
        schema.register::<Tbl>();
        let a = Iri::new("urn:A");
        let b = Iri::new("urn:B");
        let c = Iri::new("urn:C");
        let tbl = Iri::new("urn:tbl");
        // A is the registered table; B ⊂ A; C ⊂ B.
        schema.register_type_alias(a.clone(), tbl.clone());
        schema.register_subclass_of(b.clone(), a.clone());
        schema.register_subclass_of(c.clone(), b.clone());

        schema.finalize_subclass_aliases();
        assert_eq!(schema.resolve_type(&a), tbl);
        assert_eq!(schema.resolve_type(&b), tbl);
        assert_eq!(schema.resolve_type(&c), tbl);
    }

    #[test]
    fn test_finalize_ambiguous_superclass_skipped() {
        struct T1;
        impl Resource for T1 {
            fn rdf_type() -> Iri { Iri::new("urn:t1") }
            fn subject_iri(&self) -> Iri { unreachable!() }
            fn field_descriptors() -> Vec<FieldDescriptor> { vec![] }
            fn field_values(&self) -> Vec<Term> { unreachable!() }
        }
        struct T2;
        impl Resource for T2 {
            fn rdf_type() -> Iri { Iri::new("urn:t2") }
            fn subject_iri(&self) -> Iri { unreachable!() }
            fn field_descriptors() -> Vec<FieldDescriptor> { vec![] }
            fn field_values(&self) -> Vec<Term> { unreachable!() }
        }
        let mut schema = Schema::new();
        schema.register::<T1>();
        schema.register::<T2>();
        let super_class = Iri::new("urn:Super");
        // T1 ⊂ Super, T2 ⊂ Super — Super spans two tables, should not alias.
        schema.register_subclass_of(Iri::new("urn:t1"), super_class.clone());
        schema.register_subclass_of(Iri::new("urn:t2"), super_class.clone());
        schema.finalize_subclass_aliases();

        // Super is not aliased to any single table.
        assert_eq!(schema.resolve_type(&super_class), super_class);
    }

    #[test]
    fn test_schema_type_lookups() {
        let mut schema = Schema::new();
        schema.register::<Person>();

        let person_type = Iri::new("http://example.org/Person");
        let name_pred = Iri::new("http://example.org/name");

        assert_eq!(schema.field_name(&person_type, &name_pred), Some("name"));
        assert_eq!(
            schema.predicate_for_field(&person_type, "name"),
            Some(&name_pred)
        );
        assert_eq!(schema.types_for_predicate(&name_pred), &[person_type.clone()]);
        assert!(schema.fields_for_type(&person_type).is_some());
        assert_eq!(schema.fields_for_type(&person_type).unwrap().len(), 2);
    }
}
