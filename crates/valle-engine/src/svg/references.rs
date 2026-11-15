//! Local fragments imported from other SVG documents. Resolve once before binding styles,
//! preserving declarations for use-instance inheritance and giving each document its own IDs.

use std::collections::HashMap;
use std::rc::Rc;

use super::file::Embedded;
use crate::template::{Kind, Node, Prop, Tag, Template, V};

pub(crate) struct Bundle {
    pub template: Template,
    pub embedded: Vec<Embedded>,
}

/// A supported fragment-valued attribute or property, without its `url()` wrapper.
fn address<'a>(tag: Tag, key: &str, value: &'a str) -> Option<(&'a str, bool)> {
    let value = value.trim();
    if matches!(key, "href" | "xlinkHref") && tag != Tag::Image {
        return Some((value, false));
    }
    if !matches!(
        key,
        "fill"
            | "stroke"
            | "clipPath"
            | "mask"
            | "filter"
            | "markerStart"
            | "markerMid"
            | "markerEnd"
    ) {
        return None;
    }
    let value = value.strip_prefix("url(")?.strip_suffix(')')?.trim();
    let value = value
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .or_else(|| value.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
        .unwrap_or(value);
    Some((value, true))
}

/// External file and nonempty fragment, as authored. Image URLs without fragments are separate.
pub(crate) fn external(tag: Tag, key: &str, value: &str) -> Option<(String, String)> {
    let (value, _) = address(tag, key, value)?;
    let (file, id) = value.rsplit_once('#')?;
    (!file.is_empty() && !id.is_empty()).then(|| (file.into(), id.into()))
}

fn properties(node: &Node) -> impl Iterator<Item = &(Rc<str>, Prop)> {
    let attributes = match &node.kind {
        Kind::Svg { attributes, .. } => &attributes[..],
        _ => &[],
    };
    attributes.iter().chain(&node.style)
}

pub(crate) fn needed(template: &Template) -> bool {
    template.nodes.iter().any(|n| {
        let Kind::Svg { tag, .. } = n.kind else { return false };
        properties(n).any(|(key, value)| matches!(value, Prop::Const(V::Str(s)) if external(tag, key, s).is_some()))
    })
}

fn id(node: &Node) -> Option<&str> {
    properties(node).find_map(|(k, p)| match (&**k, p) {
        ("id", Prop::Const(V::Str(s))) => Some(&**s),
        _ => None,
    })
}

/// Rewrite only ID references, leaving colors, text and resource image URLs intact.
fn rewrite(
    node: &mut Node,
    mut resolve: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<(), String> {
    let Kind::Svg {
        tag, attributes, ..
    } = &mut node.kind
    else {
        return Ok(());
    };
    let has_href = attributes.iter().any(|(key, _)| &**key == "href");
    for (key, prop) in attributes.iter_mut().chain(&mut node.style) {
        if has_href && &**key == "xlinkHref" {
            continue;
        }
        let Prop::Const(V::Str(value)) = prop else {
            continue;
        };
        let Some((href, url)) = address(*tag, key, value) else {
            continue;
        };
        if let Some(new) = resolve(href)? {
            *value = if url {
                format!("url({new})").into()
            } else {
                new.into()
            };
        }
    }
    Ok(())
}

/// Append hidden definition roots; original node indices and expression programs stay intact.
pub(crate) fn expand(
    mut bundle: Bundle,
    mut load: impl FnMut(&str) -> Result<Bundle, String>,
) -> Result<Bundle, String> {
    let mut imports: HashMap<String, (String, Vec<String>)> = HashMap::new();
    let mut prefix = "__valle_external_".to_string();
    while bundle
        .template
        .nodes
        .iter()
        .filter_map(id)
        .any(|id| id.starts_with(&prefix))
    {
        prefix.push('_');
    }
    let count = bundle.template.nodes.len();
    for i in 0..count {
        let mut node = bundle.template.nodes[i].clone();
        let place = node.place();
        rewrite(&mut node, |href| {
            let Some((file, target)) = href.rsplit_once('#').filter(|(file, _)| !file.is_empty())
            else {
                return Ok(None);
            };
            if !imports.contains_key(file) {
                let mut imported = load(file).map_err(|e| format!("{place}: {file}: {e}"))?;
                let names: Vec<String> = imported
                    .template
                    .nodes
                    .iter()
                    .filter_map(id)
                    .map(str::to_string)
                    .collect();
                let namespace = format!("{prefix}{}_", imports.len());
                let wrapper = bundle.template.nodes.len() as u32;
                let offset = wrapper + 1;
                for n in &mut imported.template.nodes {
                    rewrite(n, |href| {
                        Ok(href.strip_prefix('#').map(|id| format!("#{namespace}{id}")))
                    })?;
                    if let Kind::Svg { attributes, .. } = &mut n.kind {
                        for (k, p) in attributes {
                            if &**k == "id"
                                && let Prop::Const(V::Str(v)) = p
                            {
                                *v = format!("{namespace}{v}").into();
                            }
                        }
                    }
                    n.children.iter_mut().for_each(|c| *c += offset);
                }
                // Keep the source root and its viewport declarations intact, under a hidden
                // defs wrapper. This also preserves inherited styles on paint definitions.
                bundle.template.roots.push(wrapper);
                bundle.template.nodes.push(Node {
                    kind: Kind::Svg {
                        tag: Tag::Defs,
                        attributes: vec![],
                        geometry: None,
                    },
                    path: None,
                    classes: vec![],
                    style: vec![],
                    site: Some(place.clone()),
                    children: imported.template.roots.iter().map(|r| r + offset).collect(),
                });
                bundle.template.nodes.extend(imported.template.nodes);
                bundle
                    .embedded
                    .extend(imported.embedded.into_iter().map(|mut e| {
                        e.node += offset;
                        e
                    }));
                imports.insert(file.into(), (namespace, names));
            }
            let (namespace, names) = &imports[file];
            if !names.iter().any(|id| id == target) {
                return Err(format!(
                    "{place}: {file}#{target}: the SVG declares no element {target}"
                ));
            }
            Ok(Some(format!("#{namespace}{target}")))
        })?;
        bundle.template.nodes[i] = node;
    }
    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::Picture;

    #[test]
    fn file_fragments_keep_private_ids_and_discover_all_dependencies() {
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><style>.ink {fill: url('./defs.svg#ink')}</style><defs><rect id="tile" width="99" height="99"/></defs><use href="defs.svg#tile" fill="blue"/><use href="./defs.svg#tile" x="40" fill="red"/><rect class="ink" x="80" width="20" height="20"/></svg>"##;
        let defs = r##"<svg xmlns="http://www.w3.org/2000/svg"><style>#tile {stroke: lime; stroke-width: 2}</style><defs><linearGradient id="ink"><stop stop-color="blue"/><stop offset="1" stop-color="lime"/></linearGradient></defs><g id="tile"><rect width="20" height="20"/><use href="#dot" x="20"/></g><circle id="dot" r="5"/></svg>"##;
        let files = |p: &str| (p == "art/defs.svg").then(|| defs.as_bytes().to_vec());
        let Picture::Svg(doc) =
            Picture::read(source.as_bytes(), "art/main.svg", &files, &mut vec![]).unwrap()
        else {
            panic!()
        };
        let ids: Vec<_> = doc.template.nodes.iter().filter_map(id).collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
        assert_eq!(
            super::super::file::dependencies("art/main.svg", source).unwrap(),
            ["art/defs.svg"]
        );
        let instances: Vec<_> = doc
            .styles
            .iter()
            .filter_map(|s| s.instance.as_ref())
            .collect();
        assert!(instances.len() >= 2);
        assert_ne!(instances[0].styles[1].fill, instances[1].styles[1].fill);
        assert_eq!(
            instances[0].styles[0].stroke.width,
            crate::style::math::Math::Sum([2.0, 0.0])
        );
    }

    #[test]
    fn missing_fragments_files_wrong_formats_and_cycles_fail() {
        let main =
            r##"<svg xmlns="http://www.w3.org/2000/svg"><use href="other.svg#tile"/></svg>"##;
        let empty = r##"<svg xmlns="http://www.w3.org/2000/svg"/>"##;
        let cycle = r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="tile"><use href="main.svg#root"/></g></svg>"##;
        for (other, expected) in [
            (None, "other.svg: not found"),
            (Some(empty), "declares no element tile"),
            (Some(cycle), "cycle"),
        ] {
            let read = |p: &str| match p {
                "other.svg" => other.map(|s| s.as_bytes().to_vec()),
                "main.svg" => Some(main.as_bytes().to_vec()),
                _ => None,
            };
            let error = Picture::read(main.as_bytes(), "main.svg", &read, &mut vec![]).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
        let mut images = vec![];
        let error = super::super::file::Document::parse_with(main, &mut images, &mut |_, _| {
            Ok(Picture::None)
        })
        .unwrap_err();
        assert!(error.contains("needs an SVG file"), "{error}");
    }
}
