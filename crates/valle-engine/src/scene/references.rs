//! Import resource SVG fragments before either checking or laying out a variant.

use super::*;
use crate::svg::references::{self, Bundle};

#[derive(Default)]
pub(super) struct Imported {
    assets: HashMap<usize, Template>,
    variants: Vec<(Rc<Template>, Rc<Template>)>,
}

impl Scene {
    pub(super) fn expand(&mut self, source: Rc<Template>) -> Result<Rc<Template>, Error> {
        if let Some((_, expanded)) = self
            .imported
            .variants
            .iter()
            .find(|(s, _)| Rc::ptr_eq(s, &source))
        {
            return Ok(expanded.clone());
        }
        let expanded = if references::needed(&source) {
            let result = references::expand(
                Bundle {
                    template: (*source).clone(),
                    embedded: vec![],
                },
                |href| {
                    let name = href.strip_prefix("resource://").ok_or_else(|| {
                        "inline SVG fragments name resource://name#id".to_string()
                    })?;
                    let r = self
                        .front
                        .resources
                        .iter()
                        .position(|r| &*r.name == name)
                        .ok_or_else(|| {
                            format!("the scene declares no resource {name} in controls.resources")
                        })?;
                    if !self.imported.assets.contains_key(&r) {
                        let Picture::Svg(doc) = &self.pictures[r] else {
                            return Err(format!(
                                "resource://{name}: a fragment reference needs an SVG resource"
                            ));
                        };
                        let mut template = doc.template.clone();
                        let embedded = doc.embedded.clone();
                        for image in embedded {
                            let Kind::Svg { attributes, .. } =
                                &mut template.nodes[image.node as usize].kind
                            else {
                                unreachable!("embedded images are SVG elements")
                            };
                            attributes.retain(|(key, _)| !matches!(&**key, "href" | "xlinkHref"));
                            attributes.push((
                                "href".into(),
                                Prop::Const(V::Str(
                                    format!("resource://{}", self.pictures.len()).into(),
                                )),
                            ));
                            self.pictures.push(image.picture);
                        }
                        self.imported.assets.insert(r, template);
                    }
                    Ok(Bundle {
                        template: self.imported.assets[&r].clone(),
                        embedded: vec![],
                    })
                },
            )?;
            Rc::new(result.template)
        } else {
            source.clone()
        };
        self.imported.variants.insert(0, (source, expanded.clone()));
        self.imported.variants.truncate(SHEETS);
        Ok(expanded)
    }
}
