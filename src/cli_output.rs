//! Machine-readable CLI output: the JSON envelope, semantic exit codes and
//! `--fields` trimming (#475).
//!
//! Every in-scope subcommand builds one [`Envelope`] and hands it to
//! [`OutputMode::done`], which prints the document to stdout in JSON mode and
//! exits with the status's [`ExitCode`]. Human mode keeps its existing text and
//! only adopts the exit codes.

use std::time::Instant;

use serde::Serialize;
use serde_json::Value;

/// Semantic exit codes. These are part of the CLI contract; agents branch on
/// them instead of parsing text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    Success = 0,
    Error = 1,
    Warning = 2,
    Partial = 3,
    NeedsInput = 10,
    AsyncPending = 11,
}

/// Envelope status. `Pending` is for paths that stop and wait (approval gate,
/// `needs input`); everything else maps to a terminal status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Success,
    Error,
    Warning,
    Pending,
}

impl Status {
    pub fn exit_code(self) -> ExitCode {
        match self {
            Status::Success => ExitCode::Success,
            Status::Error => ExitCode::Error,
            Status::Warning => ExitCode::Warning,
            Status::Pending => ExitCode::NeedsInput,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorInfo {
    #[serde(rename = "type")]
    pub kind: String,
    pub code: Option<String>,
    pub message: String,
    pub suggestion: Option<String>,
}

impl ErrorInfo {
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            code: None,
            message: message.into(),
            suggestion: None,
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Envelope {
    pub status: Status,
    pub command: String,
    pub data: Value,
    pub context: String,
    pub next_steps: Vec<String>,
    pub warnings: Vec<String>,
    /// USD. `Some(0.0)` on paths with no LLM calls, `None` only where the cost
    /// of the run cannot be known.
    pub cost: Option<f64>,
    pub error: Option<ErrorInfo>,
    pub duration_ms: u64,
}

impl Envelope {
    pub fn new(
        command: impl Into<String>,
        status: Status,
        data: Value,
        context: impl Into<String>,
    ) -> Self {
        Self {
            status,
            command: command.into(),
            data,
            context: context.into(),
            next_steps: Vec::new(),
            warnings: Vec::new(),
            cost: Some(0.0),
            error: None,
            duration_ms: 0,
        }
    }

    pub fn success(command: impl Into<String>, data: Value, context: impl Into<String>) -> Self {
        Self::new(command, Status::Success, data, context)
    }

    pub fn error(command: impl Into<String>, error: ErrorInfo, context: impl Into<String>) -> Self {
        let mut env = Self::new(
            command,
            Status::Error,
            Value::Object(Default::default()),
            context,
        );
        env.error = Some(error);
        env
    }

    /// Mark this envelope as needing input from the caller (approval gate).
    pub fn pending(mut self) -> Self {
        self.status = Status::Pending;
        self
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    pub fn with_cost(mut self, cost: f64) -> Self {
        self.cost = Some(cost);
        self
    }

    pub fn with_unknown_cost(mut self) -> Self {
        self.cost = None;
        self
    }

    pub fn with_next_steps(mut self, steps: Vec<String>) -> Self {
        self.next_steps = steps;
        self
    }

    pub fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        self.warnings = warnings;
        self
    }

    pub fn exit_code(&self) -> ExitCode {
        self.status.exit_code()
    }
}

/// How this process was asked to talk to the caller.
pub struct OutputMode {
    pub json: bool,
    pub fields: Vec<String>,
}

impl OutputMode {
    /// `--json` or `FORGE_OUTPUT=json` turns JSON mode on; anything else keeps
    /// the human default.
    pub fn from_flags(json: bool, fields: Vec<String>) -> Self {
        let env_json = std::env::var("FORGE_OUTPUT")
            .map(|v| v == "json")
            .unwrap_or(false);
        Self {
            json: json || env_json,
            fields,
        }
    }

    /// Human-only mode, for shared code paths that `--json` rejects before
    /// reaching them (interactive / long-running commands).
    pub fn human() -> Self {
        Self {
            json: false,
            fields: Vec::new(),
        }
    }

    fn field_filter(&self) -> Option<&[String]> {
        if self.fields.is_empty() {
            None
        } else {
            Some(&self.fields)
        }
    }

    /// Finish a command: JSON mode prints the envelope and exits with the
    /// status's code; human mode exits with the same code, the human text
    /// having already been written by the caller.
    pub fn done(&self, env: &Envelope) -> ! {
        if self.json {
            emit(env, self.field_filter())
        } else {
            exit(env.exit_code())
        }
    }

    /// Report a command failure that surfaced as an `anyhow::Error`.
    pub fn fail(&self, command: &str, e: &anyhow::Error) -> ! {
        let info = classify_error(e);
        let message = info.message.clone();
        if self.json {
            let env = Envelope::error(command, info, format!("{command} failed: {message}"));
            emit(&env, self.field_filter())
        } else {
            eprintln!("Error: {e:?}");
            exit(ExitCode::Error)
        }
    }

    /// Report a command that has no JSON contract (interactive or long-running).
    pub fn unsupported(&self, command: &str) -> ! {
        if self.json {
            let env = Envelope::error(
                command,
                ErrorInfo::new(
                    "unsupported",
                    format!("--json is not supported for {command}"),
                )
                .with_suggestion(format!(
                    "run `forge {command}` in a terminal; it is interactive or long-running"
                )),
                format!("{command} does not support machine-readable output"),
            )
            .with_data(serde_json::json!({"supported": false}));
            emit(&env, self.field_filter())
        } else {
            eprintln!("Error: --json is not supported for {command} (interactive or long-running)");
            exit(ExitCode::Error)
        }
    }
}

/// Print one JSON document to stdout and exit with the status's code.
pub fn emit(env: &Envelope, fields: Option<&[String]>) -> ! {
    let mut env = env.clone();
    env.duration_ms = elapsed_ms();
    env.data = filter_data(&env.data, fields);
    match serde_json::to_string(&env) {
        Ok(doc) => println!("{doc}"),
        Err(e) => {
            eprintln!("Error: could not serialize JSON envelope: {e}");
            exit(ExitCode::Error);
        }
    }
    let _ = std::io::Write::flush(&mut std::io::stdout());
    exit(env.exit_code())
}

/// Exit with a semantic code. Never returns.
pub fn exit(code: ExitCode) -> ! {
    std::process::exit(code as i32)
}

/// Keep only the listed top-level keys of an object `data` payload. Unknown
/// names, non-object payloads and an empty field list leave `data` untouched.
pub fn filter_data(data: &Value, fields: Option<&[String]>) -> Value {
    let Some(fields) = fields.filter(|f| !f.is_empty()) else {
        return data.clone();
    };
    let Value::Object(map) = data else {
        return data.clone();
    };
    let mut filtered = serde_json::Map::new();
    for field in fields {
        if let Some(value) = map.get(field) {
            filtered.insert(field.clone(), value.clone());
        }
    }
    Value::Object(filtered)
}

/// 1-based line and column for a byte offset in `source`. Columns count bytes,
/// matching the byte spans the parser records.
pub fn line_col(source: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(source.len());
    let mut line = 1u32;
    let mut line_start = 0usize;
    for (i, byte) in source.bytes().enumerate() {
        if i >= offset {
            break;
        }
        if byte == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    (line, (offset - line_start) as u32 + 1)
}

/// JSON view of a runtime [`Value`](crate::runtime::confidence::Value).
/// `Unit` becomes `null` — the envelope's "no result" marker.
pub fn value_json(v: &crate::runtime::confidence::Value) -> Value {
    use crate::runtime::confidence::Value as V;
    match v {
        V::Text(s) | V::Html(s) => Value::String(s.clone()),
        V::Number(n) => serde_json::json!(n),
        V::Bool(b) => Value::Bool(*b),
        V::Unit => Value::Null,
        V::List(items) | V::Array(items) => {
            Value::Array(items.iter().map(|i| value_json(&i.value)).collect())
        }
        V::Record(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), value_json(&v.value)))
                .collect(),
        ),
    }
}

/// JSON view of a diagnostic: stable code, severity, file and 1-based line/col
/// computed from the byte span.
pub fn diagnostic_json(d: &crate::diagnostic::Diagnostic, source: &str) -> Value {
    let (line, col) = line_col(source, d.span.start);
    let (end_line, end_col) = line_col(source, d.span.end);
    let severity = match d.kind {
        crate::diagnostic::DiagnosticKind::Error => "error",
        crate::diagnostic::DiagnosticKind::Warning => "warning",
    };
    serde_json::json!({
        "code": d.code,
        "severity": severity,
        "file": d.file,
        "line": line,
        "col": col,
        "end_line": end_line,
        "end_col": end_col,
        "message": d.message,
        "label": d.label,
        "help": d.help,
    })
}

/// `next_steps` for a diagnostic batch: `forge explain <code>` for each distinct
/// error code, capped at five.
pub fn explain_next_steps(diagnostics: &[crate::diagnostic::Diagnostic]) -> Vec<String> {
    let mut steps = Vec::new();
    for d in diagnostics {
        if d.kind != crate::diagnostic::DiagnosticKind::Error {
            continue;
        }
        let step = format!("forge explain {}", d.code);
        if !steps.contains(&step) {
            steps.push(step);
        }
        if steps.len() == 5 {
            break;
        }
    }
    steps
}

/// Map an `anyhow::Error` onto the envelope's error taxonomy. Classification is
/// heuristic (message + downcast) because CLI errors are constructed with
/// `anyhow::bail!` all over `main.rs`.
pub fn classify_error(e: &anyhow::Error) -> ErrorInfo {
    let message = e.to_string();
    let kind = if e.downcast_ref::<std::io::Error>().is_some()
        || message.starts_with("could not read ")
        || message.contains("No such file")
    {
        "io"
    } else if message.contains("parse")
        || message.contains("syntax")
        || message.contains("unexpected token")
    {
        "parse"
    } else if message.contains("config")
        || message.contains("provider")
        || message.contains("api key")
    {
        "config"
    } else {
        "runtime"
    };
    ErrorInfo::new(kind, message)
}

fn elapsed_ms() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn envelope_serializes_expected_shape() {
        let env = Envelope::success("check", json!({"files": ["a.forge"]}), "no problems")
            .with_warnings(vec!["careful".to_string()]);
        let value = serde_json::to_value(&env).unwrap();
        assert_eq!(value["status"], "success");
        assert_eq!(value["command"], "check");
        assert_eq!(value["data"]["files"][0], "a.forge");
        assert_eq!(value["context"], "no problems");
        assert_eq!(value["warnings"][0], "careful");
        assert_eq!(value["next_steps"], json!([]));
        assert_eq!(value["cost"], 0.0);
        assert!(value["error"].is_null());
        assert!(value["duration_ms"].is_number());
    }

    #[test]
    fn error_envelope_carries_typed_error() {
        let env = Envelope::error(
            "run",
            ErrorInfo::new("runtime", "boom").with_code("E020"),
            "run failed",
        );
        let value = serde_json::to_value(&env).unwrap();
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["type"], "runtime");
        assert_eq!(value["error"]["code"], "E020");
        assert_eq!(value["error"]["message"], "boom");
        assert!(value["error"]["suggestion"].is_null());
        // Error envelopes default to a known zero cost; `None` is reserved for
        // paths where LLM spend happened but cannot be measured.
        assert_eq!(value["cost"], 0.0);
        let unknown =
            Envelope::error("send", ErrorInfo::new("runtime", "x"), "y").with_unknown_cost();
        assert!(serde_json::to_value(&unknown).unwrap()["cost"].is_null());
    }

    #[test]
    fn field_filter_keeps_only_listed_keys() {
        let data = json!({"diagnostics": [1], "files": ["a"], "extra": true});
        let fields = vec!["diagnostics".to_string()];
        assert_eq!(
            filter_data(&data, Some(&fields)),
            json!({"diagnostics": [1]})
        );
        // Unknown names are dropped, not invented.
        let unknown = vec!["nope".to_string()];
        assert_eq!(filter_data(&data, Some(&unknown)), json!({}));
        // No filter (or empty filter) leaves data alone.
        assert_eq!(filter_data(&data, None), data);
        assert_eq!(filter_data(&data, Some(&[])), data);
        // Non-object payloads are not filtered.
        let scalar = json!("plain");
        assert_eq!(filter_data(&scalar, Some(&fields)), scalar);
    }

    #[test]
    fn line_col_counts_from_one_across_lines() {
        let source = "one\ntwo\nthree";
        assert_eq!(line_col(source, 0), (1, 1));
        assert_eq!(line_col(source, 2), (1, 3));
        assert_eq!(line_col(source, 4), (2, 1));
        assert_eq!(line_col(source, 6), (2, 3));
        assert_eq!(line_col(source, 8), (3, 1));
        assert_eq!(line_col(source, 13), (3, 6));
        // Past the end clamps instead of panicking.
        assert_eq!(line_col(source, 999), (3, 6));
    }

    #[test]
    fn status_maps_to_semantic_exit_codes() {
        assert_eq!(Status::Success.exit_code(), ExitCode::Success);
        assert_eq!(Status::Error.exit_code(), ExitCode::Error);
        assert_eq!(Status::Warning.exit_code(), ExitCode::Warning);
        assert_eq!(Status::Pending.exit_code(), ExitCode::NeedsInput);
        assert_eq!(ExitCode::Success as i32, 0);
        assert_eq!(ExitCode::Error as i32, 1);
        assert_eq!(ExitCode::Warning as i32, 2);
        assert_eq!(ExitCode::Partial as i32, 3);
        assert_eq!(ExitCode::NeedsInput as i32, 10);
        assert_eq!(ExitCode::AsyncPending as i32, 11);
        let pending = Envelope::success("run", json!({}), "waiting").pending();
        assert_eq!(pending.exit_code(), ExitCode::NeedsInput);
    }

    #[test]
    fn diagnostic_json_reports_line_col_and_next_steps() {
        use crate::diagnostic::Diagnostic;
        let source = "use\n  llm.reason\ntask analyze\n  do\n    result = reason \"x\"\n";
        let start = source.find("reason \"x\"").unwrap();
        let diag = Diagnostic::error(
            "E020",
            "a.forge",
            "uncertain result",
            start..start + 6,
            "here",
        )
        .with_help("wrap it");
        let value = diagnostic_json(&diag, source);
        assert_eq!(value["code"], "E020");
        assert_eq!(value["severity"], "error");
        assert_eq!(value["line"], 5);
        assert_eq!(value["col"], 14);
        assert_eq!(value["help"], "wrap it");

        let warning =
            Diagnostic::warning("W040", "a.forge", "unused", 0..3, "unused").with_help("drop it");
        assert_eq!(
            explain_next_steps(&[diag, warning]),
            vec!["forge explain E020".to_string()]
        );
    }

    #[test]
    fn classify_error_separates_io_parse_config_runtime() {
        let io = anyhow::anyhow!("could not read missing.forge: No such file or directory");
        assert_eq!(classify_error(&io).kind, "io");
        let parse = anyhow::anyhow!("parse error at line 3");
        assert_eq!(classify_error(&parse).kind, "parse");
        let config = anyhow::anyhow!("provider setup failed: no api key");
        assert_eq!(classify_error(&config).kind, "config");
        let runtime = anyhow::anyhow!("runtime error: division by zero");
        assert_eq!(classify_error(&runtime).kind, "runtime");
    }
}
