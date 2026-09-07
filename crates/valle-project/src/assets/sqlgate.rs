//! Read-only SQL access enforced by both the connection and SQLite authorizer. Reject writes,
//! attachment, and mutating pragmas without relying on statement-prefix checks.

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use serde::Serialize;
use serde_json::Value;

use crate::assets::ctx::Ctx;
use crate::assets::db::Db;
use crate::assets::report::{AssetsError, Result};

/// Maximum number of returned rows.
const MAX_ROWS: usize = 1000;

#[derive(Debug, Serialize)]
pub struct SqlOutcome {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// Whether the row limit truncated the results.
    pub truncated: bool,
}

pub fn sql(ctx: &Ctx, query: &str) -> Result<SqlOutcome> {
    let db = Db::open_readonly(&ctx.home)?;
    db.conn
        .authorizer(Some(|c: AuthContext<'_>| match c.action {
            AuthAction::Select
            | AuthAction::Read { .. }
            | AuthAction::Function { .. }
            | AuthAction::Recursive => Authorization::Allow,
            _ => Authorization::Deny,
        }))?;
    let mut stmt = db.conn.prepare(query).map_err(|e| {
        AssetsError::bad_query(format!("SQL rejected: {e}"))
            .with_hint("SQL access is read-only; use commands for mutations")
    })?;
    let columns: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let ncols = columns.len();
    let mut rows_iter = stmt
        .query([])
        .map_err(|e| AssetsError::bad_query(format!("SQL execution failed: {e}")))?;
    let mut rows = Vec::new();
    let mut truncated = false;
    while let Some(row) = rows_iter
        .next()
        .map_err(|e| AssetsError::bad_query(format!("SQL execution failed: {e}")))?
    {
        if rows.len() >= MAX_ROWS {
            truncated = true;
            break;
        }
        let mut out = Vec::with_capacity(ncols);
        for i in 0..ncols {
            let v = match row.get_ref(i) {
                Ok(rusqlite::types::ValueRef::Null) => Value::Null,
                Ok(rusqlite::types::ValueRef::Integer(n)) => Value::from(n),
                Ok(rusqlite::types::ValueRef::Real(f)) => Value::from(f),
                Ok(rusqlite::types::ValueRef::Text(t)) => {
                    Value::from(String::from_utf8_lossy(t).into_owned())
                }
                Ok(rusqlite::types::ValueRef::Blob(b)) => Value::from(format!("blob({})", b.len())),
                Err(_) => Value::Null,
            };
            out.push(v);
        }
        rows.push(out);
    }
    Ok(SqlOutcome {
        columns,
        rows,
        truncated,
    })
}
