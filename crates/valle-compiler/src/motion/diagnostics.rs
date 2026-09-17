//! Source-span projection and compiler diagnostic conversion.

use super::*;

pub(super) fn diagnostic_at(
    source: &str,
    code: DiagCode,
    span: Span,
    message: impl Into<String>,
) -> CompilerDiagnostic {
    let source_span = source_span_at(source, span);
    CompilerDiagnostic {
        class: code.class(),
        code,
        span: source_span,
        source_path: None,
        node_path: None,
        utility: None,
        style: None,
        css_rule: None,
        message: message.into(),
    }
}

impl Compiler<'_> {
    pub(super) fn style_diagnostic(
        &mut self,
        span: Span,
        node_path: Option<&str>,
        issue: valle_motion::style::StyleIssue,
    ) {
        let mut diagnostic = diagnostic_at(self.source, issue.code(), span, issue.to_string());
        diagnostic.node_path = node_path.map(str::to_owned);
        diagnostic.style = Some(issue);
        self.push_diagnostic(diagnostic);
    }
}

pub(super) fn source_span_at(source: &str, span: Span) -> SourceSpan {
    let start = (span.start as usize).min(source.len());
    let prefix = &source[..start];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32 + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, tail)| tail)
        .chars()
        .count() as u32
        + 1;
    SourceSpan {
        start: span.start,
        end: span.end,
        line,
        column,
    }
}

pub(super) fn named_control_span(source: &str, container: Span, name: &str) -> Span {
    let start = container.start as usize;
    let end = (container.end as usize).min(source.len());
    source
        .get(start..end)
        .and_then(|body| body.find(name))
        .map_or(container, |offset| {
            Span::new(
                (start + offset) as u32,
                (start + offset + name.len()) as u32,
            )
        })
}

pub(super) fn from_motion_diagnostic(
    source: &str,
    diagnostic: MotionDiagnostic,
    span: Span,
) -> CompilerDiagnostic {
    diagnostic_at(source, diagnostic.code, span, diagnostic.message)
}

pub(super) fn forbidden_span(source: &str, message: &str) -> Span {
    let name = message
        .strip_prefix('`')
        .and_then(|message| message.split_once('`'))
        .map(|(name, _)| name);
    name.and_then(|name| source.find(name).map(|start| (start, name.len())))
        .map_or(Span::new(0, 0), |(start, len)| {
            Span::new(start as u32, (start + len) as u32)
        })
}
