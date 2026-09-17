//! Decode Tailwind arbitrary values before shared CSS parsing. Brackets/quotes are
//! structural; escaped underscores, URL bodies and variable names retain their meaning.
use super::TailwindClassError;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Url,
    VariableName,
}
struct Frame {
    close: char,
    mode: Mode,
    math: bool,
}

pub(super) fn decode(raw: &str) -> Result<String, TailwindClassError> {
    let fail = TailwindClassError::Syntax(
        "arbitrary values require balanced brackets, quotes and escapes, and cannot contain unquoted declarations",
    );
    let chars: Vec<_> = raw.chars().collect();
    let mut result = String::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut quote = None;
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        let mode = stack.last().map_or(Mode::Normal, |frame| frame.mode);
        let math = stack.last().is_some_and(|frame| frame.math);
        if ch == '\\' {
            let next = *chars.get(at + 1).ok_or_else(|| fail.clone())?;
            if next != '_' || mode == Mode::Url {
                result.push(ch);
            }
            result.push(next);
            at += 2;
            continue;
        }
        if let Some(end) = quote {
            if ch == end {
                quote = None;
            }
            result.push(if ch == '_' && mode == Mode::Normal {
                ' '
            } else {
                ch
            });
            at += 1;
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            result.push(ch);
        } else if ch == '/' && chars.get(at + 1) == Some(&'*') {
            let end = (at + 2..chars.len().saturating_sub(1))
                .find(|&i| chars[i] == '*' && chars[i + 1] == '/')
                .ok_or_else(|| fail.clone())?;
            result.extend(&chars[at..end + 2]);
            at = end + 2;
            continue;
        } else if matches!(ch, '(' | '[') {
            if stack.len() >= 64 {
                return Err(fail.clone());
            }
            let start = chars[..at]
                .iter()
                .rposition(|c| !c.is_ascii_alphanumeric() && !matches!(c, '-' | '_'))
                .map_or(0, |i| i + 1);
            let function: String = if ch == '(' {
                chars[start..at].iter().collect()
            } else {
                String::new()
            };
            let next_mode = if mode == Mode::Url || function == "url" || function.ends_with("_url")
            {
                Mode::Url
            } else if matches!(function.as_str(), "var" | "theme")
                || function.ends_with("_var")
                || function.ends_with("_theme")
            {
                Mode::VariableName
            } else {
                Mode::Normal
            };
            let next_math = ch == '('
                && (matches!(
                    function.as_str(),
                    "calc"
                        | "min"
                        | "max"
                        | "clamp"
                        | "mod"
                        | "rem"
                        | "sin"
                        | "cos"
                        | "tan"
                        | "asin"
                        | "acos"
                        | "atan"
                        | "atan2"
                        | "pow"
                        | "sqrt"
                        | "hypot"
                        | "log"
                        | "exp"
                        | "round"
                ) || function.is_empty() && math);
            stack.push(Frame {
                close: if ch == '(' { ')' } else { ']' },
                mode: next_mode,
                math: next_math,
            });
            result.push(ch);
        } else if matches!(ch, ')' | ']') {
            if !stack.pop().is_some_and(|frame| frame.close == ch) {
                return Err(fail.clone());
            }
            result.push(ch);
        } else if matches!(ch, '{' | '}' | ';') && mode != Mode::Url {
            return Err(fail.clone());
        } else if ch == ',' {
            if let Some(frame) = stack.last_mut()
                && frame.mode == Mode::VariableName
            {
                frame.mode = Mode::Normal;
            }
            result.push(ch);
            if math {
                result.push(' ');
            }
        } else if math && matches!(ch, '+' | '-' | '*' | '/') {
            let before = result.trim_end();
            let mut previous = before.chars().rev();
            let prev = previous.next();
            let exponent = matches!(prev, Some('e' | 'E'))
                && previous.next().is_some_and(|c| c.is_ascii_digit());
            let unary = matches!(prev, None | Some('(' | ',' | '+' | '-' | '*' | '/'));
            // Hyphens inside identifiers remain identifiers; numeric units and closing
            // parentheses indicate operands, including `100%-20px` and nested math.
            let word = before
                .rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '.')
                .next()
                .unwrap_or("");
            let operand = prev == Some(')')
                || prev == Some('%')
                || word.starts_with(|c: char| c.is_ascii_digit() || c == '.');
            if !exponent
                && !unary
                && (operand
                    || chars.get(at + 1).is_some_and(|c| {
                        c.is_ascii_digit() || matches!(c, '(' | '+' | '-' | '*' | '/')
                    }))
            {
                if !result.ends_with(' ') {
                    result.push(' ');
                }
                result.push(ch);
                result.push(' ');
            } else {
                result.push(ch);
            }
        } else {
            let ch = if ch == '_' && mode == Mode::Normal {
                ' '
            } else {
                ch
            };
            if !(math && ch == ' ' && result.ends_with(' ')) {
                result.push(ch);
            }
        }
        at += 1;
    }
    if quote.is_some() || !stack.is_empty() || result.trim().is_empty() {
        return Err(fail.clone());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    /// The decoding rules the authoring surface relies on: brackets, quotes, escapes and
    /// underscore-to-space happen inside an arbitrary value, and nothing else.
    #[test]
    fn arbitrary_values_decode_brackets_quotes_escapes_and_underscores() {
        for (raw, expected) in [
            ("10px", "10px"),
            // `decode` receives the value *inside* the brackets; brackets themselves are the
            // caller's syntax.
            ("10px_20px", "10px 20px"),
            ("url(a_b.png)", "url(a_b.png)"),
            ("'A B'", "'A B'"),
            ("calc(100%_-_2rem)", "calc(100% - 2rem)"),
            ("var(--x)", "var(--x)"),
            (r"a\_b", "a_b"),
            ("theme(colors.red.500)", "theme(colors.red.500)"),
        ] {
            assert_eq!(
                super::decode(raw).unwrap_or_else(|error| panic!("{raw}: {error:?}")),
                expected,
                "{raw}"
            );
        }
        for invalid in ["", "  ", "'a", "a\\", "10px 20px/*", "[10px"] {
            assert!(
                super::decode(invalid).is_err(),
                "{invalid} must be rejected"
            );
        }
    }
}
