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
    CodeInfo {
        code: "E040",
        title: "unknown lifecycle",
        explain: r#"An agent names a `lifecycle:` that is not a declared `states` block.

Wrong:
agent broken
  lifecycle: BogusStates
  on start
    say "hi"

Right:
states Phase
  idle -> active

agent fixed
  lifecycle: Phase
  on start
    say "hi""#,
    },
    CodeInfo {
        code: "E041",
        title: "conflicting lifecycle guards",
        explain: r#"A handler carries more than one `requires lifecycle == ...` guard, so the current state is ambiguous.

Wrong:
agent a
  on tick
    requires lifecycle == idle
    requires lifecycle == active

Right:
agent a
  on tick
    requires lifecycle == idle"#,
    },
    CodeInfo {
        code: "E042",
        title: "unknown state in lifecycle guard",
        explain: r#"A `requires lifecycle == X` guard names a state that the `states` block does not declare.

Wrong:
states Phase
  idle -> active

agent a
  on tick
    requires lifecycle == running

Right:
states Phase
  idle -> active

agent a
  on tick
    requires lifecycle == idle"#,
    },
    CodeInfo {
        code: "E043",
        title: "unknown state in transition",
        explain: r#"A `transition to X` target is not a state declared by the agent's `states` block.

Wrong:
states Phase
  idle -> active

agent broken
  lifecycle: Phase
  on start
    transition to nonexistent

Right:
states Phase
  idle -> active

agent fixed
  lifecycle: Phase
  on start
    requires lifecycle == idle
    transition to active"#,
    },
    CodeInfo {
        code: "E044",
        title: "unguarded transition",
        explain: r#"A transition has no `requires lifecycle == ...` guard, so it can fire from any state.

Wrong:
agent a
  lifecycle: Phase
  on start
    transition to active

Right:
agent a
  lifecycle: Phase
  on start
    requires lifecycle == idle
    transition to active"#,
    },
    CodeInfo {
        code: "E045",
        title: "illegal transition",
        explain: r#"The guarded transition is not an edge declared in the `states` block.

Wrong:
states GamePhase
  waiting -> playing
  playing -> done

agent broken
  lifecycle: GamePhase
  on start
    requires lifecycle == done
    transition to playing

Right:
states GamePhase
  waiting -> playing
  playing -> done

agent fixed
  lifecycle: GamePhase
  on start
    requires lifecycle == waiting
    transition to playing"#,
    },
    CodeInfo {
        code: "W040",
        title: "lifecycle guard too complex for static analysis",
        explain: r#"The lifecycle guard is not a simple state comparison, so the checker cannot verify transitions statically.

Wrong:
agent a
  on tick
    requires lifecycle == pick(state)

Right:
agent a
  on tick
    requires lifecycle == idle"#,
    },
    CodeInfo {
        code: "W041",
        title: "terminal state",
        explain: r#"A state has no outgoing transitions: once entered, the lifecycle cannot leave it.

Wrong:
states Phase
  idle -> done

Right:
states Phase
  idle -> done
  done -> idle"#,
    },
    CodeInfo {
        code: "W042",
        title: "unreachable state",
        explain: r#"A state has no incoming transition and is not an initial state, so nothing can ever enter it.

Wrong:
states Phase
  idle -> done
  orphan -> idle

Right:
states Phase
  idle -> done
  done -> idle"#,
    },
    CodeInfo {
        code: "E050",
        title: "endpoint outside the server boundary",
        explain: r#"`endpoint` declarations are only legal in files whose boundary is `server`.

Wrong:
#! boundary: shared

endpoint login(user: Text) -> Text
  give "ok"

Right:
#! boundary: server

endpoint login(user: Text) -> Text
  give "ok""#,
    },
    CodeInfo {
        code: "E051",
        title: "non-serializable field in a shared type",
        explain: r#"A `shared` type carries a field typed as an agent, pool or flow reference, which cannot cross the wire.

Wrong:
#! boundary: shared

agent Worker
  on start
    say "hi"

type Job
  worker: Worker

Right:
#! boundary: shared

type Job
  worker: Text"#,
    },
    CodeInfo {
        code: "E052",
        title: "emit in a client boundary",
        explain: r#"The event bus is a server-runtime construct, so a one-shot client has no bus to emit onto.

Wrong:
#! boundary: client

agent a
  on start
    emit Ping

Right:
#! boundary: server

agent a
  on start
    emit Ping"#,
    },
    CodeInfo {
        code: "E053",
        title: "spawn in a client boundary",
        explain: r#"`spawn` needs a supervising runtime that outlives a single request; a client is a one-shot process.

Wrong:
#! boundary: client

agent a
  on start
    spawn Worker

Right:
#! boundary: server

agent a
  on start
    spawn Worker"#,
    },
    CodeInfo {
        code: "E054",
        title: "search outside the server boundary",
        explain: r#"`search` makes network requests and can only be used in server boundary files.

Wrong:
#! boundary: client

agent a
  on start
    search "latest news"

Right:
#! boundary: server

agent a
  on start
    search "latest news""#,
    },
    CodeInfo {
        code: "E055",
        title: "web call in a shared boundary",
        explain: r#"`shared` boundary files hold types and pure code; `web.fetch`/`web.post` must live in a server or client file.

Wrong:
#! boundary: shared

fn fetch_it
  give web.fetch("https://example.com")

Right:
#! boundary: server

fn fetch_it
  give web.fetch("https://example.com")"#,
    },
    CodeInfo {
        code: "E056",
        title: "data access in a client boundary",
        explain: r#"Durable storage is a server capability; a client has no storage runtime.

Wrong:
#! boundary: client

fn main
  data.store("key", "value")

Right:
#! boundary: server

fn main
  data.store("key", "value")"#,
    },
    CodeInfo {
        code: "E057",
        title: "file I/O outside the server boundary",
        explain: r#"`file.read`/`file.write` touch the host filesystem and must live in server boundary files.

Wrong:
#! boundary: client

fn main
  say file.read("notes.txt")

Right:
#! boundary: server

fn main
  say file.read("notes.txt")"#,
    },
    CodeInfo {
        code: "E058",
        title: "client code references a server-only symbol",
        explain: r#"A symbol declared in a `server` boundary file is not visible from client code.

Wrong:
#! boundary: client

fn main
  result = server_only_task("hello")

Right:
#! boundary: server

fn main
  result = server_only_task("hello")"#,
    },
    CodeInfo {
        code: "E059",
        title: "server code references a client-only symbol",
        explain: r#"A symbol declared in a `client` boundary file is not visible from server code.

Wrong:
#! boundary: server

fn main
  say client_helper()

Right:
#! boundary: client

fn main
  say client_helper()"#,
    },
    CodeInfo {
        code: "E060",
        title: "shared code references a server-only symbol",
        explain: r#"A symbol declared in a `server` boundary file is not visible from shared code.

Wrong:
#! boundary: shared

fn helper
  say server_only_fn()

Right:
#! boundary: server

fn helper
  say server_only_fn()"#,
    },
    CodeInfo {
        code: "E061",
        title: "shared code references a client-only symbol",
        explain: r#"A symbol declared in a `client` boundary file is not visible from shared code.

Wrong:
#! boundary: shared

fn helper
  say client_only_fn()

Right:
#! boundary: client

fn helper
  say client_only_fn()"#,
    },
    CodeInfo {
        code: "W070",
        title: "requires clause uses the LLM operation `reason`",
        explain: r#"LLM operations are stochastic; preconditions should be deterministic so the guard is reproducible.

Wrong:
agent a
  on verify(msg: Text)
    requires reason "is {msg} valid?"

Right:
agent a
  on verify(msg: Text)
    requires is_valid(msg)"#,
    },
    CodeInfo {
        code: "W071",
        title: "requires clause uses the LLM operation `classify`",
        explain: r#"`classify` is stochastic; use a `pure` function for preconditions.

Wrong:
agent a
  on verify(msg: Text)
    requires classify msg

Right:
agent a
  on verify(msg: Text)
    requires is_valid(msg)"#,
    },
    CodeInfo {
        code: "W072",
        title: "requires clause uses the LLM operation `search`",
        explain: r#"`search` is stochastic and network-bound; use a `pure` function for preconditions.

Wrong:
agent a
  on verify(msg: Text)
    requires search "policy for {msg}"

Right:
agent a
  on verify(msg: Text)
    requires is_valid(msg)"#,
    },
    CodeInfo {
        code: "W073",
        title: "requires clause uses the knowledge operation `recall`",
        explain: r#"Knowledge retrieval is non-deterministic; preconditions should be deterministic.

Wrong:
agent a
  on verify(msg: Text)
    requires recall "{msg}"

Right:
agent a
  on verify(msg: Text)
    requires is_valid(msg)"#,
    },
    CodeInfo {
        code: "W074",
        title: "requires clause uses try...or",
        explain: r#"`try...or` wraps a stochastic operation, which makes the precondition non-deterministic.

Wrong:
agent a
  on verify(msg: Text)
    requires try classify(msg) or false

Right:
agent a
  on verify(msg: Text)
    requires is_valid(msg)"#,
    },
    CodeInfo {
        code: "W075",
        title: "requires clause calls a task",
        explain: r#"Tasks may be stochastic or side-effecting; preconditions should call `pure` functions only.

Wrong:
agent a
  on verify(msg: Text)
    requires helper(msg)

Right:
agent a
  on verify(msg: Text)
    requires is_valid(msg)"#,
    },
    CodeInfo {
        code: "W076",
        title: "requires clause uses `find`",
        explain: r#"`find` reads live runtime state, which is not deterministic.

Wrong:
agent a
  on verify(msg: Text)
    requires find "room_42" != none

Right:
agent a
  on verify(msg: Text)
    requires is_valid(msg)"#,
    },
    CodeInfo {
        code: "W077",
        title: "requires clause uses `exec`",
        explain: r#"`exec` runs an external process; its result is not deterministic.

Wrong:
agent a
  on verify(msg: Text)
    requires exec("date") != ""

Right:
agent a
  on verify(msg: Text)
    requires is_valid(msg)"#,
    },
    CodeInfo {
        code: "W078",
        title: "requires clause uses command/session",
        explain: r#"`command` and `session` run external processes; their results are not deterministic.

Wrong:
agent a
  on verify(msg: Text)
    requires command("ls") != ""

Right:
agent a
  on verify(msg: Text)
    requires is_valid(msg)"#,
    },
    CodeInfo {
        code: "W079",
        title: "requires clause uses `session`",
        explain: r#"A `session` delegates to an external agent, which is not deterministic.

Wrong:
agent a
  on verify(msg: Text)
    requires session("reviewer") != ""

Right:
agent a
  on verify(msg: Text)
    requires is_valid(msg)"#,
    },
    CodeInfo {
        code: "W080",
        title: "spawned agent has no failure policy",
        explain: r#"An agent with no `if stuck ...` handler cannot recover its spawned specialists; Principle VII asks for accountability when a spawn goes wrong.

Wrong:
agent worker
  on start
    say "hi"

agent boss
  on start
    spawn worker

Right:
agent worker
  on start
    say "hi"
  if stuck for 3 turns
    restart

agent boss
  on start
    spawn worker"#,
    },
    CodeInfo {
        code: "W081",
        title: "find references an unknown agent template",
        explain: r#"`find` targets an agent template that is not declared in the program, so the lookup can never match.

Wrong:
agent boss
  on start
    found = find all "ghost"
    say "done"

Right:
agent ghost
  on start
    say "hi"

agent boss
  on start
    found = find all "ghost"
    say "done""#,
    },
    CodeInfo {
        code: "E090",
        title: "warden manages an undeclared symbol",
        explain: r#"A warden's `manages` list names an agent or warden that is not declared in the file.

Wrong:
agent bot
  on start
    say "hi"

warden supervisor
  manages [ghost]

Right:
agent bot
  on start
    say "hi"

warden supervisor
  manages [bot]"#,
    },
    CodeInfo {
        code: "E091",
        title: "warden `after` count does not increase",
        explain: r#"Escalation steps must appear in increasing turn order, so each `after N:` count must be greater than the previous one.

Wrong:
warden supervisor
  manages [bot]
  on stuck: nudge, self
    after 3: restart
    after 2: escalate

Right:
warden supervisor
  manages [bot]
  on stuck: nudge, self
    after 3: restart
    after 5: escalate"#,
    },
    CodeInfo {
        code: "E092",
        title: "warden escalation ladder does not increase severity",
        explain: r#"Responses must escalate in severity: nudge < downgrade < restart < replace < escalate.

Wrong:
warden supervisor
  manages [bot]
  on stuck: restart, self
    after 3: nudge

Right:
warden supervisor
  manages [bot]
  on stuck: nudge, self
    after 3: restart"#,
    },
    CodeInfo {
        code: "W090",
        title: "warden does not cover every failure type",
        explain: r#"A warden without policies for all six failure types leaves those failures unsupervised.

Wrong:
warden supervisor
  manages [bot]
  on stuck: nudge, self
    after 3: restart

Right:
warden supervisor
  manages [bot]
  on stuck: nudge, self
    after 3: restart
  on crash: restart, self
  on hallucination: nudge, self
  on contradiction: nudge, self
  on budget: nudge, self
  on timeout: restart, self"#,
    },
    CodeInfo {
        code: "E100",
        title: "allows pattern does not start with skill.",
        explain: r#"Every allow-list entry must be a `skill.<namespace>` pattern; other namespaces are not gated by `allows`.

Wrong:
agent a
  allows github.*

Right:
agent a
  allows skill.github.*"#,
    },
    CodeInfo {
        code: "E101",
        title: "allows pattern has a malformed wildcard",
        explain: r#"Only a trailing `.*` segment is supported; a `*` anywhere else matches nothing.

Wrong:
agent a
  allows skill.git*ub

Right:
agent a
  allows skill.github.*"#,
    },
    CodeInfo {
        code: "E102",
        title: "skill call outside the agent's allow-list",
        explain: r#"The agent declares `allows` patterns and this skill call is not covered by any of them, so the call is rejected.

Wrong:
agent a
  allows skill.github.*

  on start
    issues = skill.exec.ripgrep("TODO")

Right:
agent a
  allows skill.github.*, skill.exec.ripgrep

  on start
    issues = skill.exec.ripgrep("TODO")"#,
    },
    CodeInfo {
        code: "E110",
        title: "schedule is missing a when: clause",
        explain: r#"Every schedule must declare when it fires: `when: daily at "HH:MM"`, `when: every <duration>` or `when: cron "..."`.

Wrong:
agent a
  schedule hb
    mode: wake
    emit: Heartbeat

Right:
agent a
  schedule hb
    when: every 6h
    mode: wake
    emit: Heartbeat"#,
    },
    CodeInfo {
        code: "E111",
        title: "schedule is missing a mode: clause",
        explain: r#"Every schedule must declare how it fires: `mode: spawn` (with a `prompt:`) or `mode: wake` (with an `emit:` or a `.tick` handler).

Wrong:
agent a
  schedule hb
    when: every 6h

Right:
agent a
  schedule hb
    when: every 6h
    mode: wake
    emit: Heartbeat"#,
    },
    CodeInfo {
        code: "E112",
        title: "spawn schedule is missing a prompt:",
        explain: r#"`mode: spawn` starts a stateless turn, so it must say what that turn should do.

Wrong:
agent a
  schedule hb
    when: every 6h
    mode: spawn

Right:
agent a
  schedule hb
    when: every 6h
    mode: spawn
    prompt: "check the queue""#,
    },
    CodeInfo {
        code: "E113",
        title: "wake schedule delivers nothing",
        explain: r#"`mode: wake` must deliver an event: declare `emit:` with a matching `on` handler, or add a `on <schedule>.tick` handler.

Wrong:
agent a
  schedule hb
    when: every 6h
    mode: wake

Right:
agent a
  schedule hb
    when: every 6h
    mode: wake
    emit: Heartbeat
  on Heartbeat
    say "tick""#,
    },
    CodeInfo {
        code: "E114",
        title: "duplicate schedule name",
        explain: r#"Schedule names must be unique within an agent.

Wrong:
agent a
  schedule hb
    when: every 6h
    mode: wake
    emit: Heartbeat
  schedule hb
    when: every 1d
    mode: wake
    emit: Heartbeat

Right:
agent a
  schedule hb
    when: every 6h
    mode: wake
    emit: Heartbeat
  schedule daily
    when: every 1d
    mode: wake
    emit: Heartbeat"#,
    },
    CodeInfo {
        code: "E115",
        title: "duplicate schedule option",
        explain: r#"An option appears twice in the same schedule block; keep one.

Wrong:
agent a
  schedule hb
    when: every 6h
    when: every 1d
    mode: wake
    emit: Heartbeat

Right:
agent a
  schedule hb
    when: every 6h
    mode: wake
    emit: Heartbeat"#,
    },
    CodeInfo {
        code: "E116",
        title: "invalid cron expression",
        explain: r#"FORGE cron uses standard 5-field Unix syntax: `m h dom mon dow` (for example `0 9 * * *` for 09:00 daily).

Wrong:
agent a
  schedule hb
    when: cron "every morning"
    mode: wake
    emit: Heartbeat

Right:
agent a
  schedule hb
    when: cron "0 9 * * *"
    mode: wake
    emit: Heartbeat"#,
    },
    CodeInfo {
        code: "E117",
        title: "invalid time literal",
        explain: r#"Time literals are 24-hour `"HH:MM"`: hour 0-23 and minute 0-59.

Wrong:
agent a
  schedule hb
    when: daily at "25:00"
    mode: wake
    emit: Heartbeat

Right:
agent a
  schedule hb
    when: daily at "09:00"
    mode: wake
    emit: Heartbeat"#,
    },
    CodeInfo {
        code: "E118",
        title: "zero schedule duration",
        explain: r#"An `every` interval must be positive, otherwise the schedule can never fire.

Wrong:
agent a
  schedule hb
    when: every 0s
    mode: wake
    emit: Heartbeat

Right:
agent a
  schedule hb
    when: every 30s
    mode: wake
    emit: Heartbeat"#,
    },
    CodeInfo {
        code: "E119",
        title: "schedule name collides with a timer or event name",
        explain: r#"Names must be unique across timers, schedules and handler events within an agent.

Wrong:
agent a
  timer hb: 300s
  schedule hb
    when: every 6h
    mode: wake
    emit: Heartbeat

Right:
agent a
  timer hb: 300s
  schedule heartbeat
    when: every 6h
    mode: wake
    emit: Heartbeat"#,
    },
    CodeInfo {
        code: "W110",
        title: "spawn schedule has an extraneous emit:",
        explain: r#"`emit:` is ignored under `mode: spawn`; remove it or switch to `mode: wake`.

Wrong:
agent a
  schedule hb
    when: every 6h
    mode: spawn
    prompt: "check the queue"
    emit: Heartbeat

Right:
agent a
  schedule hb
    when: every 6h
    mode: spawn
    prompt: "check the queue""#,
    },
    CodeInfo {
        code: "W111",
        title: "wake schedule has an extraneous prompt:",
        explain: r#"`prompt:` is ignored under `mode: wake`; remove it or switch to `mode: spawn`.

Wrong:
agent a
  schedule hb
    when: every 6h
    mode: wake
    emit: Heartbeat
    prompt: "check the queue"

Right:
agent a
  schedule hb
    when: every 6h
    mode: wake
    emit: Heartbeat"#,
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
