use std::cell::RefCell;

use postgres::types::Type;
use postgres::Client;

use crate::engine::sql::{SqlExecutor, SqlResultSet};
use crate::error::DarqError;

/// An [`SqlExecutor`] backed by a PostgreSQL connection.
pub struct PostgresExecutor {
    client: RefCell<Client>,
    sql_callback: Option<Box<dyn Fn(&str)>>,
}

impl PostgresExecutor {
    pub fn new(client: Client) -> Self {
        Self {
            client: RefCell::new(client),
            sql_callback: None,
        }
    }

    pub fn with_sql_callback(client: Client, callback: impl Fn(&str) + 'static) -> Self {
        Self {
            client: RefCell::new(client),
            sql_callback: Some(Box::new(callback)),
        }
    }
}

fn column_to_string(row: &postgres::Row, idx: usize, col_type: &Type) -> Option<String> {
    if *col_type == Type::INT8 {
        row.get::<_, Option<i64>>(idx).map(|v| v.to_string())
    } else if *col_type == Type::INT4 {
        row.get::<_, Option<i32>>(idx).map(|v| v.to_string())
    } else if *col_type == Type::FLOAT4 {
        row.get::<_, Option<f32>>(idx).map(|v| v.to_string())
    } else if *col_type == Type::FLOAT8 {
        row.get::<_, Option<f64>>(idx).map(|v| v.to_string())
    } else if *col_type == Type::BOOL {
        row.get::<_, Option<bool>>(idx)
            .map(|v| if v { "true" } else { "false" }.to_string())
    } else {
        row.get::<_, Option<String>>(idx)
    }
}

impl SqlExecutor for PostgresExecutor {
    fn execute_sql(&self, sql: &str) -> Result<SqlResultSet, DarqError> {
        if let Some(cb) = &self.sql_callback {
            cb(sql);
        }

        let mut client = self.client.borrow_mut();
        let rows = client
            .query(sql, &[])
            .map_err(|e| DarqError::SqlError(e.to_string()))?;

        if rows.is_empty() {
            return Ok(SqlResultSet {
                columns: Vec::new(),
                rows: Vec::new(),
            });
        }

        let columns: Vec<String> = rows[0]
            .columns()
            .iter()
            .map(|c| c.name().to_string())
            .collect();

        let col_types: Vec<Type> = rows[0]
            .columns()
            .iter()
            .map(|c| c.type_().clone())
            .collect();

        let result_rows: Vec<Vec<Option<String>>> = rows
            .iter()
            .map(|row| {
                (0..columns.len())
                    .map(|i| column_to_string(row, i, &col_types[i]))
                    .collect()
            })
            .collect();

        Ok(SqlResultSet {
            columns,
            rows: result_rows,
        })
    }
}
