//! `valle docs`: the guides in docs/, built into the executable so they match it and need no
//! network or source tree. A guide's sections are its `##` headings, which `--section` picks
//! by name, so a reader takes the part it needs rather than the whole guide.

use serde_json::{Value, json};

use crate::Failure;

pub struct Guide {
    pub topic: &'static str,
    pub title: &'static str,
    pub content: &'static str,
}

pub const GUIDES: [Guide; 5] = [
    Guide {
        topic: "start",
        title: "Start here: the workflow, the commands, and how to read their results",
        content: include_str!("../../../docs/start.md"),
    },
    Guide {
        topic: "motion",
        title: "Motion scenes: components, CSS, animation, controls and effects",
        content: include_str!("../../../docs/motion.md"),
    },
    Guide {
        topic: "timeline",
        title: "Timeline documents: resources, tracks, clips, transitions, captions and sound",
        content: include_str!("../../../docs/timeline.md"),
    },
    Guide {
        topic: "project",
        title: "Project folders: files, checks, snapshots and renders",
        content: include_str!("../../../docs/project.md"),
    },
    Guide {
        topic: "cli",
        title: "The command line: every command, its output and its errors",
        content: include_str!("../../../docs/cli.md"),
    },
];

/// The topics, for clap to offer.
pub fn topics() -> impl Iterator<Item = &'static str> {
    GUIDES.iter().map(|g| g.topic)
}

/// A section: a `##` heading and what follows it, up to the next one.
struct Section<'a> {
    heading: &'a str,
    /// The heading lowercased, its words joined by hyphens: what `--section` takes.
    name: String,
    text: &'a str,
}

fn sections(content: &str) -> Vec<Section<'_>> {
    let mut starts: Vec<usize> = Vec::new();
    let mut fenced = false;
    let mut at = 0;
    for line in content.split_inclusive('\n') {
        if line.starts_with("```") {
            fenced = !fenced;
        } else if !fenced && line.starts_with("## ") {
            starts.push(at);
        }
        at += line.len();
    }
    (starts.iter().enumerate())
        .map(|(i, &start)| {
            let end = starts.get(i + 1).copied().unwrap_or(content.len());
            let text = &content[start..end];
            let heading = text.lines().next().unwrap_or_default()[3..].trim();
            Section {
                heading,
                name: slug(heading),
                text: text.trim_end_matches('\n'),
            }
        })
        .collect()
}

fn slug(heading: &str) -> String {
    let words = heading
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty());
    words.map(str::to_lowercase).collect::<Vec<_>>().join("-")
}

pub fn run(topic: Option<String>, section: Option<String>) -> Result<Value, Failure> {
    let Some(topic) = topic else {
        let listed: Vec<Value> = (GUIDES.iter())
            .map(|g| json!({ "topic": g.topic, "title": g.title, "sections": names(g) }))
            .collect();
        let lines: Vec<String> = (GUIDES.iter())
            .map(|g| format!("  {:<9} {}", g.topic, g.title))
            .collect();
        let summary = format!(
            "valle docs <topic> [--section <name>]\n\n{}\n\nNew here? Read `valle docs start`.",
            lines.join("\n")
        );
        return Ok(json!({ "status": "ok", "summary": summary, "topics": listed }));
    };
    let guide = (GUIDES.iter())
        .find(|g| g.topic == topic)
        .expect("clap takes only the topics");
    let Some(wanted) = section else {
        return Ok(json!({
            "status": "ok",
            "topic": guide.topic,
            "title": guide.title,
            "sections": names(guide),
            "content": guide.content,
        }));
    };
    let all = sections(guide.content);
    let wanted_name = slug(&wanted);
    let Some(found) = all.iter().find(|s| s.name == wanted_name) else {
        let known: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        return Err(Failure::new(
            "usage",
            format!("valle docs {topic} has no section {wanted:?}"),
        )
        .hint(format!("its sections are {}", known.join(", "))));
    };
    Ok(json!({
        "status": "ok",
        "topic": guide.topic,
        "section": found.name,
        "heading": found.heading,
        "content": found.text,
    }))
}

fn names(guide: &Guide) -> Vec<String> {
    sections(guide.content)
        .into_iter()
        .map(|s| s.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_are_level_two_headings_outside_code() {
        let text = "# Title\n\nintro\n\n## First one\n\na\n```md\n## not a heading\n```\n\n\
                    ## Second: two\n\nb\n";
        let s = sections(text);
        let found: Vec<(&str, &str)> = s.iter().map(|s| (s.name.as_str(), s.heading)).collect();
        assert_eq!(
            found,
            [("first-one", "First one"), ("second-two", "Second: two")]
        );
        assert!(s[0].text.contains("## not a heading"));
        assert!(s[1].text.ends_with('b'));
    }

    #[test]
    fn every_guide_has_a_title_and_unique_sections() {
        for g in &GUIDES {
            assert!(g.content.starts_with("# "), "{}", g.topic);
            let mut names = names(g);
            assert!(!names.is_empty(), "{} has no sections", g.topic);
            let n = names.len();
            names.sort();
            names.dedup();
            assert_eq!(names.len(), n, "{} repeats a section name", g.topic);
        }
    }

    #[test]
    fn a_section_is_found_by_its_name_or_heading() {
        let timeline = &GUIDES[2];
        let first = &names(timeline)[0];
        for asked in [first.clone(), first.to_uppercase().replace('-', " ")] {
            let v = run(Some("timeline".into()), Some(asked))
                .unwrap_or_else(|e| panic!("{}", e.message));
            assert_eq!(v["section"], *first);
        }
        let Err(e) = run(Some("timeline".into()), Some("nowhere".into())) else {
            panic!("an unknown section was found")
        };
        assert!(e.hint.unwrap().contains(first.as_str()));
    }
}
