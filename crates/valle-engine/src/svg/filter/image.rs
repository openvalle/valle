//! Image inputs and the drawing dependencies they introduce into SVG filter graphs.
use std::collections::HashMap;
use std::rc::Rc;

use super::{Attributes, Operation};
use crate::style::Style;
use crate::svg::{Element, Fit};

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub href: Rc<str>,
    pub fit: Fit,
    /// The primitive's node, for its embedded picture and image-rendering style.
    pub node: usize,
}

impl Image {
    pub(super) fn parse(a: &Attributes<'_>) -> Result<Self, String> {
        let href = a
            .string("href")
            .or_else(|| a.string("xlinkHref"))
            .ok_or("feImage needs href")?;
        if href.is_empty() || href == "#" {
            return Err("feImage href must name an image or an SVG element".into());
        }
        let fit = a
            .string("preserveAspectRatio")
            .map(|s| crate::svg::fit(&s))
            .transpose()?
            .unwrap_or(crate::svg::MEET);
        Ok(Self {
            href: href.into(),
            fit,
            node: 0,
        })
    }
}

pub(super) fn validate(styles: &[Style], parents: &[Option<u32>]) -> Result<(), String> {
    let element = |n: usize| styles[n].svg.as_deref().map(|s| &s.element);
    let mut ids = HashMap::new();
    for (i, s) in styles.iter().enumerate() {
        if let Some(id) = s.svg.as_deref().and_then(|s| s.id.as_deref()) {
            ids.entry(id).or_insert(i);
        }
    }
    let mut roots = Vec::new();
    let mut edges = vec![Vec::new(); styles.len()];
    for (i, s) in styles.iter().enumerate() {
        if let Some(Element::Primitive(p)) = element(i)
            && let Operation::Image(image) = &p.op
            && let Some(id) = image.href.strip_prefix('#')
        {
            let &source = ids
                .get(id)
                .ok_or_else(|| format!("feImage href: #{id} names no element"))?;
            if !matches!(
                element(source),
                Some(
                    Element::G
                        | Element::Svg { .. }
                        | Element::Symbol { .. }
                        | Element::Shape(_)
                        | Element::Text(_)
                        | Element::Use { .. }
                        | Element::Image { .. }
                )
            ) {
                return Err(format!("feImage href: #{id} is not a graphics element"));
            }
            edges[i].push(source);
            roots.push(i);
        }
        // Definition containers do not draw as children. A referenced filter does
        // evaluate its primitives, so its image dependencies remain reachable.
        if let Some(parent) = parents[i] {
            let definition = matches!(
                element(i),
                Some(
                    Element::Defs
                        | Element::Filter(_)
                        | Element::Gradient(_)
                        | Element::Pattern(_)
                        | Element::ClipPath { .. }
                        | Element::Mask(_)
                        | Element::Marker(_)
                        | Element::Symbol { .. }
                )
            );
            if !definition {
                edges[parent as usize].push(i);
            }
        }
        for id in [&s.filter_ref, &s.mask_ref, &s.clip_ref]
            .into_iter()
            .flatten()
        {
            if let Some(&n) = ids.get(&**id) {
                edges[i].push(n);
            }
        }
        if let Some(Element::Use { href, .. }) = element(i)
            && let Some(&n) = ids.get(&**href)
        {
            edges[i].push(n);
        }
    }
    // Iterative DFS avoids consuming the stack on a deeply nested authored tree.
    let mut state = vec![0u8; styles.len()];
    for root in roots {
        if state[root] == 2 {
            continue;
        }
        state[root] = 1;
        let mut stack = vec![(root, 0)];
        while let Some((node, next)) = stack.last_mut() {
            if *next == edges[*node].len() {
                state[*node] = 2;
                stack.pop();
                continue;
            }
            let to = edges[*node][*next];
            *next += 1;
            if state[to] == 1 {
                let id = styles[to]
                    .svg
                    .as_deref()
                    .and_then(|s| s.id.as_deref())
                    .unwrap_or("unnamed element");
                return Err(format!("feImage references form a drawing cycle at {id}"));
            }
            if state[to] == 0 {
                state[to] = 1;
                stack.push((to, 0));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::image::Picture;
    use crate::svg::file::{Document, dependencies};

    #[test]
    fn image_references_reject_missing_graphics_and_drawing_cycles() {
        for (body, message) in [
            (
                r##"<filter id="f"><feImage href="#missing"/></filter>"##,
                "#missing",
            ),
            (
                r##"<filter id="f"><feImage href="#f"/></filter>"##,
                "not a graphics element",
            ),
            (
                r##"<defs><filter id="f"><feImage href="#self"/></filter></defs><rect id="self" width="20" height="20" filter="url(#f)"/>"##,
                "drawing cycle",
            ),
            (
                r##"<defs><filter id="a"><feImage href="#second"/></filter><filter id="b"><feImage href="#first"/></filter></defs><g id="first" filter="url(#a)"><rect width="10" height="10"/></g><g id="second" filter="url(#b)"><rect width="10" height="10"/></g>"##,
                "drawing cycle",
            ),
        ] {
            let text = format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{body}</svg>"#);
            let error = Document::parse(&text, &mut Vec::new()).unwrap_err();
            assert!(error.contains(message), "{error}");
        }
        let valid = r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><g id="shape"><rect width="10" height="10"/></g><filter id="f"><feImage href="#shape"/></filter></defs><rect width="10" height="10" filter="url(#f)"/></svg>"##;
        Document::parse(valid, &mut Vec::new()).unwrap();
    }

    #[test]
    fn external_images_use_the_file_loader_and_packaging_dependencies() {
        let text = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><defs><rect id="shape" width="10" height="10"/><filter id="f"><feImage href="images/tile.svg" xlink:href="wrong.svg"/><feImage xlink:href="#shape"/></filter></defs><rect width="10" height="10" filter="url(#f)"/></svg>"##;
        assert_eq!(
            dependencies("art/root.svg", text).unwrap(),
            ["art/images/tile.svg"]
        );
        let mut loaded = Vec::new();
        let doc = Document::parse_with(text, &mut Vec::new(), &mut |href, images| {
            loaded.push(href.to_string());
            Picture::load(br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10"/></svg>"#, images)
        }).unwrap();
        assert_eq!(loaded, ["images/tile.svg"]);
        assert_eq!(doc.embedded.len(), 1);
        let error =
            Document::parse_with(text, &mut Vec::new(), &mut |_, _| Err("not found".into()))
                .unwrap_err();
        assert!(error.contains("<feImage> not found"), "{error}");
    }
}
