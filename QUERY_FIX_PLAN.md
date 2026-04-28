# Action Plan: Making Remaining SPARQL Queries Work

## Current Status
- **10 queries pass** (with 4 returning 0 rows due to data/lowering gaps)
- **13 queries fail** (10 main + 3 SITES)

The failures fall into 7 distinct categories, ordered by effort vs. impact.

---

## 1. Auto-inject well-known prefixes (LOW EFFORT, LOW IMPACT)

**Affects:** `SITES/locations.rq`, `SITES/samplingPoints.rq`

**Problem:** Both use `rdfs:label` without declaring `prefix rdfs:`. spargebra fails at parse time.

**Fix:** Pre-process query strings in `parser.rs::parse()` to prepend missing well-known prefixes (`rdfs`, `rdf`, `xsd`, `owl`) when their use is detected but they're undeclared.

```rust
// In parser.rs
const WELL_KNOWN: &[(&str, &str)] = &[
    ("rdfs:", "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>"),
    ("rdf:",  "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>"),
    ("xsd:",  "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>"),
    ("owl:",  "PREFIX owl: <http://www.w3.org/2002/07/owl#>"),
];
```

**Caveat:** Both queries also use `cpmeta:operatesOn` and `cpmeta:hasSamplingPoint` which aren't in our schema. Even with the prefix fix, they'll fail with "unknown predicate." So this is a parse-error → semantic-error step forward, not a passing query.

---

## 2. Implement UNION lowering + SQL generation (MEDIUM EFFORT, MEDIUM IMPACT)

**Affects:** `atcSpeciesAndHeights.rq` (currently runs but ignores UNION constraint, returns 0 rows)

**Status:** Parser done (commit `ec6648c`). Lowering and SQL generation are stubs.

**Fix:**
1. **IR:** Add `UnionGroup { left: Vec<QueryPattern>, right: Vec<QueryPattern> }` to `QueryPlan`
2. **Lower (`lower.rs`):** For each `(left_ggp, right_ggp)` in `ggp.unions`, lower both sides to `Vec<QueryPattern>`. Pass outer types so each branch can resolve.
3. **SQL (`sql.rs`):** Each union becomes a derived table joined to the outer query:
   ```sql
   INNER JOIN (
     SELECT ... FROM <left branch>
     UNION ALL
     SELECT ... FROM <right branch>
   ) AS "_un0" ON shared_var = "_un0".shared_var
   ```
   Variables present in only one branch get NULL in the other (use `SELECT col, NULL AS otherCol`).

**Tricky bit:** atcSpeciesAndHeights' UNION is inside an inner GROUP BY subquery; the union join must happen inside the subquery, before grouping.

---

## 3. Add missing ontology schemas (MEDIUM EFFORT, HIGH IMPACT)

These all hit "unknown predicate" — meaning we need to declare new resources/predicates in `cpmeta_schema.rs`.

### 3a. stationentry ontology — 5 queries
**Affects:** `etcLabelingValues`, `hoVsTc`, `inactivePis`, `labelingStatus`, `provStationPis`, `stationsTable`

Predicates needed: `hasLongName`, `hasShortName`, `hasProductionCounterpart`, `hasCountry`, plus likely a "ProvisionalStation" type.

**Step 1 (research):** Connect to the Postgres DB and check:
```sql
\dt ct_*  -- list ct_ tables
SELECT column_name FROM information_schema.columns WHERE table_name LIKE 'ct_%entry%';
```
Find the table that holds provisional/station-entry data.

**Step 2:** Add a `ProvStation` resource definition mirroring the actual table columns:
```rust
define_resource!(
    ProvStation, tbl!("ct_prov_stations"), [
        ("has_long_name",  cpst!("hasLongName"),  FieldType::String),
        ("has_short_name", cpst!("hasShortName"), FieldType::String),
        ("has_country",    cpst!("hasCountry"),   FieldType::String),
        ("has_production_counterpart", cpst!("hasProductionCounterpart"),
            ref_to!(tbl!("ct_stations"))),
        // ...
    ]
);
```

If the DB **doesn't have** a stationentry table at all, these queries are unrunnable — they target metadata that wasn't loaded into this Postgres dump.

### 3b. dcat ontology — 1 query
**Affects:** `dcat.rq`

`dcat:distribution` is unlikely to be in the cpmeta DB. Probably skip unless the DB has a `ct_distributions` table.

### 3c. ssn ontology — 1 query
**Affects:** `sensorsDeployments.rq`

`ssn:forProperty` likewise requires sensor metadata tables. Skip unless DB has them.

### 3d. sites ontology — 1 query
**Affects:** `SITES/stations.rq`

`<https://meta.fieldsites.se/ontologies/sites/Station>` is the **Swedish field sites** ontology, totally separate from ICOS Carbon Portal. The queries only run against a `meta.fieldsites.se` endpoint, not against the cpmeta DB. **Skip — wrong dataset entirely.**

---

## 4. Implement RDFS class hierarchy / `rdfs:subClassOf` (HIGH EFFORT, LOW IMPACT)

**Affects:** `provisionlessProdStations.rq`

```sparql
?stClass rdfs:subClassOf cpmeta:IcosStation .
?s a ?stClass ; cpmeta:hasStationClass [] .
```

**Fix path:**
1. Define a class hierarchy in the schema:
   ```rust
   schema.register_subclass(cpmeta!("AtmosphericStation"), cpmeta!("IcosStation"));
   schema.register_subclass(cpmeta!("EcosystemStation"),   cpmeta!("IcosStation"));
   // ...
   ```
2. In the lowerer: when seeing `?x rdfs:subClassOf <SuperType>`, expand `?x` into the union of registered subclasses (lower as multiple plans, one per subclass).

This query also needs the stationentry predicate `cpst:hasProductionCounterpart`, so it's blocked by **3a** first.

---

## 5. Class metadata table for `?type rdfs:label` (HIGH EFFORT, LOW IMPACT)

**Affects:** `prodStationPis.rq`'s `?station a/rdfs:label ?stTheme`

After property path desugaring, this becomes `?station rdf:type ?T . ?T rdfs:label ?stTheme`. The intermediate `?T` is a class IRI; we need its label.

**Fix paths (pick one):**
- **(a) Add a `_classes` virtual table:** Schema-level metadata table mapping each registered type IRI to its label. Generate SQL that joins against it.
- **(b) Pattern-detect `rdf:type ?T . ?T rdfs:label ?L`:** Replace at lowering time with a CASE expression keyed on the type column.

(b) is less invasive. Currently this query produces a 25-branch UNION ALL because the lowerer treats `?T` as ambiguous (every typed table has `rdfs:label`).

Result: enables prodStationPis to actually return rows (it currently runs with empty output).

---

## 6. Named graph variable binding (HIGH EFFORT, LOW IMPACT)

**Affects:** `resubmittedFiles.rq`

```sparql
graph ?rdfGraph { ... }
```

**Status:** Parser ignores graph names entirely. `?rdfGraph` is unbound, fails validation.

**Fix paths:**
- **Pragmatic:** Allow graph variables to bind to a constant placeholder (e.g., `'<default>'`) so SELECT validation passes and the column outputs a constant. Loses information but unblocks the query.
- **Correct:** Track graph provenance per-row (requires adding a graph column to every table — major schema change).

This query *also* uses property paths (`cpmeta:wasSubmittedBy/prov:endedAtTime`), but those already work. The blocker is purely the GRAPH variable.

---

## 7. Property path type inference (HIGH EFFORT, LOW IMPACT)

**Affects:** `geoFilter.rq` ("ambiguous type for ?dobj: candidates []")

The query has only property-path predicates on `?dobj`:
```sparql
?dobj cpmeta:wasSubmittedBy/prov:wasAssociatedWith ?submitter .
?dobj cpmeta:hasObjectSpec ?spec .
?dobj geo:sfIntersects/geo:asWKT "POLYGON(...)"^^geo:wktLiteral .
```

The path `wasSubmittedBy/wasAssociatedWith` desugars to a blank node intermediate, so the lowerer doesn't see `wasSubmittedBy` as a direct constraint on `?dobj` — it sees `wasSubmittedBy` going to an anonymous intermediate, which doesn't pin down `?dobj`'s type.

**Fix:** During property path desugaring, also record that the path's *first* predicate is a constraint on the original subject's type. The first predicate of `wasSubmittedBy/wasAssociatedWith` is `wasSubmittedBy`, which restricts `?dobj` to types that have it (e.g., `ct_static_objects`).

**Additional blocker:** The query uses `geo:sfIntersects` and `geo:asWKT` (geospatial functions) and `wktLiteral` typed literals. These need PostGIS support and full geospatial expression handling — out of scope.

---

## 8. Address 0-row results in passing queries

These queries succeed but return 0 rows — likely a data/lowering issue worth investigating:

- **stationRoles, prodStationPis:** Verified that `at_organization` FK values in `ct_memberships` don't match any `ct_stations.id`. This is a **data layer issue** — the integer FKs in this Postgres dump connect memberships to organizations, not directly to stations. Possibly ICOS uses a join-through pattern (memberships → organizations → station-of-organization) or stations and organizations share IRIs but not integer IDs. **Action:** investigate whether `ct_organizations` has a `station_id` column or similar bridge.

- **atcSpeciesAndHeights:** 0 rows because UNION isn't lowered (item 2). Once lowered, should produce data.

- **concaveHulls:** Likely correct (no SOCAT polygon data in this dump).

---

## Recommended order

1. **(2) UNION lowering** — high-value, self-contained, no DB dependencies
2. **(1) Auto-prefix injection** — trivial, unblocks parse errors
3. **(8) Investigate `at_organization` data layout** — may unblock stationRoles + prodStationPis without code
4. **(3a) stationentry schema** — only after confirming the tables exist in this DB; would unblock 5 queries at once
5. **(5) Class label pattern** — small win for prodStationPis quality
6. **(4) subClassOf** — depends on (3a)
7. Items (6), (7) and remaining ontologies — diminishing returns

The blunt reality: **the realistic ceiling is ~16/23 queries** without adding new database tables. Items (3b), (3c), (3d), (6), and (7)'s geo features represent fundamental data-model gaps in this Postgres dump that no amount of compiler work can fix.
