//! Motion-owned diagnostics avoid a reverse dependency on the compiler. Codes determine whether
//! input is illegal or valid but unsupported. Paths use slash-separated kind:value segments; an
//! empty path indicates a global issue.

use serde::{Deserialize, Serialize};

/// Diagnostic classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum DiagClass {
    /// Invalid syntax or contract input.
    Illegal,
    /// Valid input unsupported by the implementation.
    Unsupported,
}

/// Closed diagnostic code vocabulary. Add an explicit classification for every new code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
pub enum DiagCode {
    /// Cycle in the signal dependency graph.
    SignalCycle,
    /// Signal references a missing provider clip.
    SignalProviderMissing,
    /// Signal references its own clip, whose location depends on its unfinished layout.
    SignalSelfReference,
    /// Duplicate clip ID in a plan.
    DuplicateClip,
    /// Invalid artifact structure.
    ArtifactInvalid,

    // Compilation diagnostics.
    /// Source parsing failed.
    SyntaxError,
    /// Invalid module structure or declarations.
    ModuleShape,
    /// Syntax outside the supported grammar.
    GrammarForbidden,
    /// Reference to an undeclared identifier.
    UnknownIdentifier,
    /// Invalid context field path.
    UnknownContextPath,
    /// Reference to an undeclared prop.
    UnknownProp,
    /// Forbidden wall-clock animation, responsive prefix, or interaction variant.
    TailwindForbidden,
    /// Valid Tailwind utility absent from the versioned Valle catalog.
    TailwindUnsupported,
    /// Forbidden nondeterministic sandbox access, including clocks, unseeded randomness, and I/O.
    SandboxForbidden,
    /// Static evaluation threw or returned a non-JSON value.
    StaticEvalFailed,
    /// Prepare-time builtin explicitly rejected its arguments. Preserve this diagnostic instead of
    /// treating it as ordinary failed constant folding.
    BuiltinRejected,
    /// Valid syntax not supported by the current implementation.
    UnsupportedSyntax,
    /// Motion Glass syntax without production admission.
    MotionGlassNotAdmitted,
}

impl DiagCode {
    pub fn class(self) -> DiagClass {
        match self {
            DiagCode::SignalCycle
            | DiagCode::SignalProviderMissing
            | DiagCode::SignalSelfReference
            | DiagCode::DuplicateClip
            | DiagCode::ArtifactInvalid
            | DiagCode::SyntaxError
            | DiagCode::ModuleShape
            | DiagCode::GrammarForbidden
            | DiagCode::UnknownIdentifier
            | DiagCode::UnknownContextPath
            | DiagCode::UnknownProp
            | DiagCode::TailwindForbidden
            | DiagCode::SandboxForbidden
            | DiagCode::StaticEvalFailed
            | DiagCode::BuiltinRejected
            | DiagCode::MotionGlassNotAdmitted => DiagClass::Illegal,
            DiagCode::TailwindUnsupported | DiagCode::UnsupportedSyntax => DiagClass::Unsupported,
        }
    }

    /// Stable kebab-case code name matching serialization.
    pub fn as_str(self) -> &'static str {
        match self {
            DiagCode::SignalCycle => "signal-cycle",
            DiagCode::SignalProviderMissing => "signal-provider-missing",
            DiagCode::SignalSelfReference => "signal-self-reference",
            DiagCode::DuplicateClip => "duplicate-clip",
            DiagCode::ArtifactInvalid => "artifact-invalid",
            DiagCode::SyntaxError => "syntax-error",
            DiagCode::ModuleShape => "module-shape",
            DiagCode::GrammarForbidden => "grammar-forbidden",
            DiagCode::UnknownIdentifier => "unknown-identifier",
            DiagCode::UnknownContextPath => "unknown-context-path",
            DiagCode::UnknownProp => "unknown-prop",
            DiagCode::TailwindForbidden => "tailwind-forbidden",
            DiagCode::TailwindUnsupported => "tailwind-unsupported",
            DiagCode::SandboxForbidden => "sandbox-forbidden",
            DiagCode::StaticEvalFailed => "static-eval-failed",
            DiagCode::BuiltinRejected => "builtin-rejected",
            DiagCode::UnsupportedSyntax => "unsupported-syntax",
            DiagCode::MotionGlassNotAdmitted => "motion-glass-not-admitted",
        }
    }
}

/// Diagnostic code, location, and human-readable message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionDiagnostic {
    pub code: DiagCode,
    /// Slash-separated diagnostic path, empty for global issues.
    pub path: String,
    pub message: String,
}

/// Plan errors share the Motion diagnostic representation.
pub type PlanError = MotionDiagnostic;

impl MotionDiagnostic {
    pub fn new(code: DiagCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        MotionDiagnostic {
            code,
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn class(&self) -> DiagClass {
        self.code.class()
    }
}

impl core::fmt::Display for MotionDiagnostic {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.path.is_empty() {
            write!(f, "[{}] {}", self.code.as_str(), self.message)
        } else {
            write!(
                f,
                "[{}] {}: {}",
                self.code.as_str(),
                self.path,
                self.message
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_is_a_property_of_the_code() {
        // Every code must have an explicit classification.
        for c in [
            DiagCode::SignalCycle,
            DiagCode::SignalProviderMissing,
            DiagCode::SignalSelfReference,
            DiagCode::DuplicateClip,
            DiagCode::ArtifactInvalid,
            DiagCode::SyntaxError,
            DiagCode::ModuleShape,
            DiagCode::GrammarForbidden,
            DiagCode::UnknownIdentifier,
            DiagCode::UnknownContextPath,
            DiagCode::UnknownProp,
            DiagCode::TailwindForbidden,
            DiagCode::SandboxForbidden,
            DiagCode::StaticEvalFailed,
            DiagCode::BuiltinRejected,
        ] {
            assert_eq!(c.class(), DiagClass::Illegal, "{}", c.as_str());
        }
        // Unsupported syntax remains distinguishable from illegal input.
        assert_eq!(DiagCode::UnsupportedSyntax.class(), DiagClass::Unsupported);
        assert_eq!(
            DiagCode::TailwindUnsupported.class(),
            DiagClass::Unsupported
        );
    }

    #[test]
    fn code_string_and_serde_agree() {
        for c in [DiagCode::SignalCycle, DiagCode::SignalProviderMissing] {
            let json = serde_json::to_string(&c).unwrap();
            assert_eq!(json, format!("\"{}\"", c.as_str()));
            assert_eq!(serde_json::from_str::<DiagCode>(&json).unwrap(), c);
        }
        // Reject unknown diagnostic codes during deserialization.
        assert!(serde_json::from_str::<DiagCode>(r#""whatever""#).is_err());
    }

    #[test]
    fn display_reads_like_a_compiler_message() {
        let d = MotionDiagnostic::new(
            DiagCode::SignalProviderMissing,
            "clip:intro/signal:targetRect",
            "no clip named `chart`",
        );
        assert_eq!(
            d.to_string(),
            "[signal-provider-missing] clip:intro/signal:targetRect: no clip named `chart`"
        );
        let d = MotionDiagnostic::new(DiagCode::SignalCycle, "", "2 clips in a cycle");
        assert_eq!(d.to_string(), "[signal-cycle] 2 clips in a cycle");
    }
}
