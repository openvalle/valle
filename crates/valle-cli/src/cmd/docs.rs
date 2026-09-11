//! Offline guides embedded from the same source tree as the CLI.

use std::{io::Write, process::ExitCode};

use anyhow::{Context, Result};
use serde_json::json;

struct Guide {
    topic: &'static str,
    title: &'static str,
    content: &'static str,
}

const GUIDES: &[Guide] = &[
    Guide {
        topic: "cli",
        title: "CLI guide",
        content: include_str!("../../../../docs/cli.md"),
    },
    Guide {
        topic: "motion",
        title: "Motion authoring reference",
        content: include_str!("../../../../docs/motion.md"),
    },
    Guide {
        topic: "timeline",
        title: "Timeline authoring reference",
        content: include_str!("../../../../docs/timeline.md"),
    },
    Guide {
        topic: "media",
        title: "Media processing reference",
        content: include_str!("../../../../docs/media.md"),
    },
    Guide {
        topic: "project",
        title: "Project editing reference",
        content: include_str!("../../../../docs/project.md"),
    },
    Guide {
        topic: "assets",
        title: "Asset library reference",
        content: include_str!("../../../../docs/assets.md"),
    },
];

pub(crate) fn topics() -> impl Iterator<Item = &'static str> {
    GUIDES.iter().map(|guide| guide.topic)
}

pub(crate) fn run(topic: Option<&str>) -> Result<ExitCode> {
    let guide = topic
        .map(|topic| {
            GUIDES
                .iter()
                .find(|guide| guide.topic == topic)
                .with_context(|| format!("unknown documentation topic: {topic}"))
        })
        .transpose()?;
    if crate::output::machine() {
        let result = match guide {
            Some(guide) => json!({
                "status": "ok", "version": env!("CARGO_PKG_VERSION"),
                "topic": guide.topic, "title": guide.title,
                "format": "markdown", "content": guide.content,
            }),
            None => json!({
                "status": "ok", "version": env!("CARGO_PKG_VERSION"),
                "topics": GUIDES.iter().map(|guide| json!({
                    "topic": guide.topic, "title": guide.title,
                })).collect::<Vec<_>>(),
            }),
        };
        crate::output::emit(result);
    } else {
        let mut stdout = std::io::stdout().lock();
        let result = match guide {
            Some(guide) => stdout.write_all(guide.content.as_bytes()),
            None => {
                let mut index = String::from("Bundled documentation (valle docs <TOPIC>):\n");
                for guide in GUIDES {
                    index.push_str(&format!("  {:<10} {}\n", guide.topic, guide.title));
                }
                stdout.write_all(index.as_bytes())
            }
        };
        // Readers such as `head` may finish before the complete guide is written.
        if let Err(error) = result
            && error.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(error.into());
        }
    }
    Ok(ExitCode::SUCCESS)
}
