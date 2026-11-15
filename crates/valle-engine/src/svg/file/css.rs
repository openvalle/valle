//! Static stylesheets in SVG files: matching selectors and ordering declarations before the
//! ordinary style parser computes their values. Unsupported selectors fail explicitly.

use std::rc::Rc;

use roxmltree::Node;

use super::{INERT, SVG, camel};
use crate::template::{Prop, V};

#[derive(Default)]
pub(super) struct Sheet {
    rules: Vec<Rule>,
}

struct Rule {
    selectors: Vec<Selector>,
    declarations: Vec<Declaration>,
}

struct Declaration {
    key: Rc<str>,
    value: Rc<str>,
    important: bool,
}

struct Selector(Vec<(Compound, Relation)>);

struct Compound {
    tag: Option<String>,
    ids: Vec<String>,
    classes: Vec<String>,
}

#[derive(Clone, Copy)]
enum Relation {
    Descendant,
    Child,
    Next,
    Following,
}

impl Sheet {
    pub(super) fn read(xml: &roxmltree::Document) -> Result<Self, String> {
        let mut sheet = Self::default();
        for node in xml.descendants().filter(|n| {
            n.is_element()
                && n.tag_name().namespace() == Some(SVG)
                && n.tag_name().name() == "style"
        }) {
            if node.attribute("type").is_some_and(|v| v != "text/css") {
                return Err("<style> type must be text/css".into());
            }
            if node
                .attribute("media")
                .is_some_and(|v| !matches!(v.trim(), "" | "all"))
            {
                return Err("SVG <style> media queries are not supported".into());
            }
            let text: String = node.children().filter_map(|n| n.text()).collect();
            let text = comments(&text)?;
            let mut rest = text.as_str();
            while !rest
                .trim_matches(|c: char| c.is_whitespace() || c == '\0')
                .is_empty()
            {
                let open = boundary(rest, '{')?.ok_or("stylesheet rule needs { declarations }")?;
                let selectors = split(&rest[..open], ',')?
                    .into_iter()
                    .map(Selector::parse)
                    .collect::<Result<Vec<_>, _>>()?;
                rest = &rest[open + 1..];
                let close = boundary(rest, '}')?.ok_or("stylesheet rule has no closing }")?;
                let declarations = declarations(&rest[..close])?;
                sheet.rules.push(Rule {
                    selectors,
                    declarations,
                });
                rest = &rest[close + 1..];
            }
        }
        Ok(sheet)
    }

    pub(super) fn style(&self, node: Node) -> Result<Vec<(Rc<str>, Prop)>, String> {
        let inline = declarations(&comments(node.attribute("style").unwrap_or_default())?)?;
        let mut matched = Vec::new();
        for rule in &self.rules {
            let specificity = rule
                .selectors
                .iter()
                .filter(|s| s.matches(node))
                .map(Selector::specificity)
                .max();
            if let Some(specificity) = specificity {
                for d in &rule.declarations {
                    matched.push(((d.important, false, specificity), d));
                }
            }
        }
        for d in &inline {
            matched.push(((d.important, true, [0; 3]), d));
        }
        // Stable sorting retains source order among declarations of equal precedence.
        matched.sort_by_key(|(priority, _)| *priority);
        Ok(matched
            .into_iter()
            .map(|(_, d)| (d.key.clone(), Prop::Const(V::Str(d.value.clone()))))
            .collect())
    }
}

impl Selector {
    fn parse(text: &str) -> Result<Self, String> {
        // A comment separates tokens, but does not create a descendant combinator.
        let ident = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-');
        let mut selector = String::new();
        for part in text.split('\0') {
            if let (Some(a), Some(b)) = (selector.chars().last(), part.chars().next())
                && (ident(a) || a == '#')
                && ident(b)
            {
                return Err("a comment splits an SVG selector token".into());
            }
            selector.push_str(part);
        }
        let mut rest = selector.trim();
        let mut parts = Vec::new();
        let mut relation = Relation::Descendant;
        while !rest.is_empty() {
            let end = rest
                .find(|c: char| c.is_ascii_whitespace() || matches!(c, '>' | '+' | '~'))
                .unwrap_or(rest.len());
            if end == 0 {
                return Err(format!("invalid SVG selector {text:?}"));
            }
            let token = &rest[..end];
            let mut compound = Compound {
                tag: None,
                ids: Vec::new(),
                classes: Vec::new(),
            };
            let mut at = 0;
            while at < token.len() {
                let marker = token.as_bytes()[at];
                if marker == b'*' && at == 0 {
                    at += 1;
                    continue;
                }
                let start = at + usize::from(matches!(marker, b'.' | b'#'));
                let count = token[start..]
                    .bytes()
                    .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
                    .count();
                if count == 0 || (at > 0 && !matches!(marker, b'.' | b'#')) {
                    return Err(format!(
                        "SVG selector {text:?}: use type, *, .class or #id selectors with descendant, >, + or ~ combinators"
                    ));
                }
                let name = token[start..start + count].to_string();
                match marker {
                    b'.' => compound.classes.push(name),
                    b'#' => compound.ids.push(name),
                    _ => compound.tag = Some(name),
                }
                at = start + count;
            }
            parts.push((compound, relation));
            rest = rest[end..].trim_start();
            relation = match rest.as_bytes().first() {
                Some(b'>') => Relation::Child,
                Some(b'+') => Relation::Next,
                Some(b'~') => Relation::Following,
                _ => Relation::Descendant,
            };
            if !matches!(relation, Relation::Descendant) {
                rest = rest[1..].trim_start();
                if rest.is_empty() {
                    return Err(format!("incomplete SVG selector {text:?}"));
                }
            }
        }
        if parts.is_empty() {
            return Err("empty SVG selector".into());
        }
        Ok(Self(parts))
    }

    fn specificity(&self) -> [usize; 3] {
        self.0.iter().fold([0; 3], |[ids, classes, tags], (p, _)| {
            [
                ids + p.ids.len(),
                classes + p.classes.len(),
                tags + usize::from(p.tag.is_some()),
            ]
        })
    }

    fn matches(&self, node: Node) -> bool {
        self.at(self.0.len() - 1, node)
    }

    fn at(&self, i: usize, node: Node) -> bool {
        let (compound, relation) = &self.0[i];
        if !node.is_element()
            || node.tag_name().namespace() != Some(SVG)
            || compound
                .tag
                .as_deref()
                .is_some_and(|t| node.tag_name().name() != t)
            || compound
                .ids
                .iter()
                .any(|id| node.attribute("id") != Some(id))
            || compound.classes.iter().any(|class| {
                !node
                    .attribute("class")
                    .unwrap_or_default()
                    .split_ascii_whitespace()
                    .any(|c| c == class)
            })
        {
            return false;
        }
        if i == 0 {
            return true;
        }
        match relation {
            Relation::Child => node.parent_element().is_some_and(|p| self.at(i - 1, p)),
            Relation::Descendant => node.ancestors().skip(1).any(|p| self.at(i - 1, p)),
            Relation::Next => node
                .prev_siblings()
                .skip(1)
                .find(|p| p.is_element())
                .is_some_and(|p| self.at(i - 1, p)),
            Relation::Following => node.prev_siblings().skip(1).any(|p| self.at(i - 1, p)),
        }
    }
}

fn declarations(text: &str) -> Result<Vec<Declaration>, String> {
    let separated = text.replace('\0', " ");
    let text = separated.as_str();
    let mut result = Vec::new();
    for declaration in split(text, ';')?
        .into_iter()
        .filter(|s| !s.trim().is_empty())
    {
        let at = boundary(declaration, ':')?.ok_or("CSS declaration needs property: value")?;
        let key = camel(&declaration[..at].trim().to_ascii_lowercase());
        let mut value = declaration[at + 1..].trim();
        let important = value.rsplit_once('!').is_some_and(|(before, after)| {
            if after.trim().eq_ignore_ascii_case("important") {
                value = before.trim_end();
                true
            } else {
                false
            }
        });
        if key.is_empty() || value.is_empty() {
            return Err("CSS declaration needs a property and value".into());
        }
        if !INERT.contains(&key.as_str()) {
            result.push(Declaration {
                key: key.into(),
                value: value.into(),
                important,
            });
        }
    }
    Ok(result)
}

fn split(text: &str, delimiter: char) -> Result<Vec<&str>, String> {
    let mut rest = text;
    let mut out = Vec::new();
    while let Some(at) = boundary(rest, delimiter)? {
        out.push(&rest[..at]);
        rest = &rest[at + delimiter.len_utf8()..];
    }
    out.push(rest);
    Ok(out)
}

/// Finds punctuation outside strings and function arguments; malformed blocks fail early.
fn boundary(text: &str, delimiter: char) -> Result<Option<usize>, String> {
    let mut quote = None;
    let mut escaped = false;
    let mut depth = 0;
    for (i, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        if c == delimiter && depth == 0 {
            return Ok(Some(i));
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ')' | '{' | '}' => return Err(format!("unexpected {c} in SVG CSS")),
            _ => {}
        }
    }
    if quote.is_some() || depth != 0 || escaped {
        return Err("unclosed SVG CSS string or function".into());
    }
    Ok(None)
}

fn comments(text: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = text;
    let mut quote = None;
    let mut escaped = false;
    while let Some(c) = rest.chars().next() {
        if quote.is_none() && rest.starts_with("/*") {
            let end = rest.find("*/").ok_or("unclosed SVG CSS comment")?;
            rest = &rest[end + 2..];
            out.push('\0');
            continue;
        }
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if quote == Some(c) {
            quote = None;
        } else if quote.is_none() && matches!(c, '\'' | '"') {
            quote = Some(c);
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;

    #[test]
    fn rules_cascade_by_importance_specificity_and_order() {
        let xml = roxmltree::Document::parse(indoc! {r#"
            <svg xmlns="http://www.w3.org/2000/svg">
              <g id="group" class="palette"><rect id="target" class="a b" style="fill: pink; stroke: black !important"/><rect id="next"/></g>
              <style><![CDATA[
                * { fill: red }
                .a { fill: blue }
                rect.a.b { fill: yellow }
                #target { fill: purple; stroke: yellow !important }
                .palette > rect.a { fill: green !important }
                g .a { opacity: .5 }
                #target + rect { fill: orange }
                .a ~ rect { stroke: blue }
              ]]></style>
              <style>.a { fill: cyan !IMPORTANT }</style>
            </svg>
        "#}).unwrap();
        let sheet = Sheet::read(&xml).unwrap();
        let get = |id: &str, property: &str| {
            let node = xml
                .descendants()
                .find(|n| n.attribute("id") == Some(id))
                .unwrap();
            let styles = sheet.style(node).unwrap();
            styles
                .into_iter()
                .rev()
                .find_map(|(key, value)| {
                    (key.as_ref() == property).then(|| match value {
                        Prop::Const(V::Str(s)) => s.to_string(),
                        _ => panic!(),
                    })
                })
                .unwrap()
        };
        assert_eq!(get("target", "fill"), "green");
        assert_eq!(get("target", "stroke"), "black");
        assert_eq!(get("target", "opacity"), ".5");
        assert_eq!(get("next", "fill"), "orange");
        assert_eq!(get("next", "stroke"), "blue");
    }

    #[test]
    fn comments_do_not_turn_compound_selectors_into_descendants() {
        let xml = roxmltree::Document::parse(r#"<svg xmlns="http://www.w3.org/2000/svg"><style>/* before */ .a/**/.b { fill: red } /* after */</style><g class="a"><rect id="child" class="b"/></g><rect id="both" class="a b"/></svg>"#).unwrap();
        let sheet = Sheet::read(&xml).unwrap();
        for (id, count) in [("child", 0), ("both", 1)] {
            let node = xml
                .descendants()
                .find(|n| n.attribute("id") == Some(id))
                .unwrap();
            assert_eq!(sheet.style(node).unwrap().len(), count);
        }
        assert!(Selector::parse("rect\0rect").is_err());
        assert!(Selector::parse("#\0id").is_err());
    }

    #[test]
    fn strings_comments_and_unsupported_syntax_are_not_misparsed() {
        let values =
            declarations(r#"font-family: "A;B:/*C*/"; fill: rgb(1, 2, 3) ! IMPORTANT"#).unwrap();
        assert_eq!(&*values[0].value, r#""A;B:/*C*/""#);
        assert!(values[1].important);
        assert_eq!(
            comments("/* comment */ .a {fill:red}").unwrap(),
            "\0 .a {fill:red}"
        );
        for selector in ["rect:hover", "[fill=red]", "@media all", ".a >", ""] {
            assert!(Selector::parse(selector).is_err(), "{selector}");
        }
        assert!(declarations("fill: rgb(1,2,3").is_err());
        assert!(comments("/* open").is_err());
    }
}
