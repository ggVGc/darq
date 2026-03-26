Plan: Replace triples with resource-level IR

 Context

 Currently, darq's internal representation and storage both use RDF triples. The Resource trait converts Rust structs into triples, the TripleStore stores Vec<Triple>, and the engine evaluates SPARQL by
 pattern-matching against raw triples. The user wants triples to exist only at the SPARQL parsing level. After parsing, the system should work in terms of Resources and their fields — matching how the
 data is actually modeled.

 The goal: a SPARQL query like ?p a ex:Person . ?p ex:name ?name . ?p ex:age 30 should lower into an IR that says "find Person instances where age=30, bind name to ?name" — not three separate
 triple-pattern matches.

 New Pipeline

 SPARQL string
   → parse (unchanged)
   → expand prefixes (unchanged)
   → validate predicates (unchanged)
   → lower (NEW: AST triple patterns → resource-level IR)
   → evaluate (REWRITTEN: IR + ResourceStore → Bindings)
   → apply modifiers (unchanged)
   → project (updated: collects variables from IR)
   → QueryResult

 IR Design (src/ir.rs — new file)

 pub enum Subject {
     Variable(String),
     Bound(Iri),
 }

 pub enum Value {
     Variable(String),
     Bound(Term),
 }

 pub struct FieldConstraint {
     pub field_name: String,
     pub value: Value,
 }

 pub enum QueryPattern {
     /// Match instances of a resource type where all field constraints hold.
     /// Groups what were previously multiple triple patterns into one check.
     Resource {
         subject: Subject,
         type_iri: Option<Iri>,  // None = scan all types (for `?s a ?type`)
         constraints: Vec<FieldConstraint>,
         type_variable: Option<String>,  // binds type IRI (from `?s a ?type`)
     },

     /// Iterate all fields of matching resources, one row per field.
     /// Used for variable-predicate patterns like `?s ?p ?o`.
     /// Includes synthetic rdf:type field in iteration.
     FieldScan {
         subject: Subject,
         predicate_var: String,
         object: Value,
         type_iri: Option<Iri>,  // None = scan all types
     },
 }

 pub struct QueryPlan {
     pub patterns: Vec<QueryPattern>,
     pub select: SelectClause,
     pub modifier: SolutionModifier,
 }

 QueryPlan also provides collect_variables() -> Vec<String> for SELECT *, preserving first-appearance order.

 Lowering Algorithm (src/lower.rs — new file)

 pub fn lower(query: &SelectQuery, schema: &Schema) -> Result<QueryPlan, DarqError>

 1. Group AST triple patterns by subject (variable name or bound IRI).
 2. For each group, classify patterns:
   - a <Type> (concrete type assertion) → determines type_iri
   - a ?type (type variable) → sets type_variable
   - <predicate> obj (concrete predicate) → becomes FieldConstraint
   - ?pred obj (variable predicate) → becomes FieldScan
 3. Determine type: explicit a <Type>, or inferred from concrete predicates via schema.types_for_predicate() (intersect across all predicates — must yield exactly one type).
 4. Map predicates to field names using schema.field_name(type_iri, predicate).
 5. Emit one Resource pattern per group (if any concrete constraints or type assertion), plus one FieldScan per variable-predicate pattern.
 6. Order patterns by first appearance of each subject in the original query.

 Enhanced Schema (src/schema.rs — modify)

 Replace known_predicates: HashSet<Iri> with:
 struct TypeInfo {
     type_iri: Iri,
     fields: Vec<FieldDescriptor>,
 }

 struct Schema {
     types: HashMap<Iri, TypeInfo>,
     predicate_to_types: HashMap<Iri, Vec<Iri>>,
 }

 New methods: field_name(type_iri, predicate), predicate_for_field(type_iri, field_name), types_for_predicate(predicate), fields_for_type(type_iri), known_types().

 Derive is_known_predicate() from predicate_to_types.contains_key().

 Remove to_triples() from the Resource trait (at the end, after migration).

 ResourceStore (src/resource_store.rs — new file)

 struct ResourceInstance {
     type_iri: Iri,
     subject: Iri,
     fields: HashMap<String, Term>,
 }

 struct ResourceStore {
     by_type: HashMap<Iri, Vec<ResourceInstance>>,
     by_subject: HashMap<Iri, (Iri, usize)>,  // subject → (type_iri, index)
 }

 - load<R: Resource>() builds ResourceInstance directly from field_descriptors() + field_values() — no triple intermediary.
 - instances_of(type_iri), find_by_subject(subject), all_types().

 Engine Evaluation (src/engine.rs — modify)

 New evaluate_plan() uses nested-loop join over QueryPatterns:

 - Resource: iterate instances of the type (or all types if None). For each, check subject binding, check all field constraints, optionally bind type_variable.
 - FieldScan: iterate types (restricted or all), iterate instances, iterate fields + synthetic rdf:type. For each field, check/bind subject, predicate_var, object. FieldScan looks up predicate IRI from
 Schema via predicate_for_field(type_iri, field_name).

 Error Changes (src/error.rs)

 Add: UnknownType(Iri), AmbiguousType { subject: String, candidates: Vec<Iri> }.

 What Gets Removed (final step)

 - rdf::Triple struct
 - Resource::to_triples() default method
 - src/store.rs entirely (TripleStore, IriPattern, TermPattern, store::TriplePattern)
 - Old execute() function and all triple-level evaluation code in engine

 Implementation Order

 Step 1: Enhance Schema

 Modify src/schema.rs — add TypeInfo, change internal storage to types + predicate_to_types maps, add new lookup methods. Keep old API (is_known_predicate, known_predicates) working.

 Step 2: Create IR types

 New src/ir.rs — Subject, Value, FieldConstraint, QueryPattern, QueryPlan, collect_variables(). Add to lib.rs.

 Step 3: Implement lowering

 New src/lower.rs — the lowering algorithm. Add new error variants. Add to lib.rs. Unit test with hand-built AST nodes.

 Step 4: Create ResourceStore

 New src/resource_store.rs — ResourceInstance, ResourceStore, load/query methods. Add to lib.rs. Unit test independently.

 Step 5: Implement IR evaluator

 Add evaluate_plan() to src/engine.rs. The nested-loop join over QueryPatterns against ResourceStore.

 Step 6: Wire up new pipeline

 Add execute overload using Schema + ResourceStore. Parse → expand → validate → lower → evaluate_plan → modifiers → project.

 Step 7: Migrate tests and examples

 Update integration tests and examples to use ResourceStore. Verify identical results.

 Step 8: Remove old code

 Delete store.rs, Triple, to_triples(), old execute(), rename new entry point to execute().

 Verification

 After each step, run cargo test. After step 7, all 42 existing tests should produce identical results against the new pipeline. Key cases to verify:
 - Variable predicate expansion (?s ?p ?o → 9 rows via FieldScan)
 - Cross-pattern joins (shared variables across ResourcePatterns)
 - Multiple constraints on same field (conjunctive)
 - Type inference from predicates (no explicit a <Type>)
 - Bound subject IRI patterns
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
