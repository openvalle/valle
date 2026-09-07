//! Commands shared by CLI and Studio. Shells parse inputs into [`Verb`]; the core owns execution.
//! JSON entry points validate unknown fields. Command times use seconds, while storage uses integer
//! milliseconds.

use serde::{Deserialize, Serialize};

use crate::assets::kind::AssetKind;

/// Asset ingestion mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AddMode {
    /// Use APFS clonefile when available, falling back to copying.
    #[default]
    Reflink,
    Copy,
    /// Register an external location and validate its size and modification time.
    Reference,
}

/// Entity operations using stable IDs and aliases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum EntityOp {
    Add {
        name: String,
        /// Free-form entity kind, such as person, place, or product.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        aliases: Vec<String>,
    },
    List,
    Edit {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Replace the complete alias list idempotently.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        aliases: Option<Vec<String>>,
    },
}

/// Asset command set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "kebab-case")]
pub enum Verb {
    /// Ingest one asset. Callers handle files independently in a batch. Repeated ingestion updates
    /// explicit titles, unions tags, and revives removed content.
    Add {
        path: String,
        #[serde(default)]
        mode: AddMode,
        /// Explicit kind override for content probing cannot identify.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<AssetKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
    },
    /// Remove bytes and locations while retaining knowledge. Purging also removes metadata and
    /// annotations, requiring force when human annotations exist.
    Rm {
        hash: String,
        #[serde(default)]
        purge: bool,
        #[serde(default)]
        force: bool,
    },
    List {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<AssetKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
        /// Select stale references only.
        #[serde(default)]
        stale: bool,
        /// Include removed assets.
        #[serde(default)]
        removed: bool,
    },
    /// Asset knowledge card with probing, locations, analysis status, and annotations.
    Show { hash: String },
    /// Library counts, kind distribution, analyzer coverage, cost, and stale references.
    Describe,
    /// Resolve a hash to an absolute path, reporting stale references or removed assets.
    Resolve { hash: String },
    /// Derive word or punctuation-delimited sentence transcripts from the ASR slot.
    Transcript { hash: String, level: String },
    /// Edit human metadata fields; tags use their own operation.
    Edit {
        hash: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// Audio subkind, such as music or SFX.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subkind: Option<String>,
    },
    Tag {
        hash: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        add: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rm: Vec<String>,
    },
    /// Create a point, range, or asset-level annotation, or delete an annotation by ID.
    Annotate {
        hash: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        range: Option<[f64; 2]>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
        /// Linked entity IDs.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        entities: Vec<String>,
        /// Replace an existing annotation; omission creates one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rm: Option<String>,
    },
    Entity {
        #[serde(flatten)]
        op: EntityOp,
    },
    /// Select and analyze assets with resumable jobs and foreground progress.
    Analyze {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        hashes: Vec<String>,
        #[serde(default)]
        all: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<AssetKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
        /// Analyzer names. Model tools execute separately from asset analyzers.
        with: Vec<String>,
        /// Per-invocation API budget in CNY, stored as integer cents.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        budget: Option<f64>,
        #[serde(default)]
        force: bool,
    },
    /// Search filtered FTS projections, returning assets, time ranges, and evidence.
    Search {
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<AssetKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entity: Option<String>,
        /// Structured filters, such as orientation or minimum duration.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Check references, orphans, and projection drift; deep verification rehashes CAS data.
    Verify {
        #[serde(default)]
        deep: bool,
    },
    /// Collect orphan cache entries and blobs while retaining authoritative data and knowledge
    /// preserved by removal.
    Gc,
    /// Rebuild index.db from authoritative files.
    Reindex,
    /// Read-only SQL enforced by the connection and authorizer.
    Sql { query: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(v: &Verb) -> Verb {
        let s = serde_json::to_string(v).unwrap();
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn kebab_tag_shape() {
        let v = Verb::Add {
            path: "/tmp/a.mp4".into(),
            mode: AddMode::Reflink,
            kind: None,
            title: Some("Interview".into()),
            tags: vec!["Promo".into()],
        };
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["verb"], "add");
        assert_eq!(j["mode"], "reflink");
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn minimal_json_uses_defaults() {
        let v: Verb = serde_json::from_str(r#"{"verb":"list"}"#).unwrap();
        assert_eq!(
            v,
            Verb::List {
                kind: None,
                tag: None,
                stale: false,
                removed: false
            }
        );
        let v: Verb = serde_json::from_str(r#"{"verb":"analyze","with":["shots","asr"]}"#).unwrap();
        match v {
            Verb::Analyze {
                with, all, budget, ..
            } => {
                assert_eq!(with, vec!["shots", "asr"]);
                assert!(!all);
                assert!(budget.is_none());
            }
            other => panic!("expected analyze, got {other:?}"),
        }
    }

    #[test]
    fn entity_flatten_action() {
        let v: Verb = serde_json::from_str(
            r#"{"verb":"entity","action":"add","name":"Alex","aliases":["Al"]}"#,
        )
        .unwrap();
        assert_eq!(
            v,
            Verb::Entity {
                op: EntityOp::Add {
                    name: "Alex".into(),
                    kind: None,
                    aliases: vec!["Al".into()]
                }
            }
        );
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn all_sixteen_verbs_roundtrip() {
        let verbs: Vec<Verb> = vec![
            serde_json::from_str(r#"{"verb":"add","path":"a.mp4"}"#).unwrap(),
            serde_json::from_str(r#"{"verb":"rm","hash":"3f8ac2"}"#).unwrap(),
            serde_json::from_str(r#"{"verb":"list"}"#).unwrap(),
            serde_json::from_str(r#"{"verb":"show","hash":"3f8ac2"}"#).unwrap(),
            serde_json::from_str(r#"{"verb":"describe"}"#).unwrap(),
            serde_json::from_str(r#"{"verb":"resolve","hash":"3f8ac2"}"#).unwrap(),
            serde_json::from_str(r#"{"verb":"edit","hash":"3f8ac2","title":"t"}"#).unwrap(),
            serde_json::from_str(r#"{"verb":"tag","hash":"3f8ac2","add":["x"]}"#).unwrap(),
            serde_json::from_str(
                r#"{"verb":"annotate","hash":"3f8ac2","at":83.0,"text":"Opening"}"#,
            )
            .unwrap(),
            serde_json::from_str(r#"{"verb":"entity","action":"list"}"#).unwrap(),
            serde_json::from_str(r#"{"verb":"analyze","with":["shots"]}"#).unwrap(),
            serde_json::from_str(r#"{"verb":"search","query":"Alex Park"}"#).unwrap(),
            serde_json::from_str(r#"{"verb":"verify","deep":true}"#).unwrap(),
            serde_json::from_str(r#"{"verb":"gc"}"#).unwrap(),
            serde_json::from_str(r#"{"verb":"reindex"}"#).unwrap(),
            serde_json::from_str(r#"{"verb":"sql","query":"select 1"}"#).unwrap(),
        ];
        assert_eq!(verbs.len(), 16);
        for v in &verbs {
            assert_eq!(&roundtrip(v), v);
        }
    }
}
