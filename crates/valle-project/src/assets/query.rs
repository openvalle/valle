use super::{Library, fts};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;

impl Library {
    pub fn show(&self, path: &str) -> Result<Value> {
        let mut value = self.describe(&self.entry(path)?)?;
        value["status"] = json!("ok");
        Ok(value)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn list(
        &self,
        directory: Option<&str>,
        kind: Option<&str>,
        tag: Option<&str>,
        missing: bool,
        query: Option<&str>,
        filter: Option<&str>,
        limit: usize,
    ) -> Result<Value> {
        let directory = directory
            .map(|d| {
                super::relative(d).map(|p| p.to_string_lossy().trim_end_matches('/').to_owned())
            })
            .transpose()?;
        let supported_kinds = [
            "video",
            "audio",
            "image",
            "font",
            "model3d",
            "environment",
            "shader",
            "component",
            "data",
            "other",
        ];
        if let Some(kind) = kind
            && !supported_kinds.contains(&kind)
        {
            bail!("unknown asset kind: {kind}");
        }
        let filters = Filters::parse(filter)?;
        let ranks = if let Some(query) = query {
            let Some(expression) = fts::query_expr(query) else {
                return Ok(json!({"status":"ok","items":[],"total":0}));
            };
            Some(self.conn.prepare("SELECT rowid,bm25(asset_search) FROM asset_search WHERE asset_search MATCH ? ORDER BY bm25(asset_search)")?.query_map([expression],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,f64>(1)?)))?.collect::<rusqlite::Result<BTreeMap<_,_>>>()?)
        } else {
            None
        };
        let mut matches = Vec::new();
        for entry in self.entries()? {
            if entry.missing != missing || kind.is_some_and(|k| entry.kind != k) {
                continue;
            }
            if directory.as_ref().is_some_and(|d| {
                let directory = crate::files::folded(d);
                let path = crate::files::folded(&entry.path);
                !directory.is_empty()
                    && path != directory
                    && !path.starts_with(&format!("{directory}/"))
            }) {
                continue;
            }
            if ranks.as_ref().is_some_and(|r| !r.contains_key(&entry.id)) {
                continue;
            }
            let item = self.describe(&entry)?;
            if tag.is_some_and(|t| !item["tags"].as_array().unwrap().iter().any(|v| v == t))
                || !filters.matches(&item["info"])
            {
                continue;
            }
            let rank = ranks
                .as_ref()
                .and_then(|r| r.get(&entry.id))
                .copied()
                .unwrap_or(0.0);
            matches.push((rank, item));
        }
        matches.sort_by(|(a, ai), (b, bi)| {
            a.total_cmp(b)
                .then_with(|| ai["path"].as_str().cmp(&bi["path"].as_str()))
        });
        let total = matches.len();
        Ok(
            json!({"status":"ok","root":self.root,"total":total,"items":matches.into_iter().take(limit).map(|(_,item)|item).collect::<Vec<_>>(),"truncated":total>limit}),
        )
    }
}

#[derive(Default)]
struct Filters {
    min: Option<f64>,
    max: Option<f64>,
    orientation: Option<String>,
}
impl Filters {
    fn parse(value: Option<&str>) -> Result<Self> {
        let mut filters = Self::default();
        for pair in value.into_iter().flat_map(|s| s.split(',')) {
            let Some((key, value)) = pair.split_once('=') else {
                bail!("filter needs KEY=VALUE: {pair}");
            };
            match key {
                "min-dur" | "max-dur" => {
                    let number = value.parse::<f64>()?;
                    if !number.is_finite() || number < 0.0 {
                        bail!("duration filter must be finite and nonnegative");
                    }
                    if key == "min-dur" {
                        filters.min = Some(number);
                    } else {
                        filters.max = Some(number);
                    }
                }
                "orientation" if ["portrait", "landscape", "square"].contains(&value) => {
                    filters.orientation = Some(value.to_owned())
                }
                _ => bail!(
                    "unknown filter {pair}; use min-dur, max-dur, or orientation=portrait|landscape|square"
                ),
            }
        }
        Ok(filters)
    }
    fn matches(&self, info: &Value) -> bool {
        if self
            .min
            .is_some_and(|min| info["duration"].as_f64().is_none_or(|v| v < min))
            || self
                .max
                .is_some_and(|max| info["duration"].as_f64().is_none_or(|v| v > max))
        {
            return false;
        }
        if let Some(orientation) = &self.orientation {
            let Some((w, h)) = info["width"].as_u64().zip(info["height"].as_u64()) else {
                return false;
            };
            if match orientation.as_str() {
                "portrait" => w >= h,
                "landscape" => w <= h,
                _ => w != h,
            } {
                return false;
            }
        }
        true
    }
}
