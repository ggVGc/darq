 Context

 darq is a SPARQL endpoint where the data model is defined as Rust types. Fields on those types map to RDF predicates, so all valid predicates are known upfront. Queries referencing unknown predicates
 error immediately. The initial scope is SELECT queries with basic graph patterns only — no filtering, no named graphs, no OPTIONAL/UNION.

 Module Structure

 darq/
   Cargo.toml          (nom as only non-std dependency)
   src/
     lib.rs             re-exports, top-level execute()
     rdf.rs             core RDF types
     schema.rs          Resource trait + Schema registry
     store.rs           TripleStore with pattern matching
     sparql/
       mod.rs           re-exports
       ast.rs           SPARQL AST types
       parser.rs        nom-based parser
     engine.rs          query evaluation: AST + Store -> results
     error.rs           DarqError enum
   tests/
     integration.rs     end-to-end test with Person example

 Phase 1: RDF types + error type

 Files: src/rdf.rs, src/error.rs, src/lib.rs, Cargo.toml

 rdf.rs — Core types with no dependencies:
 - Iri(String) — full IRI, derives Clone/Eq/Hash/Ord
 - Literal enum — String(String), Integer(i64), Boolean(bool)
 - Term enum — Iri(Iri), Literal(Literal) (no blank nodes in v1)
 - Triple { subject: Term, predicate: Iri, object: Term } — predicate is always an IRI

 error.rs:
 - DarqError enum: ParseError(String), UnknownPrefix(String), UnknownPredicate(Iri), InternalError(String)

 Phase 2: Schema + Store

 Files: src/schema.rs, src/store.rs

 schema.rs — The Resource trait defines how Rust types become RDF:
 - fn rdf_type() -> Iri — the rdf:type IRI for this type
 - fn subject_iri(&self) -> Iri — the subject IRI for this instance
 - fn field_descriptors() -> Vec<FieldDescriptor> — static list of { predicate: Iri, name: &'static str }
 - fn field_values(&self) -> Vec<Term> — object terms, positionally matched to descriptors
 - fn to_triples(&self) -> Vec<Triple> — default impl emits rdf:type triple + one triple per field

 Schema struct:
 - register::<R: Resource>() — records all predicates from R::field_descriptors()
 - is_known_predicate(&self, pred: &Iri) -> bool
 - validate_query(&self, query: &SelectQuery) -> Result<(), DarqError> — rdf:type is always valid

 store.rs — Simple Vec<Triple> store:
 - load<R: Resource>(&mut self, resource: &R) — calls to_triples() and appends
 - match_pattern(&self, pattern: &TriplePattern) -> Vec<Binding> — linear scan, binds variables

 Uses PatternTerm enum (Bound(Term) | Variable(String)) — separate from AST types since these are post-expansion.

 Phase 3: SPARQL Parser

 Files: src/sparql/ast.rs, src/sparql/parser.rs, src/sparql/mod.rs

 Parser library: nom — mature, well-suited for this grammar size.

 AST types (in ast.rs):
 - SelectQuery { prefixes, base, select, where_pattern, modifier }
 - PrefixDecl { prefix: String, iri: Iri }
 - SelectClause — Variables(Vec<Variable>) | Star
 - Variable(String) — name without ?/$
 - GroupGraphPattern { patterns: Vec<TriplePattern> }
 - TriplePattern { subject, predicate, object } using TermOrVariable enum
 - TermOrVariable — Variable | Iri | PrefixedName { prefix, local } | RdfType | Literal
 - SolutionModifier { distinct, order_by, limit, offset }

 SPARQL subset parsed (grammar productions):
 - [4] Prologue — PREFIX and BASE declarations
 - [9] SelectClause — SELECT with variable list or *, optional DISTINCT
 - [17] WhereClause — optional WHERE keyword + GroupGraphPattern
 - [54] GroupGraphPattern — { TriplesBlock }
 - [56] TriplesBlock — TriplesSameSubjectPath separated by .
 - [77] TriplesSameSubjectPath — subject + property list (; for multiple predicates, , for multiple objects)
 - [76] Verb — IRI or a keyword
 - [23-27] SolutionModifier — ORDER BY, LIMIT, OFFSET

 Prefix expansion: Separate pass after parsing. Walks AST, replaces PrefixedName with Iri. Errors on undeclared prefixes.

 Key terminal parsers: IRI_REF (<...>), PNAME_LN/PNAME_NS (prefixed names), VAR1/VAR2 (?x/$x), INTEGER, STRING_LITERAL2 ("..."), BooleanLiteral.

 Phase 4: Query Engine

 File: src/engine.rs

 Top-level execute(query_str, schema, store) -> Result<QueryResult, DarqError>:
 1. Parse query string into AST
 2. Expand prefixes
 3. Validate all predicates against schema
 4. Evaluate basic graph pattern (nested-loop join)
 5. Apply solution modifiers (DISTINCT, ORDER BY, OFFSET, LIMIT)
 6. Project to selected variables

 BGP evaluation — nested-loop join:
 - Start with vec![empty_binding]
 - For each triple pattern: substitute already-bound variables, match against store, merge compatible bindings
 - O(n^k) worst case — acceptable per simplicity constraint

 QueryResult: { variables: Vec<Variable>, rows: Vec<Vec<Option<Term>>> }

 Phase 5: Integration Test

 File: tests/integration.rs

 Define a Person struct implementing Resource with name and age fields mapped to ex:name and ex:age. Load Alice and Bob. Run queries:
 1. SELECT * WHERE { ?p a ex:Person . ?p ex:name ?n . } — basic pattern
 2. SELECT ?name WHERE { ... } ORDER BY ?name LIMIT 1 — modifiers
 3. Query with unknown predicate — expect DarqError::UnknownPredicate

 Implementation Order

 1. Cargo.toml + src/lib.rs + src/error.rs + src/rdf.rs
 2. src/schema.rs + src/store.rs
 3. src/sparql/ast.rs + src/sparql/parser.rs + src/sparql/mod.rs
 4. src/engine.rs
 5. tests/integration.rs

 Each phase should compile and have passing tests before moving to the next.

 Verification

 - cargo build compiles cleanly at each phase
 - cargo test passes at each phase
 - Integration test demonstrates: data loading, query parsing, pattern matching, predicate validation, solution modifiers
