use anyhow::{Context, Result};
use rusqlite::{
    Connection, OpenFlags,
    hooks::{AuthAction, AuthContext, Authorization},
    types::ValueRef,
};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) fn query(root: &Path, query: &str) -> Result<Value> {
    let conn = Connection::open_with_flags(
        root.join(".valle/catalog.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    conn.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Select
        | AuthAction::Read { .. }
        | AuthAction::Function { .. }
        | AuthAction::Recursive => Authorization::Allow,
        _ => Authorization::Deny,
    }))?;
    let mut statement = conn
        .prepare(query)
        .context("SQL rejected; only read-only SELECT queries are allowed")?;
    let columns = statement
        .column_names()
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>();
    let mut cursor = statement.query([])?;
    let mut rows = Vec::new();
    let mut truncated = false;
    while let Some(row) = cursor.next()? {
        if rows.len() == 1000 {
            truncated = true;
            break;
        }
        let values = (0..columns.len())
            .map(|index| {
                Ok(match row.get_ref(index)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => json!(n),
                    ValueRef::Real(n) => json!(n),
                    ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                    ValueRef::Blob(b) => json!(format!("blob({})", b.len())),
                })
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.push(values);
    }
    Ok(json!({"status":"ok","columns":columns,"rows":rows,"truncated":truncated}))
}
