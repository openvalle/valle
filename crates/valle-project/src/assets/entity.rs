//! Entities with stable IDs and aliases, stored in `entities.json`. Renaming does not require
//! reannotation.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::assets::ctx::Ctx;
use crate::assets::db::Db;
use crate::assets::fsutil;
use crate::assets::home::Home;
use crate::assets::lock;
use crate::assets::report::{AssetsError, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    /// Free-form entity kind, such as person, place, or product.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct EntitiesFile {
    entities: Vec<Entity>,
}

/// Load entities; a missing file means an empty table.
pub fn load(home: &Home) -> Result<Vec<Entity>> {
    let p = home.entities_path();
    if !p.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&p)?;
    let f: EntitiesFile = serde_json::from_slice(&bytes)
        .map_err(|e| AssetsError::io(format!("corrupt entities.json: {e}")))?;
    Ok(f.entities)
}

fn save(home: &Home, entities: &[Entity]) -> Result<()> {
    let f = EntitiesFile {
        entities: entities.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&f)
        .map_err(|e| AssetsError::io(format!("failed to serialize entities: {e}")))?;
    fsutil::write_atomic(&home.entities_path(), &bytes, "entities")
}

/// Replace the entity projection from the authoritative table.
pub fn project(db: &Db, entities: &[Entity]) -> Result<()> {
    db.conn.execute("DELETE FROM entities", [])?;
    for e in entities {
        db.conn.execute(
            "INSERT INTO entities (id, kind, name, aliases) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                e.id,
                e.kind,
                e.name,
                serde_json::to_string(&e.aliases).unwrap_or_else(|_| "[]".into())
            ],
        )?;
    }
    Ok(())
}

pub fn exists(home: &Home, id: &str) -> Result<bool> {
    Ok(load(home)?.iter().any(|e| e.id == id))
}

/// Reject name or alias collisions, excluding the entity being updated.
fn check_collision(entities: &[Entity], names: &[&str], exclude_id: Option<&str>) -> Result<()> {
    for e in entities {
        if Some(e.id.as_str()) == exclude_id {
            continue;
        }
        for n in names {
            if e.name == *n || e.aliases.iter().any(|a| a == n) {
                return Err(AssetsError::refused(format!(
                    "name or alias '{n}' is already used by entity {}({})",
                    e.id, e.name
                ))
                .with_hint(
                    "reuse the existing entity, or choose a distinct name for a different entity",
                ));
            }
        }
    }
    Ok(())
}

pub fn add(ctx: &Ctx, name: &str, kind: Option<&str>, aliases: &[String]) -> Result<Value> {
    let _lock = lock::acquire(&ctx.home)?;
    let mut entities = load(&ctx.home)?;
    let mut names: Vec<&str> = vec![name];
    names.extend(aliases.iter().map(|s| s.as_str()));
    check_collision(&entities, &names, None)?;
    let max: u64 = entities
        .iter()
        .filter_map(|e| e.id.strip_prefix('e').and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0);
    let ent = Entity {
        id: format!("e{}", max + 1),
        kind: kind.map(|s| s.to_owned()),
        name: name.to_owned(),
        aliases: aliases.to_vec(),
    };
    entities.push(ent.clone());
    save(&ctx.home, &entities)?;
    project(&Db::open(&ctx.home)?, &entities)?;
    Ok(serde_json::to_value(&ent).unwrap_or_default())
}

pub fn list(ctx: &Ctx) -> Result<Value> {
    let entities = load(&ctx.home)?;
    Ok(json!({ "entities": entities, "count": entities.len() }))
}

pub fn edit(ctx: &Ctx, id: &str, name: Option<&str>, aliases: Option<&[String]>) -> Result<Value> {
    let _lock = lock::acquire(&ctx.home)?;
    let mut entities = load(&ctx.home)?;
    let mut names: Vec<&str> = Vec::new();
    if let Some(n) = name {
        names.push(n);
    }
    if let Some(al) = aliases {
        names.extend(al.iter().map(|s| s.as_str()));
    }
    check_collision(&entities, &names, Some(id))?;
    let ent = entities.iter_mut().find(|e| e.id == id).ok_or_else(|| {
        AssetsError::not_found(format!("entity not found: {id}"))
            .with_hint("list existing entities with `valle assets entity list`")
    })?;
    if let Some(n) = name {
        ent.name = n.to_owned();
    }
    if let Some(al) = aliases {
        ent.aliases = al.to_vec();
    }
    let out = serde_json::to_value(&*ent).unwrap_or_default();
    save(&ctx.home, &entities)?;
    let db = Db::open(&ctx.home)?;
    project(&db, &entities)?;
    // Refresh affected asset projections after names or aliases change.
    let mut stmt = db
        .conn
        .prepare("SELECT DISTINCT hash FROM annotation_entities WHERE entity_id=?1")?;
    let hashes: Vec<String> = stmt
        .query_map([id], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    for h in hashes {
        crate::assets::units::rebuild_for_asset(&db, &ctx.home, &h)?;
    }
    Ok(out)
}
