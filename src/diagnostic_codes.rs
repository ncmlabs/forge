//! Stable diagnostic codes (#474).
//!
//! Every `Diagnostic` carries a code from [`CODES`]. Codes are stable: they are
//! part of the CLI contract (`forge explain <code>`, conformance `error_code`)
//! and must never be renumbered or reused. Ranges are reserved per checker:
//!
//! | Range       | Source                |
//! |-------------|-----------------------|
//! | E001–E009   | parser                |
//! | E010–E019   | resolver              |
//! | E020–E029   | uncertain checker     |
//! | E030–E039   | pure checker          |
//! | E040–E049   | states checker        |
//! | E050–E069   | boundary checker      |
//! | E070–E079   | requires checker      |
//! | E080–E089   | spawn checker         |
//! | E090–E099   | warden checker        |
//! | E100–E109   | allows checker        |
//! | E110–E129   | schedule checker      |
//! | E130–E139   | correlate checker     |
//! | E140–E149   | webhook checker       |
//!
//! Warnings use the same range prefixed with `W` instead of `E`.

pub struct CodeInfo {
    pub code: &'static str,
    pub title: &'static str,
    pub explain: &'static str,
}

pub const CODES: &[CodeInfo] = &[
    CodeInfo {
        code: "E001",
        title: "parse error",
        explain: r#"The parser could not build a syntax tree for this file. The message names the token the grammar expected at the reported position.

Wrong:
pure broken
  do
    give 1 +

Right:
pure fixed
  do
    give 1 + 2"#,
    },
    CodeInfo {
        code: "E002",
        title: "internal parser error",
        explain: r#"The parser failed internally while building the AST. This is a FORGE bug, not a problem with the program; please report the source that triggered it.

Wrong:
task broken
  needs x: Text

Right:
task fixed
  needs x: Text
  gives Text
  do
    give x"#,
    },
    CodeInfo {
        code: "E010",
        title: "unknown capability",
        explain: r#"A capability name is not in the capability registry. Builtins are `llm.*`, `web.*`, `file.*` and `data.*`; skill capabilities must be declared with `use` or by a project manifest.

Wrong:
agent a
  on start(msg: Text)
    give llm.think msg

Right:
use
  llm.reason

agent a
  on start(msg: Text)
    give reason msg"#,
    },
    CodeInfo {
        code: "E011",
        title: "composition type mismatch",
        explain: r#"Two chained capabilities disagree on types: the output of the left side is not accepted by the right side.

Wrong:
task bad
  needs n: Number
  gives Text
  do
    give reason "score {n}" -> classify n

Right:
task good
  needs n: Number
  gives Text
  do
    give reason "score {n}" -> classify "is this good?""#,
    },
    CodeInfo {
        code: "E012",
        title: "capability argument count mismatch",
        explain: r#"A capability is called with the wrong number of arguments; the message states the declared arity.

Wrong:
use
  llm.reason

agent a
  on start(msg: Text)
    give reason msg "and more"

Right:
use
  llm.reason

agent a
  on start(msg: Text)
    give reason msg"#,
    },
    CodeInfo {
        code: "E013",
        title: "capability argument type mismatch",
        explain: r#"An argument's type does not match the declared parameter type of the capability.

Wrong:
agent a
  on start(count: Number)
    give reason count

Right:
agent a
  on start(count: Number)
    give reason "count is {count}""#,
    },
    CodeInfo {
        code: "E020",
        title: "unhandled uncertain value",
        explain: r#"A value produced by an oracle (`recall`, `reason`, `classify`, ...) may be uncertain and must be dispatched with `when` or `match` before it is used.

Wrong:
task bad
  needs q: Text
  gives Text
  do
    prior = recall "{q}"
    give prior

Right:
task good
  needs q: Text
  gives Text
  do
    prior = recall "{q}"
    when prior.sure -> give prior
    else -> give "unknown""#,
    },
    CodeInfo {
        code: "E021",
        title: "inline oracle result given without dispatch",
        explain: r#"An oracle call is handed straight to `give`/`say` without a confidence dispatch, so the uncertain value would escape the agent unconverted.

Wrong:
task bad
  needs q: Text
  gives Text
  do
    give reason "answer {q}"

Right:
task good
  needs q: Text
  gives Text
  do
    answer = reason "answer {q}"
    when answer.sure -> give answer
    else -> give "unknown""#,
    },
    CodeInfo {
        code: "E030",
        title: "pure function uses a stochastic or side-effecting operation",
        explain: r#"A `pure` function performs an operation that is not deterministic (`reason`, `classify`, `search`, `recall`, `learn`, `spawn`, `find`, ...). Pure functions must stay deterministic.

Wrong:
pure bad
  needs x: Text
  gives Text
  do
    give reason "think about {x}"

Right:
task good
  needs x: Text
  gives Text
  do
    give reason "think about {x}""#,
    },
    CodeInfo {
        code: "E031",
        title: "pure function uses try...or",
        explain: r#"`try...or` wraps a stochastic operation, so it cannot appear in a deterministic `pure` function.

Wrong:
pure bad
  needs x: Text
  gives Text
  do
    give try helper(x) or "fallback"

Right:
task good
  needs x: Text
  gives Text
  do
    give try helper(x) or "fallback""#,
    },
    CodeInfo {
        code: "E032",
        title: "pure function escalates",
        explain: r#"`escalate` is a side effect and cannot appear in a deterministic `pure` function.

Wrong:
pure bad
  needs x: Text
  gives Text
  do
    escalate to human

Right:
task good
  needs x: Text
  gives Text
  do
    escalate to human"#,
    },
    CodeInfo {
        code: "E033",
        title: "pure function calls a task",
        explain: r#"A `pure` function may only call other `pure` functions; a `task` may be stochastic or side-effecting.

Wrong:
pure bad
  needs x: Text
  gives Text
  do
    give enrich(x)

Right:
pure good
  needs x: Text
  gives Text
  do
    give normalize(x)"#,
    },
];

/// Look up a diagnostic code. Returns `None` for unknown codes.
pub fn lookup(code: &str) -> Option<&'static CodeInfo> {
    CODES.iter().find(|c| c.code == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for info in CODES {
            assert!(seen.insert(info.code), "duplicate code {}", info.code);
        }
    }

    #[test]
    fn every_code_matches_the_contract_pattern() {
        for info in CODES {
            let bytes = info.code.as_bytes();
            assert_eq!(bytes.len(), 4, "bad code length: {}", info.code);
            assert!(
                bytes[0] == b'E' || bytes[0] == b'W',
                "bad code prefix: {}",
                info.code
            );
            assert!(
                bytes[1..].iter().all(u8::is_ascii_digit),
                "bad code digits: {}",
                info.code
            );
        }
    }

    #[test]
    fn every_explain_has_a_wrong_and_a_right_snippet() {
        for info in CODES {
            assert!(
                info.explain.contains("Wrong:"),
                "{} has no Wrong: snippet",
                info.code
            );
            assert!(
                info.explain.contains("Right:"),
                "{} has no Right: snippet",
                info.code
            );
            assert!(!info.title.is_empty(), "{} has no title", info.code);
        }
    }

    #[test]
    fn lookup_finds_every_registered_code() {
        for info in CODES {
            assert_eq!(lookup(info.code).map(|c| c.code), Some(info.code));
        }
        assert!(lookup("E999").is_none());
    }

    /// Every code passed to a `Diagnostic` constructor in `src/` must be
    /// registered above. Codes are scanned literally: `"E123"` / `"W123"`.
    #[test]
    fn constructor_codes_are_registered() {
        const SOURCES: &[&str] = &[
            include_str!("parser.rs"),
            include_str!("resolver.rs"),
            include_str!("diagnostic.rs"),
            include_str!("checker/allows_checker.rs"),
            include_str!("checker/boundary_checker.rs"),
            include_str!("checker/correlate_checker.rs"),
            include_str!("checker/pure_checker.rs"),
            include_str!("checker/requires_checker.rs"),
            include_str!("checker/schedule_checker.rs"),
            include_str!("checker/spawn_checker.rs"),
            include_str!("checker/states_checker.rs"),
            include_str!("checker/uncertain_checker.rs"),
            include_str!("checker/warden_checker.rs"),
            include_str!("checker/webhook_checker.rs"),
        ];

        for source in SOURCES {
            for code in literal_codes(source) {
                assert!(
                    lookup(code).is_some(),
                    "code {code} is used by a diagnostic but missing from CODES"
                );
            }
        }
    }

    #[test]
    fn literal_code_scan_finds_codes() {
        assert_eq!(
            literal_codes(r#"Diagnostic::error("E012", f, m, s, l)"#),
            vec!["E012"]
        );
        assert_eq!(
            literal_codes(r#"Diagnostic::warning("W070", ...)"#),
            vec!["W070"]
        );
        assert!(literal_codes(r#""not a code""#).is_empty());
    }

    /// Collect every `"E123"`/`"W123"` literal in a source file.
    fn literal_codes(source: &str) -> Vec<&str> {
        let bytes = source.as_bytes();
        let mut found = Vec::new();
        for i in 0..bytes.len().saturating_sub(5) {
            if bytes[i] == b'"'
                && (bytes[i + 1] == b'E' || bytes[i + 1] == b'W')
                && bytes[i + 2..i + 5].iter().all(u8::is_ascii_digit)
                && bytes[i + 5] == b'"'
            {
                found.push(&source[i + 1..i + 5]);
            }
        }
        found
    }
}
