use ariadne::{Color, Label, Report, ReportKind, Source};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticKind {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    /// Stable code from [`crate::diagnostic_codes::CODES`] (e.g. `E030`).
    pub code: &'static str,
    pub message: String,
    pub file: String,
    pub span: Range<usize>,
    pub label: String,
    pub help: Option<String>,
}

impl Diagnostic {
    /// Build an error diagnostic. `code` must be registered in
    /// [`crate::diagnostic_codes::CODES`].
    pub fn error(
        code: &'static str,
        file: impl Into<String>,
        message: impl Into<String>,
        span: Range<usize>,
        label: impl Into<String>,
    ) -> Self {
        Self {
            kind: DiagnosticKind::Error,
            code,
            message: message.into(),
            file: file.into(),
            span,
            label: label.into(),
            help: None,
        }
    }

    /// Build a warning diagnostic. `code` must be registered in
    /// [`crate::diagnostic_codes::CODES`].
    pub fn warning(
        code: &'static str,
        file: impl Into<String>,
        message: impl Into<String>,
        span: Range<usize>,
        label: impl Into<String>,
    ) -> Self {
        Self {
            kind: DiagnosticKind::Warning,
            code,
            message: message.into(),
            file: file.into(),
            span,
            label: label.into(),
            help: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn render(&self, source: &str) {
        let kind = match self.kind {
            DiagnosticKind::Error => ReportKind::Error,
            DiagnosticKind::Warning => ReportKind::Warning,
        };

        let span = self.span.clone();
        // Clamp span to source length to avoid panics
        let clamped = span.start.min(source.len())
            ..span.end.min(source.len()).max(span.start.min(source.len()));

        let mut builder = Report::build(kind, clamped.clone())
            .with_message(&self.message)
            .with_label(
                Label::new(clamped)
                    .with_message(&self.label)
                    .with_color(Color::Red),
            );

        if let Some(help) = &self.help {
            builder = builder.with_help(help);
        }

        let _ = builder.finish().eprint(Source::from(source));
    }
}

pub fn render_diagnostics(source: &str, diagnostics: &[Diagnostic]) {
    for diag in diagnostics {
        diag.render(source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_creates_warning_kind() {
        let diag = Diagnostic::warning("W040", "test.forge", "unused state", 0..5, "not reachable");
        assert!(matches!(diag.kind, DiagnosticKind::Warning));
        assert_eq!(diag.code, "W040");
        assert_eq!(diag.message, "unused state");
    }
}
