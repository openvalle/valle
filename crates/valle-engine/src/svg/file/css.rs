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
    attributes: Vec<Attribute>,
    pseudo: Vec<Pseudo>,
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
            let end = compound_end(rest)?;
            if end == 0 {
                return Err(format!("invalid SVG selector {text:?}"));
            }
            let token = &rest[..end];
            let mut compound = Compound {
                tag: None,
                ids: Vec::new(),
                classes: Vec::new(),
                attributes: Vec::new(),
                pseudo: Vec::new(),
            };
            let mut at = 0;
            while at < token.len() {
                let marker = token.as_bytes()[at];
                if marker == b':' {
                    let (pseudo, count) = Pseudo::parse(&token[at + 1..])?;
                    compound.pseudo.push(pseudo);
                    at += count + 1;
                    continue;
                }
                if marker == b'[' {
                    let end = boundary(&token[at + 1..], ']')?
                        .ok_or("unclosed SVG attribute selector")?
                        + at
                        + 1;
                    compound
                        .attributes
                        .push(Attribute::parse(&token[at + 1..end])?);
                    at = end + 1;
                    continue;
                }
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
                classes + p.classes.len() + p.attributes.len() + p.pseudo.len(),
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
            || compound.attributes.iter().any(|a| !a.matches(node))
            || compound.pseudo.iter().any(|p| !p.matches(node))
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

/// End of one compound, retaining quoted attribute values and function arguments.
fn compound_end(text: &str) -> Result<usize, String> {
    let mut quote = None;
    let mut escaped = false;
    let mut stack = Vec::new();
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
        if stack.is_empty() && (c.is_ascii_whitespace() || matches!(c, '>' | '+' | '~')) {
            return Ok(i);
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '[' => stack.push(']'),
            '(' => stack.push(')'),
            ']' | ')' if stack.pop() != Some(c) => {
                return Err("mismatched SVG selector brackets".into());
            }
            _ => {}
        }
    }
    if quote.is_some() || escaped || !stack.is_empty() {
        return Err("unclosed SVG selector".into());
    }
    Ok(text.len())
}

enum Pseudo {
    Root,
    Empty,
    Only {
        of_type: bool,
    },
    Nth {
        of_type: bool,
        last: bool,
        a: i64,
        b: i64,
    },
}

impl Pseudo {
    fn parse(text: &str) -> Result<(Self, usize), String> {
        let end = text
            .find(|c: char| !(c.is_ascii_alphabetic() || c == '-'))
            .unwrap_or(text.len());
        let name = text[..end].to_ascii_lowercase();
        let of_type = name.ends_with("of-type");
        let last = name.contains("last");
        let simple = match name.as_str() {
            "root" => Some(Self::Root),
            "empty" => Some(Self::Empty),
            "only-child" | "only-of-type" => Some(Self::Only { of_type }),
            "first-child" | "last-child" | "first-of-type" | "last-of-type" => Some(Self::Nth {
                of_type,
                last,
                a: 0,
                b: 1,
            }),
            _ => None,
        };
        if let Some(p) = simple {
            return Ok((p, end));
        }
        if !matches!(
            name.as_str(),
            "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type"
        ) {
            return Err(format!(
                "SVG selector :{name} is not a static structural pseudo-class"
            ));
        }
        let args = text[end..]
            .strip_prefix('(')
            .ok_or("nth selector needs (An+B)")?;
        let close = args.find(')').ok_or("unclosed nth selector")?;
        let (a, b) = nth(&args[..close])?;
        Ok((
            Self::Nth {
                of_type,
                last,
                a,
                b,
            },
            end + close + 2,
        ))
    }

    fn matches(&self, node: Node) -> bool {
        let (of_type, last) = match self {
            Self::Root => return node.parent_element().is_none(),
            Self::Empty => {
                return !node.children().any(|n| {
                    n.is_element() || n.is_text() && !n.text().unwrap_or_default().is_empty()
                });
            }
            Self::Only { of_type } => (*of_type, false),
            Self::Nth { of_type, last, .. } => (*of_type, *last),
        };
        let eligible = |n: &Node| n.is_element() && (!of_type || n.tag_name() == node.tag_name());
        let before = node.prev_siblings().skip(1).filter(eligible).count();
        let after = node.next_siblings().skip(1).filter(eligible).count();
        if matches!(self, Self::Only { .. }) {
            return before == 0 && after == 0;
        }
        let index = (if last { after } else { before }) as i128 + 1;
        let Self::Nth { a, b, .. } = self else {
            unreachable!()
        };
        let delta = index - i128::from(*b);
        *a == 0 && delta == 0
            || *a != 0 && delta % i128::from(*a) == 0 && delta / i128::from(*a) >= 0
    }
}

fn nth(text: &str) -> Result<(i64, i64), String> {
    let text = text.trim().to_ascii_lowercase();
    match text.as_str() {
        "odd" => return Ok((2, 1)),
        "even" => return Ok((2, 0)),
        _ => {}
    }
    let error = || format!("invalid An+B selector {text:?}");
    let Some((a, b)) = text.split_once('n') else {
        return text.parse().map(|b| (0, b)).map_err(|_| error());
    };
    let a = match a {
        "" | "+" => 1,
        "-" => -1,
        _ => a.parse().map_err(|_| error())?,
    };
    let b = b.trim();
    let b = if b.is_empty() {
        0
    } else {
        let (sign, digits) = if let Some(v) = b.strip_prefix('+') {
            ("+", v)
        } else if let Some(v) = b.strip_prefix('-') {
            ("-", v)
        } else {
            return Err(error());
        };
        let digits = digits.trim();
        if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
            return Err(error());
        }
        format!("{sign}{digits}").parse().map_err(|_| error())?
    };
    Ok((a, b))
}

struct Attribute {
    name: String,
    test: Option<(String, String)>,
}

impl Attribute {
    fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let end = text
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
            .unwrap_or(text.len());
        if end == 0 {
            return Err("attribute selector needs an unqualified name".into());
        }
        let name = text[..end].to_string();
        let rest = text[end..].trim();
        if rest.is_empty() {
            return Ok(Self { name, test: None });
        }
        let op = ["~=", "|=", "^=", "$=", "*=", "="]
            .into_iter()
            .find(|op| rest.starts_with(op))
            .ok_or("unsupported SVG attribute selector operator")?;
        let value = rest[op.len()..].trim();
        let value = if let Some(q @ ('\'' | '"')) = value.chars().next() {
            value
                .strip_prefix(q)
                .and_then(|v| v.strip_suffix(q))
                .filter(|v| !v.contains(q) && !v.contains('\\'))
                .ok_or("attribute selector needs a quoted string without trailing flags")?
                .to_string()
        } else if !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            value.to_string()
        } else {
            return Err("attribute selector needs an identifier or quoted string".into());
        };
        Ok(Self {
            name,
            test: Some((op.to_string(), value)),
        })
    }

    fn matches(&self, node: Node) -> bool {
        let Some(value) = node
            .attributes()
            .find(|a| a.namespace().is_none() && a.name() == self.name)
            .map(|a| a.value())
        else {
            return false;
        };
        let Some((op, expected)) = &self.test else {
            return true;
        };
        match op.as_str() {
            "=" => value == expected,
            "~=" => !expected.is_empty() && value.split_ascii_whitespace().any(|v| v == expected),
            "|=" => {
                value == expected
                    || value
                        .strip_prefix(expected)
                        .is_some_and(|v| v.starts_with('-'))
            }
            "^=" => !expected.is_empty() && value.starts_with(expected),
            "$=" => !expected.is_empty() && value.ends_with(expected),
            "*=" => !expected.is_empty() && value.contains(expected),
            _ => unreachable!(),
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
            '(' | '[' => depth += 1,
            ')' | ']' if depth > 0 => depth -= 1,
            ')' | ']' | '{' | '}' => return Err(format!("unexpected {c} in SVG CSS")),
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
    fn structural_selectors_count_elements_and_types_in_both_directions() {
        let xml = roxmltree::Document::parse(r#"<svg xmlns="http://www.w3.org/2000/svg"><g id="many">text<!--comment--><rect id="a"/><circle id="b"/><rect id="c"/><rect id="d"/></g><g id="one"><rect id="e"/></g><g id="space"> </g><g id="empty"><!--comment--></g></svg>"#).unwrap();
        for (selector, expected) in [
            (":root", vec![""]),
            ("#many > :first-child", vec!["a"]),
            ("#many > :last-child", vec!["d"]),
            ("#many > :nth-child(2n + 1)", vec!["a", "c"]),
            ("#many > :nth-last-child(-n+2)", vec!["c", "d"]),
            ("#many > rect:nth-of-type(2)", vec!["c"]),
            ("#many > rect:nth-last-of-type(2)", vec!["c"]),
            ("#many > rect:first-of-type", vec!["a"]),
            ("#many > rect:last-of-type", vec!["d"]),
            (":only-child", vec!["", "e"]),
            ("#many > :only-of-type", vec!["b"]),
            ("g:empty", vec!["empty"]),
            ("#many > :nth-child(0n+2)", vec!["b"]),
        ] {
            let selector = Selector::parse(selector).unwrap();
            let found: Vec<_> = xml
                .descendants()
                .filter(|n| selector.matches(*n))
                .map(|n| n.attribute("id").unwrap_or(""))
                .collect();
            assert_eq!(found, expected);
        }
        for (value, expected) in [
            ("odd", (2, 1)),
            ("EVEN", (2, 0)),
            ("-n + 3", (-1, 3)),
            ("+2", (0, 2)),
            ("2n-1", (2, -1)),
        ] {
            assert_eq!(nth(value).unwrap(), expected);
        }
        for value in [
            "n中文",
            "2 n+1",
            "n1",
            "n++1",
            "n+1 2",
            "2n of rect",
            "1.5",
            "999999999999999999999999",
        ] {
            assert!(nth(value).is_err(), "{value}");
        }
        for selector in [
            ":hover",
            ":not(rect)",
            ":nth-child()",
            ":nth-child(2n",
            ":first-child(2)",
        ] {
            assert!(Selector::parse(selector).is_err(), "{selector}");
        }
    }

    #[test]
    fn attributes_match_exact_words_languages_and_substrings() {
        let xml = roxmltree::Document::parse(r#"<svg xmlns="http://www.w3.org/2000/svg"><rect data-name="Alpha beta,gamma > end" data-lang="en-US" data-empty=""/><rect/></svg>"#).unwrap();
        let node = xml
            .descendants()
            .find(|n| n.has_tag_name((SVG, "rect")))
            .unwrap();
        for selector in [
            "[data-name]",
            "rect[data-name~='Alpha']",
            "[data-lang|=en]",
            "[data-name^=Alpha]",
            "[data-name$=end]",
            "[data-name*='beta,gamma >']",
            "[data-empty='']",
            "svg > [data-lang][data-empty]",
        ] {
            let s = Selector::parse(selector).unwrap();
            assert!(s.matches(node), "{selector}");
            assert!(
                !s.matches(node.next_sibling_element().unwrap()),
                "{selector}"
            );
        }
        for selector in [
            "[data-name~=alpha]",
            "[data-name~='Alpha beta,gamma']",
            "[missing]",
            "[data-name*='']",
            "[data-name^='']",
            "[data-name$='']",
            "[data-name~='']",
        ] {
            assert!(
                !Selector::parse(selector).unwrap().matches(node),
                "{selector}"
            );
        }
        assert_eq!(
            Selector::parse("rect[data-name][data-lang]")
                .unwrap()
                .specificity(),
            [0, 2, 1]
        );
        assert_eq!(split("[data-name='a,b'],rect", ',').unwrap().len(), 2);
        for bad in ["[x", "[x=]", "[x!=a]", "[x='a' i]", "[x='a' b='c']"] {
            assert!(Selector::parse(bad).is_err(), "{bad}");
        }
    }

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
        for selector in ["rect:hover", "[fill=]", "@media all", ".a >", ""] {
            assert!(Selector::parse(selector).is_err(), "{selector}");
        }
        assert!(declarations("fill: rgb(1,2,3").is_err());
        assert!(comments("/* open").is_err());
    }
}
