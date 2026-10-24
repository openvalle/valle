//! A folder of reusable files. SQLite owns tags and notes; media facts and search are derived.

mod catalog;
mod clock;
pub mod fts;
mod mutations;
mod query;
mod recovery;
mod scan;
mod sql;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub use crate::files::Kind;
pub use catalog::Library;

pub trait Inspector: Send + Sync {
    fn inspect(&self, _path: &Path, _kind: Kind) -> Value {
        Value::Null
    }
    fn validate(&self, _path: &Path) -> Result<()> {
        Ok(())
    }
}

pub struct NoInspector;
impl Inspector for NoInspector {}

pub struct Context {
    pub root: PathBuf,
    pub inspector: Box<dyn Inspector>,
}

impl Context {
    pub fn file(&self, path: &str) -> Result<(PathBuf, String, Kind)> {
        let mut library = Library::open(&self.root)?;
        library.scan(self.inspector.as_ref())?;
        let entry = library.entry(path)?;
        if entry.missing {
            bail!("asset is missing: {path}");
        }
        let file = safe_path(&library.root, &entry.path)?;
        Ok((file.clone(), entry.digest, Kind::detect(&file)))
    }
    pub fn resolve(inspector: Box<dyn Inspector>) -> Result<Self> {
        Ok(Self {
            root: crate::folder::app_home()?.join("assets"),
            inspector,
        })
    }

    pub fn execute(&self, command: Command) -> Value {
        match self.try_execute(command) {
            Ok(value) => value,
            Err(error) => {
                json!({"status":"error","error":{"code":"assets_failed","message":format!("{error:#}"),"hint":"Use assets show to inspect the current state; if catalog.db cannot open, use valle assets recover."}})
            }
        }
    }

    fn try_execute(&self, command: Command) -> Result<Value> {
        if let Command::Recover { backup } = command {
            return recovery::recover(self, backup.as_deref());
        }
        let mut library = Library::open(&self.root)?;
        library.scan(self.inspector.as_ref())?;
        match command {
            Command::Add {
                paths,
                to,
                name,
                tags,
                note,
            } => library.add_batch(
                &paths,
                to.as_deref(),
                name.as_deref(),
                &tags,
                note.as_deref(),
                self.inspector.as_ref(),
            ),
            Command::List {
                directory,
                kind,
                tag,
                missing,
            } => library.list(
                directory.as_deref(),
                kind.as_deref(),
                tag.as_deref(),
                missing,
                None,
                None,
                1000,
            ),
            Command::Search {
                query,
                kind,
                directory,
                tag,
                filter,
                limit,
            } => library.list(
                directory.as_deref(),
                kind.as_deref(),
                tag.as_deref(),
                false,
                Some(&query),
                filter.as_deref(),
                limit.unwrap_or(50).min(1000),
            ),
            Command::Show { path } => library.show(&path),
            Command::Edit { path, name, to } => library.edit(&path, name.as_deref(), to.as_deref()),
            Command::Tag { path, add, rm } => library.tag(&path, &add, &rm),
            Command::Annotate {
                path,
                text,
                at,
                range,
                id,
                rm,
            } => library.annotate(&path, text.as_deref(), at, range, id, rm),
            Command::Remove { path, missing } => library.remove(path.as_deref(), missing),
            Command::Sql { query } => sql::query(&library.root, &query),
            Command::Recover { .. } => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Command {
    Add {
        paths: Vec<PathBuf>,
        #[serde(default)]
        to: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        note: Option<String>,
    },
    List {
        #[serde(default)]
        directory: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        tag: Option<String>,
        #[serde(default)]
        missing: bool,
    },
    Search {
        query: String,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        directory: Option<String>,
        #[serde(default)]
        tag: Option<String>,
        #[serde(default)]
        filter: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    Show {
        path: String,
    },
    Edit {
        path: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        to: Option<String>,
    },
    Tag {
        path: String,
        #[serde(default)]
        add: Vec<String>,
        #[serde(default)]
        rm: Vec<String>,
    },
    Annotate {
        path: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        at: Option<f64>,
        #[serde(default)]
        range: Option<[f64; 2]>,
        #[serde(default)]
        id: Option<i64>,
        #[serde(default)]
        rm: Option<i64>,
    },
    Remove {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        missing: bool,
    },
    Recover {
        #[serde(default)]
        backup: Option<String>,
    },
    Sql {
        query: String,
    },
}

pub(crate) fn relative(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute()
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || path.components().any(|p| match p {
            std::path::Component::Normal(name) => name.to_string_lossy().starts_with('.'),
            _ => true,
        })
    {
        bail!("use a library-relative path without hidden directories or '..': {value}");
    }
    Ok(path.to_path_buf())
}

pub(crate) fn filename(value: &str) -> Result<&str> {
    let path = relative(value)?;
    if value.is_empty() || path.components().count() != 1 || value.contains('/') {
        bail!("provide a filename, without a directory: {value}");
    }
    Ok(value)
}

pub(crate) fn safe_path(root: &Path, path: &str) -> Result<PathBuf> {
    let relative = relative(path)?;
    let mut current = root.to_path_buf();
    for part in relative.components() {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => bail!(
                "library paths must not follow symlinks: {}",
                current.display()
            ),
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(current)
}
