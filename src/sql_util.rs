use crate::rdf::Iri;
use crate::schema::{FieldDescriptor, FieldType, Schema};

/// Extract the local name from an IRI (the part after the last `#` or `/`).
pub fn iri_local_name(iri: &Iri) -> &str {
    let s = &iri.0;
    if let Some(pos) = s.rfind('#') {
        &s[pos + 1..]
    } else if let Some(pos) = s.rfind('/') {
        &s[pos + 1..]
    } else {
        s
    }
}

/// Resolve the SQL table name for a type IRI: registered name if available, else IRI local name.
pub fn resolve_table_name<'a>(schema: &'a Schema, type_iri: &'a Iri) -> &'a str {
    schema
        .table_name(type_iri)
        .unwrap_or_else(|| iri_local_name(type_iri))
}

/// Check if a field is an array type (StringArray or ReferenceArray).
pub fn is_array_field(fields: &[FieldDescriptor], name: &str) -> bool {
    fields.iter().any(|f| {
        f.name == name
            && matches!(
                f.field_type,
                FieldType::StringArray | FieldType::ReferenceArray(_)
            )
    })
}

/// Check if a field is a Reference type.
pub fn is_ref_field(fields: &[FieldDescriptor], name: &str) -> bool {
    fields
        .iter()
        .any(|f| f.name == name && matches!(f.field_type, FieldType::Reference(_)))
}

/// Generate a SQL comparison for a field value, handling array fields with `= ANY()`.
///
/// For arrays: `{value_sql} = ANY("{alias}"."{field_name}")`
/// For scalars: `"{alias}"."{field_name}" = {value_sql}`
pub fn field_comparison_sql(alias: &str, field_name: &str, value_sql: &str, is_array: bool) -> String {
    if is_array {
        format!(
            "{} = ANY(\"{}\".\"{}\")",
            value_sql, alias, field_name
        )
    } else {
        format!(
            "\"{}\".\"{}\" = {}",
            alias, field_name, value_sql
        )
    }
}

/// Append a table source to the FROM clause with appropriate join type.
///
/// - First table: `{source} AS "{alias}"`, join_conds go to where_parts.
/// - No join conditions: `CROSS JOIN {source} AS "{alias}"`.
/// - With join conditions: `INNER JOIN {source} AS "{alias}" ON {conds}`.
pub fn assemble_from_join(
    from_parts: &mut Vec<String>,
    where_parts: &mut Vec<String>,
    source: &str,
    alias: &str,
    join_conds: Vec<String>,
) {
    if from_parts.is_empty() {
        from_parts.push(format!("{} AS \"{}\"", source, alias));
        where_parts.extend(join_conds);
    } else if join_conds.is_empty() {
        from_parts.push(format!("CROSS JOIN {} AS \"{}\"", source, alias));
    } else {
        from_parts.push(format!(
            "INNER JOIN {} AS \"{}\" ON {}",
            source,
            alias,
            join_conds.join(" AND ")
        ));
    }
}

/// Output of building reference LEFT JOINs for IRI resolution.
pub struct RefJoinResult {
    pub join_clauses: Vec<String>,
    pub select_expr: String,
}

/// Build LEFT JOIN clauses to resolve a foreign-key reference field to full IRIs.
///
/// For single-target references, produces a simple LEFT JOIN.
/// For multi-target references, produces multiple LEFT JOINs with COALESCE.
pub fn build_ref_left_joins(
    schema: &Schema,
    target_types: &[Iri],
    fk_expr: &str,
    id_column: &str,
    subject_column: &str,
    alias_prefix: &str,
    alias_start: usize,
) -> RefJoinResult {
    if target_types.len() == 1 {
        let target_table = resolve_table_name(schema, &target_types[0]);
        let ref_alias = format!("{}{}", alias_prefix, alias_start);
        let join = format!(
            "LEFT JOIN \"{}\" AS \"{}\" ON \"{}\".\"{}\" = {}",
            target_table, ref_alias, ref_alias, id_column, fk_expr
        );
        RefJoinResult {
            join_clauses: vec![join],
            select_expr: format!("\"{}\".\"{}\"", ref_alias, subject_column),
        }
    } else {
        let mut join_clauses = Vec::new();
        let mut coalesce_parts = Vec::new();
        for (i, target_type) in target_types.iter().enumerate() {
            let target_table = resolve_table_name(schema, target_type);
            let ref_alias = format!("{}{}", alias_prefix, alias_start + i);
            join_clauses.push(format!(
                "LEFT JOIN \"{}\" AS \"{}\" ON \"{}\".\"{}\" = {}",
                target_table, ref_alias, ref_alias, id_column, fk_expr
            ));
            coalesce_parts.push(format!("\"{}\".\"{}\"", ref_alias, subject_column));
        }
        RefJoinResult {
            join_clauses,
            select_expr: format!("COALESCE({})", coalesce_parts.join(", ")),
        }
    }
}
