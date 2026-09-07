//! NFC-normalized CJK bigrams for FTS5, shared by indexing and queries. Index CJK runs as
//! overlapping pairs and ASCII words in lowercase. Query multi-character runs as adjacent phrases
//! and single characters as prefixes; preserve quoted phrase semantics.

use unicode_normalization::UnicodeNormalization;

/// Recognize Han characters, extension A, and kana.
fn is_cjk(c: char) -> bool {
    matches!(u32::from(c),
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x3040..=0x30FF)
}

/// Lexical run: CJK characters or an ASCII word.
enum Run {
    Cjk(Vec<char>),
    Word(String),
}

fn runs(text: &str) -> Vec<Run> {
    let mut out = Vec::new();
    let mut cjk: Vec<char> = Vec::new();
    let mut word = String::new();
    for c in text.nfc() {
        if is_cjk(c) {
            if !word.is_empty() {
                out.push(Run::Word(std::mem::take(&mut word)));
            }
            cjk.push(c);
        } else if c.is_alphanumeric() {
            if !cjk.is_empty() {
                out.push(Run::Cjk(std::mem::take(&mut cjk)));
            }
            word.extend(c.to_lowercase());
        } else {
            if !cjk.is_empty() {
                out.push(Run::Cjk(std::mem::take(&mut cjk)));
            }
            if !word.is_empty() {
                out.push(Run::Word(std::mem::take(&mut word)));
            }
        }
    }
    if !cjk.is_empty() {
        out.push(Run::Cjk(cjk));
    }
    if !word.is_empty() {
        out.push(Run::Word(word));
    }
    out
}

fn bigrams(chars: &[char]) -> Vec<String> {
    if chars.len() == 1 {
        return vec![chars[0].to_string()];
    }
    chars.windows(2).map(|w| w.iter().collect()).collect()
}

/// Transform text into the indexed token stream.
pub fn index_text(text: &str) -> String {
    let mut toks = Vec::new();
    for r in runs(text) {
        match r {
            Run::Cjk(chars) => toks.extend(bigrams(&chars)),
            Run::Word(w) => toks.push(w),
        }
    }
    toks.join(" ")
}

/// Combine multiple text fields into one index stream.
pub fn index_texts<'a>(texts: impl IntoIterator<Item = &'a str>) -> String {
    texts
        .into_iter()
        .map(index_text)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Convert a query to an FTS5 MATCH expression with implicit AND. Return `None` when no searchable
/// terms remain.
pub fn query_expr(query: &str) -> Option<String> {
    let mut parts = Vec::new();
    // Extract phrases using ASCII or typographic quotation marks.
    let normalized: String = query.nfc().collect();
    let mut rest = normalized.as_str();
    let mut segments: Vec<(bool, String)> = Vec::new(); // (is_phrase, text)
    while let Some(start) = rest.find(['"', '「', '“']) {
        let (before, after) = rest.split_at(start);
        if !before.trim().is_empty() {
            segments.push((false, before.to_owned()));
        }
        let open = after.chars().next().expect("nonempty");
        let close = match open {
            '「' => '」',
            '“' => '”',
            _ => '"',
        };
        let inner = &after[open.len_utf8()..];
        match inner.find(close) {
            Some(end) => {
                segments.push((true, inner[..end].to_owned()));
                rest = &inner[end + close.len_utf8()..];
            }
            None => {
                segments.push((false, inner.to_owned()));
                rest = "";
            }
        }
    }
    if !rest.trim().is_empty() {
        segments.push((false, rest.to_owned()));
    }

    for (is_phrase, seg) in segments {
        if is_phrase {
            let toks = index_text(&seg);
            if !toks.is_empty() {
                parts.push(format!("\"{}\"", toks.replace('"', "")));
            }
            continue;
        }
        for term in seg.split_whitespace() {
            for r in runs(term) {
                match r {
                    Run::Cjk(chars) if chars.len() == 1 => {
                        // A single character matches bigram tokens beginning with that character.
                        parts.push(format!("\"{}\" *", chars[0]));
                    }
                    Run::Cjk(chars) => {
                        // An adjacent bigram phrase preserves substring matching.
                        parts.push(format!("\"{}\"", bigrams(&chars).join(" ")));
                    }
                    Run::Word(w) => {
                        parts.push(format!("\"{}\"", w.replace('"', "")));
                    }
                }
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" AND "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_pure_cjk_bigrams() {
        assert_eq!(index_text("小明去公园"), "小明 明去 去公 公园");
        assert_eq!(index_text("中"), "中");
    }

    #[test]
    fn index_mixed_cjk_ascii() {
        assert_eq!(index_text("BGM 卡点 v2"), "bgm 卡点 v2");
        assert_eq!(index_text("欢快BGM"), "欢快 bgm");
    }

    #[test]
    fn query_multi_char_is_phrase() {
        assert_eq!(query_expr("小明 公园").unwrap(), "\"小明\" AND \"公园\"");
        assert_eq!(query_expr("世纪公园").unwrap(), "\"世纪 纪公 公园\"");
    }

    #[test]
    fn query_single_char_prefix() {
        assert_eq!(query_expr("明").unwrap(), "\"明\" *");
    }

    #[test]
    fn query_quoted_phrase() {
        assert_eq!(query_expr("\"世纪公园\"").unwrap(), "\"世纪 纪公 公园\"");
        assert_eq!(query_expr("「开场 候选」").unwrap(), "\"开场 候选\"");
    }

    #[test]
    fn query_empty_and_punct() {
        assert!(query_expr("").is_none());
        assert!(query_expr("!!!").is_none());
    }
}
