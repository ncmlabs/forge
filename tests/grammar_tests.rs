use pest::Parser;
use forge::parser::{ForgeParser, Rule};

// ============================================================
// Token-level tests
// ============================================================

#[test]
fn parse_ident() {
    ForgeParser::parse(Rule::ident, "hello").unwrap();
    ForgeParser::parse(Rule::ident, "my_task").unwrap();
    ForgeParser::parse(Rule::ident, "_private").unwrap();
    ForgeParser::parse(Rule::ident, "x1").unwrap();
}

#[test]
fn keywords_rejected_as_ident() {
    assert!(ForgeParser::parse(Rule::ident, "give").is_err());
    assert!(ForgeParser::parse(Rule::ident, "task").is_err());
    assert!(ForgeParser::parse(Rule::ident, "when").is_err());
    assert!(ForgeParser::parse(Rule::ident, "reason").is_err());
}

#[test]
fn keyword_prefix_allowed_as_ident() {
    // "giving" starts with "give" but is not the keyword "give"
    ForgeParser::parse(Rule::ident, "giving").unwrap();
    ForgeParser::parse(Rule::ident, "tasks").unwrap();
    ForgeParser::parse(Rule::ident, "searching").unwrap();
}

#[test]
fn parse_number_lit() {
    ForgeParser::parse(Rule::number_lit, "42").unwrap();
    ForgeParser::parse(Rule::number_lit, "0.85").unwrap();
    ForgeParser::parse(Rule::number_lit, "3").unwrap();
}

#[test]
fn parse_bool_lit() {
    ForgeParser::parse(Rule::bool_lit, "true").unwrap();
    ForgeParser::parse(Rule::bool_lit, "false").unwrap();
}

#[test]
fn parse_template_string() {
    ForgeParser::parse(Rule::template_string, r#""hello""#).unwrap();
    ForgeParser::parse(Rule::template_string, r#""hello {name}""#).unwrap();
    ForgeParser::parse(Rule::template_string, r#""a {x} b {y} c""#).unwrap();
}

#[test]
fn parse_type_name() {
    ForgeParser::parse(Rule::type_name, "Text").unwrap();
    ForgeParser::parse(Rule::type_name, "Number").unwrap();
    ForgeParser::parse(Rule::type_name, "Bool").unwrap();
    ForgeParser::parse(Rule::type_name, "Results").unwrap();
    ForgeParser::parse(Rule::type_name, "Report").unwrap();
    ForgeParser::parse(Rule::type_name, "Intent").unwrap();
    // Custom type names via ident
    ForgeParser::parse(Rule::type_name, "MyCustomType").unwrap();
}

// ============================================================
// Expression tests
// ============================================================

#[test]
fn parse_reason_expr() {
    ForgeParser::parse(Rule::reason_expr, r#"reason "summarize this""#).unwrap();
    ForgeParser::parse(Rule::reason_expr, r#"reason "what is {doc}""#).unwrap();
}

#[test]
fn parse_classify_expr() {
    ForgeParser::parse(
        Rule::classify_expr,
        r#"classify message into ["buy", "sell"]"#,
    )
    .unwrap();
}

#[test]
fn parse_search_expr() {
    ForgeParser::parse(Rule::search_expr, r#"search "rust pest parser""#).unwrap();
    ForgeParser::parse(Rule::search_expr, "search topic").unwrap();
}

#[test]
fn parse_try_or_expr() {
    ForgeParser::parse(Rule::try_or_expr, "try foo or bar").unwrap();
    ForgeParser::parse(Rule::try_or_expr, r#"try search "query" or "default""#).unwrap();
    ForgeParser::parse(Rule::try_or_expr, "try a + b or c").unwrap();
    ForgeParser::parse(Rule::try_or_expr, "try a > 0 and b or fallback").unwrap();
}

#[test]
fn parse_call_expr() {
    ForgeParser::parse(Rule::call_expr, "greet()").unwrap();
    ForgeParser::parse(Rule::call_expr, "greet(name)").unwrap();
    ForgeParser::parse(Rule::call_expr, r#"flag("low-confidence")"#).unwrap();
}

#[test]
fn parse_call_with_named_args() {
    ForgeParser::parse(
        Rule::call_expr,
        r#"route_to_handler(intent, history: history)"#,
    )
    .unwrap();
}

#[test]
fn parse_field_access() {
    // Field access is now a postfix operation
    ForgeParser::parse(Rule::postfix_expr, "memory.history").unwrap();
    ForgeParser::parse(Rule::postfix_expr, "gather.*").unwrap();
    ForgeParser::parse(Rule::postfix_expr, "synthesize.draft").unwrap();
}

#[test]
fn parse_compose_expr() {
    ForgeParser::parse(Rule::compose_expr, "A >> B >> C").unwrap();
    ForgeParser::parse(
        Rule::compose_expr,
        "classify_intent >> route_to_handler >> send_response",
    )
    .unwrap();
}

#[test]
fn parse_fan_out() {
    ForgeParser::parse(Rule::compose_expr, "(A | B | C) >> merge >> D").unwrap();
}

#[test]
fn parse_conf_predicate() {
    ForgeParser::parse(Rule::conf_predicate, "result.sure").unwrap();
    ForgeParser::parse(Rule::conf_predicate, "result.unsure").unwrap();
    ForgeParser::parse(Rule::conf_predicate, "result.unreliable").unwrap();
    ForgeParser::parse(Rule::conf_predicate, "result.conflicted").unwrap();
    ForgeParser::parse(Rule::conf_predicate, "result.sure(above: 0.85)").unwrap();
}

#[test]
fn parse_type_dot_access() {
    ForgeParser::parse(Rule::type_dot_access, "Intent.unknown").unwrap();
    ForgeParser::parse(Rule::type_dot_access, "Conversation.empty").unwrap();
}

#[test]
fn parse_constructor_expr() {
    ForgeParser::parse(Rule::constructor_expr, "Report(checked)").unwrap();
    ForgeParser::parse(
        Rule::constructor_expr,
        r#"Failure("search unavailable", retry: true)"#,
    )
    .unwrap();
}

// ============================================================
// Declaration tests
// ============================================================

#[test]
fn parse_use_decl() {
    let src = "use\n  llm.reason\n  web.search\n";
    ForgeParser::parse(Rule::use_decl, src).unwrap();
}

#[test]
fn parse_simple_task() {
    let src = "\
task greet
  needs name: Text
  gives Text
  do
    say \"Hello, {name}!\"
";
    ForgeParser::parse(Rule::task_decl, src).unwrap();
}

#[test]
fn parse_composition_task() {
    let src = "\
task process_message
  is classify_intent >> route_to_handler >> send_response
";
    ForgeParser::parse(Rule::task_decl, src).unwrap();
}

#[test]
fn parse_task_with_if_fails() {
    let src = "\
task safe_search
  needs query: Text
  gives Results or Failure
  do
    result = search query
    give result
  if fails
    give Failure(\"search unavailable\", retry: true)
";
    ForgeParser::parse(Rule::task_decl, src).unwrap();
}

#[test]
fn parse_flow() {
    let src = "\
flow research
  needs topic: Text
  gives Report

  stage gather
    web_results = search topic
    paper_results = search \"{topic} research paper\"

  stage synthesize
    needs gather.*
    draft = reason \"synthesize these sources into a report: {gather.*}\"

  stage verify
    needs synthesize.draft
    checked = reason \"fact-check this: {synthesize.draft}\"
    give Report(checked)
";
    ForgeParser::parse(Rule::flow_decl, src).unwrap();
}

#[test]
fn parse_agent() {
    let src = "\
agent support_bot
  memory
    history: Conversation
    user: Profile

  on message: Text
    intent = classify_intent(message)
    response = route_to_handler(intent, history: memory.history)
    memory.history = memory.history
    give response

  on reset
    memory.history = Conversation.empty

  if stuck
    escalate to human
";
    ForgeParser::parse(Rule::agent_decl, src).unwrap();
}

#[test]
fn parse_pool() {
    let src = "\
pool search_workers
  workers: SearchAgent * 3
  strategy: fastest
  fallback: CachedSearch
";
    ForgeParser::parse(Rule::pool_decl, src).unwrap();
}

#[test]
fn parse_pool_with_timeout() {
    let src = "\
pool fact_checkers
  workers: FactChecker * 5
  strategy: majority
  timeout: 10s
";
    ForgeParser::parse(Rule::pool_decl, src).unwrap();
}

#[test]
fn parse_contract() {
    let src = "\
contract Researcher
  can search(query: Text) -> Results
  can summarize(sources: Results) -> Summary
";
    ForgeParser::parse(Rule::contract_decl, src).unwrap();
}

#[test]
fn parse_system() {
    let src = "\
system analytics_pipeline
  use
    ingestion: DataIngestor
    analysis: Researcher
    reporting: ReportWriter

  ingestion >> analysis >> reporting
";
    ForgeParser::parse(Rule::system_decl, src).unwrap();
}

#[test]
fn parse_fn_main() {
    let src = "\
fn main
  greet(\"world\")
";
    ForgeParser::parse(Rule::fn_main_decl, src).unwrap();
}

// ============================================================
// Full file tests
// ============================================================

#[test]
fn parse_hello_forge() {
    let source = std::fs::read_to_string("examples/hello.forge").unwrap();
    ForgeParser::parse(Rule::program, &source).unwrap();
}

#[test]
fn parse_classify_forge() {
    let source = std::fs::read_to_string("examples/classify.forge").unwrap();
    ForgeParser::parse(Rule::program, &source).unwrap();
}

// ============================================================
// v3 keyword tests
// ============================================================

#[test]
fn v3_keywords_rejected_as_ident() {
    assert!(ForgeParser::parse(Rule::ident, "pure").is_err());
    assert!(ForgeParser::parse(Rule::ident, "event").is_err());
    assert!(ForgeParser::parse(Rule::ident, "states").is_err());
    assert!(ForgeParser::parse(Rule::ident, "timer").is_err());
    assert!(ForgeParser::parse(Rule::ident, "match").is_err());
    assert!(ForgeParser::parse(Rule::ident, "emit").is_err());
    assert!(ForgeParser::parse(Rule::ident, "transition").is_err());
    assert!(ForgeParser::parse(Rule::ident, "requires").is_err());
    assert!(ForgeParser::parse(Rule::ident, "forward").is_err());
    assert!(ForgeParser::parse(Rule::ident, "subscribe").is_err());
}

#[test]
fn v3_keyword_prefixes_allowed_as_ident() {
    ForgeParser::parse(Rule::ident, "purely").unwrap();
    ForgeParser::parse(Rule::ident, "events").unwrap();
    ForgeParser::parse(Rule::ident, "matching").unwrap();
    ForgeParser::parse(Rule::ident, "timers").unwrap();
}

// ============================================================
// v3 type system tests
// ============================================================

#[test]
fn parse_array_types() {
    ForgeParser::parse(Rule::type_name, "Text[9]").unwrap();
    ForgeParser::parse(Rule::type_name, "Player[]").unwrap();
    ForgeParser::parse(Rule::type_name, "Number[3]").unwrap();
    // Plain types still work
    ForgeParser::parse(Rule::type_name, "Text").unwrap();
    ForgeParser::parse(Rule::type_name, "MyType").unwrap();
}

// ============================================================
// v3 expression tests
// ============================================================

#[test]
fn parse_comparison_ops() {
    ForgeParser::parse(Rule::expr, "x == y").unwrap();
    ForgeParser::parse(Rule::expr, "x != y").unwrap();
    ForgeParser::parse(Rule::expr, "x >= 0").unwrap();
    ForgeParser::parse(Rule::expr, "x <= 8").unwrap();
    ForgeParser::parse(Rule::expr, "x > 0").unwrap();
    ForgeParser::parse(Rule::expr, "x < 10").unwrap();
}

#[test]
fn parse_boolean_ops() {
    ForgeParser::parse(Rule::expr, "x and y").unwrap();
    ForgeParser::parse(Rule::expr, "x or y").unwrap();
    ForgeParser::parse(Rule::expr, "not x").unwrap();
    ForgeParser::parse(
        Rule::expr,
        r#"cell >= 0 and cell <= 8 and board == """#,
    )
    .unwrap();
}

#[test]
fn parse_arithmetic_ops() {
    ForgeParser::parse(Rule::expr, "1 + 2").unwrap();
    ForgeParser::parse(Rule::expr, "x - 1").unwrap();
    ForgeParser::parse(Rule::expr, "a * b").unwrap();
    ForgeParser::parse(Rule::expr, "count / 2").unwrap();
    ForgeParser::parse(Rule::expr, "1 + 2 * 3").unwrap();
}

#[test]
fn parse_unary_neg() {
    ForgeParser::parse(Rule::expr, "-1").unwrap();
    ForgeParser::parse(Rule::expr, "-x").unwrap();
}

#[test]
fn parse_array_literal() {
    ForgeParser::parse(Rule::array_lit, "[1, 2, 3]").unwrap();
    ForgeParser::parse(Rule::array_lit, "[]").unwrap();
    ForgeParser::parse(Rule::array_lit, r#"["a", "b"]"#).unwrap();
}

#[test]
fn parse_nested_array_literal() {
    ForgeParser::parse(Rule::array_lit, "[[0, 1, 2], [3, 4, 5]]").unwrap();
}

#[test]
fn parse_indexing() {
    ForgeParser::parse(Rule::postfix_expr, "board[0]").unwrap();
    ForgeParser::parse(Rule::postfix_expr, "board[cell]").unwrap();
    ForgeParser::parse(Rule::postfix_expr, "line[0]").unwrap();
}

#[test]
fn parse_method_call() {
    ForgeParser::parse(Rule::postfix_expr, "list.count()").unwrap();
    ForgeParser::parse(Rule::postfix_expr, "board.none(empty)").unwrap();
}

#[test]
fn parse_chained_postfix() {
    // field access then indexing
    ForgeParser::parse(Rule::postfix_expr, "memory.board[cell]").unwrap();
    // double field access
    ForgeParser::parse(Rule::postfix_expr, "memory.players.count").unwrap();
}

// ============================================================
// v3 statement tests
// ============================================================

#[test]
fn parse_emit_stmt() {
    ForgeParser::parse(Rule::emit_stmt, "emit MoveEvent(room, player, cell)").unwrap();
    ForgeParser::parse(Rule::emit_stmt, "emit GameEndEvent()").unwrap();
}

#[test]
fn parse_transition_stmt() {
    ForgeParser::parse(Rule::transition_stmt, "transition to playing").unwrap();
    ForgeParser::parse(Rule::transition_stmt, "transition to done").unwrap();
}

#[test]
fn parse_timer_stmts() {
    ForgeParser::parse(Rule::start_timer_stmt, "start reconnect_window for player").unwrap();
    ForgeParser::parse(Rule::start_timer_stmt, "start turn_limit").unwrap();
    ForgeParser::parse(Rule::cancel_timer_stmt, "cancel reconnect_window for player").unwrap();
    ForgeParser::parse(Rule::reset_timer_stmt, "reset turn_limit").unwrap();
}

#[test]
fn parse_forward_stmt() {
    ForgeParser::parse(Rule::forward_stmt, "forward msg to target").unwrap();
}

#[test]
fn parse_memory_update_with_index() {
    ForgeParser::parse(Rule::memory_update_stmt, "memory.board[cell] = x").unwrap();
    // Without index still works
    ForgeParser::parse(Rule::memory_update_stmt, "memory.history = val").unwrap();
}

#[test]
fn parse_requires_clause() {
    ForgeParser::parse(Rule::requires_clause, "requires lifecycle == playing").unwrap();
    ForgeParser::parse(
        Rule::requires_clause,
        "requires lifecycle == waiting on fail: silent",
    )
    .unwrap();
    ForgeParser::parse(
        Rule::requires_clause,
        r#"requires valid_move(board, cell) on fail: give "invalid""#,
    )
    .unwrap();
    ForgeParser::parse(
        Rule::requires_clause,
        "requires x > 0 on fail: log",
    )
    .unwrap();
}

#[test]
fn parse_fail_policies() {
    ForgeParser::parse(Rule::fail_policy, "silent").unwrap();
    ForgeParser::parse(Rule::fail_policy, "log").unwrap();
    ForgeParser::parse(Rule::fail_policy, "escalate").unwrap();
    ForgeParser::parse(Rule::fail_policy, "crash").unwrap();
    ForgeParser::parse(Rule::fail_policy, r#"give "error""#).unwrap();
}

// ============================================================
// v3 match statement tests
// ============================================================

#[test]
fn parse_pattern() {
    ForgeParser::parse(Rule::wildcard_pattern, "_").unwrap();
    ForgeParser::parse(Rule::binding_pattern, "sym").unwrap();
    ForgeParser::parse(Rule::constructor_pattern, "Winner(sym)").unwrap();
    ForgeParser::parse(Rule::constructor_pattern, "Buy(item, qty)").unwrap();
    // Bare uppercase constructors (no parens)
    ForgeParser::parse(Rule::constructor_pattern, "Draw").unwrap();
    ForgeParser::parse(Rule::constructor_pattern, "Ongoing").unwrap();
    // Empty arg constructors
    ForgeParser::parse(Rule::constructor_pattern, "Nothing()").unwrap();
}

#[test]
fn parse_match_stmt() {
    let src = "\
match outcome
      Winner(sym) -> give sym
      Draw -> give \"draw\"
      _ -> give \"ongoing\"
";
    ForgeParser::parse(Rule::match_stmt, src).unwrap();
}

// ============================================================
// v3 if/else and for tests
// ============================================================

#[test]
fn parse_if_else_stmt() {
    let src = "\
if x > 0
      give x
";
    ForgeParser::parse(Rule::if_else_stmt, src).unwrap();
}

#[test]
fn parse_if_else_with_else() {
    let src = "\
if x > 0
      give x
    else
      give 0
";
    ForgeParser::parse(Rule::if_else_stmt, src).unwrap();
}

#[test]
fn parse_if_else_if() {
    let src = "\
if x > 10
      give \"big\"
    else if x > 0
      give \"small\"
    else
      give \"zero\"
";
    ForgeParser::parse(Rule::if_else_stmt, src).unwrap();
}

#[test]
fn parse_for_loop() {
    let src = "\
for item in list
      say item
";
    ForgeParser::parse(Rule::for_loop, src).unwrap();
}

// ============================================================
// v3 declaration tests
// ============================================================

#[test]
fn parse_pure_decl() {
    let src = "\
pure valid_move
  needs board: Text[9], cell: Number
  gives Bool
  do
    give cell >= 0 and cell <= 8
";
    ForgeParser::parse(Rule::pure_decl, src).unwrap();
}

#[test]
fn parse_event_decl() {
    let src = "\
event MoveEvent
  room_id: Text
  player: Text
  cell: Number
";
    ForgeParser::parse(Rule::event_decl, src).unwrap();
}

#[test]
fn parse_states_decl() {
    let src = "\
states RoomLifecycle
  waiting -> playing when players_full
  playing -> done when winner_found
";
    ForgeParser::parse(Rule::states_decl, src).unwrap();
}

#[test]
fn parse_type_decl() {
    let src = "\
type MoveRequest
  room_id: Text
  cell: Number
  token: Text
";
    ForgeParser::parse(Rule::type_decl, src).unwrap();
}

#[test]
fn parse_endpoint_decl() {
    let src = "\
endpoint move(req: MoveRequest) -> GameState or MoveError
  give process(req)
";
    ForgeParser::parse(Rule::endpoint_decl, src).unwrap();
}

#[test]
fn parse_boundary_directive() {
    ForgeParser::parse(Rule::boundary_directive, "#! boundary: server").unwrap();
    ForgeParser::parse(Rule::boundary_directive, "#!boundary:client").unwrap();
    ForgeParser::parse(Rule::boundary_directive, "#! boundary: shared").unwrap();
}

#[test]
fn parse_program_with_boundary() {
    let src = "\
#! boundary: server
task greet
  needs name: Text
  gives Text
  do
    say \"Hello, {name}!\"
";
    ForgeParser::parse(Rule::program, src).unwrap();
}

// ============================================================
// v3 agent extension tests
// ============================================================

#[test]
fn parse_agent_with_lifecycle() {
    let src = "\
agent room
  lifecycle: RoomLifecycle

  memory
    board: Text[9]
    turn: Number

  timer reconnect_window: 30s

  subscribe MoveEvent

  on join: Text
    say \"joined\"

  if stuck
    escalate to human
";
    ForgeParser::parse(Rule::agent_decl, src).unwrap();
}

#[test]
fn parse_on_handler_with_params() {
    let src = "  on move(player: Text, cell: Number)\n    say player\n";
    ForgeParser::parse(Rule::on_handler, src).unwrap();
}

#[test]
fn parse_on_handler_dotted_name() {
    let src = "  on reconnect_window.expired(player: Text)\n    say player\n";
    ForgeParser::parse(Rule::on_handler, src).unwrap();
}

#[test]
fn parse_on_handler_with_requires() {
    let src = "  on move(player: Text, cell: Number)\n    requires lifecycle == playing on fail: silent\n    requires cell >= 0 on fail: log\n    say player\n";
    ForgeParser::parse(Rule::on_handler, src).unwrap();
}

#[test]
fn parse_timer_field() {
    ForgeParser::parse(Rule::timer_field, "timer reconnect_window: 30s").unwrap();
    ForgeParser::parse(Rule::timer_field, "timer turn_limit: 15s").unwrap();
    ForgeParser::parse(Rule::timer_field, "timer idle_check: 2min").unwrap();
}

#[test]
fn parse_subscribe_line() {
    ForgeParser::parse(Rule::subscribe_line, "subscribe MoveEvent").unwrap();
    ForgeParser::parse(
        Rule::subscribe_line,
        "subscribe GameEndEvent where room == target",
    )
    .unwrap();
}

#[test]
fn parse_handler_event_name() {
    ForgeParser::parse(Rule::handler_event_name, "message").unwrap();
    ForgeParser::parse(Rule::handler_event_name, "reconnect_window.expired").unwrap();
    ForgeParser::parse(Rule::handler_event_name, "turn_limit.expired").unwrap();
}

#[test]
fn parse_lifecycle_clause() {
    ForgeParser::parse(Rule::lifecycle_clause, "lifecycle: RoomLifecycle").unwrap();
}

// ============================================================
// v3 nested statement tests (i3/i4 levels)
// ============================================================

#[test]
fn parse_if_inside_do_block() {
    let src = "\
do
    if x > 0
      give x
";
    ForgeParser::parse(Rule::do_block, src).unwrap();
}

#[test]
fn parse_for_inside_do_block() {
    let src = "\
do
    for item in list
      say item
";
    ForgeParser::parse(Rule::do_block, src).unwrap();
}

#[test]
fn parse_match_inside_do_block() {
    let src = "\
do
    match result
      Winner(sym) -> give sym
      _ -> give \"ongoing\"
";
    ForgeParser::parse(Rule::do_block, src).unwrap();
}

#[test]
fn parse_nested_if_in_for() {
    // for at i2 (inside do), body at i3, if at i3, if body at i4
    let src = "\
do
    for line in lines
      if line != \"\"
        say line
";
    ForgeParser::parse(Rule::do_block, src).unwrap();
}
