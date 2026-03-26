❯ I have heard that it is difficult to map SPARQL queries to a relational data model. What are the main reasons for this?                                                                                    

● This is directly relevant to darq's design. The main difficulties:

  1. Self-joins on the triple table. RDF stores data as (subject, predicate, object) triples. A single "entity" is spread across many rows. To reconstruct it, you need one self-join per predicate:

  ?person ex:name ?name .
  ?person ex:age ?age .

  becomes something like:

  SELECT t1.object, t2.object
  FROM triples t1
  JOIN triples t2 ON t1.subject = t2.subject
  WHERE t1.predicate = 'ex:name'
    AND t2.predicate = 'ex:age'

  With N predicates, that's N-1 self-joins on the same table. Relational optimizers aren't designed for this pattern.

  2. Variable predicates. SPARQL allows ?s ?p ?o — the predicate itself is unknown. In SQL, column names are fixed at query time. There's no natural way to express "any column" without UNIONing across all
  possible tables or querying a single triple table.

  3. Schema flexibility vs fixed columns. RDF is schema-less — different instances of the "same type" can have different predicates, and properties can be multi-valued (multiple foaf:knows values per
  person). Relational tables expect one value per column per row.

  4. OPTIONAL maps poorly to LEFT JOIN. SPARQL OPTIONAL looks like LEFT OUTER JOIN, but variables can be shared across multiple OPTIONAL blocks, creating correlated patterns that don't translate cleanly.

  5. Query planning is harder. In a triple table, every pattern hits the same table with the same schema. Traditional relational statistics (table sizes, column selectivity) don't help the optimizer
  distinguish a highly selective pattern from a broad one.

  This is exactly what darq sidesteps. By fixing the schema as Rust types, darq knows all valid predicates upfront — so it could, in the future, use property-table-style storage (one "table" per type, one
  "column" per field) instead of a flat triple table, and plan joins using knowledge of the data model. The "unknown predicates must error" rule is the foundation for that.

✻ Cooked for 31s

❯ How does darq deal with the situaiton of "?s ?p ?o"?                                                                                                                                                       

● It doesn't handle it well right now. Let me trace what happens.

  The schema validation in engine.rs only checks concrete predicates:

  Read 1 file (ctrl+o to expand)
  ⎿  You've hit your limit · resets 7pm (Europe/Stockholm)
     /upgrade or /extra-usage to finish what you’re working on.
  ⎿  SessionStart:resume hook error

❯ How does darq deal with the situaiton of "?s ?p ?o"?                                                                                                                                                       
  ⎿  API Error: Rate limit reached

❯ /model                                                                                                                                                                                                     
  ⎿  Set model to Opus 4.6

❯ How does darq deal with the situaiton of "?s ?p ?o"?                                                                                                                                                       

● Right, so to finish the thought — ?s ?p ?o with a variable predicate currently silently works because it bypasses validation (the predicate isn't a concrete IRI, so as_concrete_iri returns None) and the
  store's linear scan just matches everything.
                                                                                                                                                                                                             
  That gives correct results today, but it undermines the core design principle. The whole point of darq is that the schema is known — if you ever move to indexed storage (e.g. a property table per type), 
  a fully unbound ?p would need to scan every table and union the results, which is exactly the relational mapping problem we discussed.                                                                     
                                                                                                                                                                                                             
  There are a few options for how to handle it:                                                                                                                                                              
   
  1. Reject variable predicates — error at validation time. This is the strictest interpretation of "unknown predicates must error." If the predicate is a variable, we can't prove it's known, so reject it.
   Simple, safe, but restrictive.                                                                                                                                                                          
  2. Allow it but rewrite — expand ?s ?p ?o into a UNION over all known predicates internally. The schema knows every valid predicate, so this is a closed set. Correct, but potentially expensive.          
  3. Allow it on the flat store, gate it later — keep the current behavior for the Vec<Triple> store. If/when you add indexed storage, decide then whether to support or reject it. Pragmatic but defers the 
  decision.                                                                                                                                                                                                  
                                                                                                                                                                                                             
  Which direction feels right to you?                                                                                                                                                                        
                  
❯ Implement option 2, which means expanding it to every known predicate. Then, if another triple expression constrains the set, we can get a more efficient query.                                           
                                                                                                                                                                   
● Explore(Explore engine and schema for variable predicate rewrite)                                                                                                                                          
  ⎿  Done (4 tool uses · 27.3k tokens · 25s)
  (ctrl+o to expand)                                                                                                                                                                                         
                                  
● The exploration reveals the key issue: simply inlining N patterns into the flat Vec<TriplePattern> would create a conjunction (must match all), not a union (match any). The expansion needs to happen at  
  evaluation time inside evaluate_bgp, not as an AST rewrite.                                                                                                                                                
                                                                                                                                                                                                             
  Read 1 file (ctrl+o to expand)                                                                                                                                                                             
                                                                                                                                                                                                             
● This is the old initial plan. I'll overwrite it with the new task.                                                                                                                                         
                                                                                                                                                                                                             
● Updated plan                                                                                                                                                                                               
  ⎿  /plan to preview                                                                                                                                                                                        
                                                                                                                                                                                                             
─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
 Ready to code?                                                                                                                                                                                              
                                                                                                                                                                                                             
 Here is Claude's plan:
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
 Variable predicate expansion                           

 Context

 SPARQL allows variable predicates (?s ?p ?o). darq's design principle is that all valid predicates are known via the schema. Currently, variable predicates silently work because the store's linear scan
 matches everything — but this is implicit and won't survive a move to indexed storage. The fix: when evaluating a pattern with an unbound variable predicate, explicitly expand over all known predicates
 and union the results.

 Changes

 1. src/schema.rs — expose known predicates

 Add a public method to return all registered predicate IRIs:

 pub fn known_predicates(&self) -> &HashSet<Iri> {
     &self.known_predicates
 }

 2. src/engine.rs — expand variable predicates during BGP evaluation

 Modify evaluate_bgp to accept &Schema. When a triple pattern has a variable predicate that is not yet bound in the current solution:

 - Iterate over schema.known_predicates()
 - For each predicate, create a concrete store pattern with that predicate substituted in
 - Match against the store
 - Add the predicate variable binding (e.g. ?p -> ex:name) to each result
 - Collect all results as a union

 If ?p is already bound from a prior pattern, to_pattern_term already handles it — no change needed there.

 The key helper:

 fn is_unbound_predicate_var(tov: &TermOrVariable, binding: &Binding) -> Option<String> {
     if let TermOrVariable::Variable(Variable(name)) = tov {
         if !binding.contains_key(name) {
             return Some(name.clone());
         }
     }
     None
 }

 Update execute() to pass schema to evaluate_bgp.

 3. tests/integration.rs — add tests

 - ?s ?p ?o returns all triples with ?p bound to the actual predicate IRIs
 - ?s ?p ?o with a prior pattern constraining ?p uses the bound value

 Files to modify

 - src/schema.rs — add known_predicates() method
 - src/engine.rs — modify evaluate_bgp signature and variable-predicate handling
 - tests/integration.rs — add two tests

 Verification

 - cargo test — all existing 40 tests pass
 - New tests confirm variable predicate expansion produces correct bindings
