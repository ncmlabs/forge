use ariadne::{Color, Config, Label, Report, ReportKind, Source};
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

    /// Render to stderr, coloured only when stderr is an interactive terminal
    /// and `NO_COLOR` is unset (#474).
    pub fn render(&self, source: &str) {
        let color = std::io::IsTerminal::is_terminal(&std::io::stderr())
            && std::env::var_os("NO_COLOR").is_none();
        let _ = self.render_to(source, color, &mut std::io::stderr());
    }

    /// Render this diagnostic against `source`. The real file path is used as
    /// the ariadne source id, so output names the file instead of `<unknown>`.
    pub fn render_to(
        &self,
        source: &str,
        color: bool,
        out: &mut impl std::io::Write,
    ) -> std::io::Result<()> {
        let kind = match self.kind {
            DiagnosticKind::Error => ReportKind::Error,
            DiagnosticKind::Warning => ReportKind::Warning,
        };

        let span = self.span.clone();
        // Clamp span to source length to avoid panics
        let clamped = span.start.min(source.len())
            ..span.end.min(source.len()).max(span.start.min(source.len()));

        let mut builder = Report::build(kind, (self.file.as_str(), clamped.clone()))
            .with_code(self.code)
            .with_message(&self.message)
            .with_config(Config::default().with_color(color))
            .with_label(
                Label::new((self.file.as_str(), clamped))
                    .with_message(&self.label)
                    .with_color(Color::Red),
            );

        if let Some(help) = &self.help {
            builder = builder.with_help(help);
        }

        builder
            .finish()
            .write((self.file.as_str(), Source::from(source)), out)
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

    #[test]
    fn render_to_names_the_real_file_and_code_without_ansi() {
        let source = "agent a\n  on start\n    say \"hi\"\n";
        let diag = Diagnostic::error(
            "E030",
            "src/example.forge",
            "pure function `f` cannot use `reason`",
            0..7,
            "here",
        );
        let mut out = Vec::new();
        diag.render_to(source, false, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("src/example.forge"), "output: {text}");
        assert!(text.contains("E030"), "output: {text}");
        assert!(!text.contains('\x1b'), "output has ANSI: {text:?}");
    }
}
