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
    ForgeParser::parse(Rule::field_access_expr, "memory.history").unwrap();
    ForgeParser::parse(Rule::field_access_expr, "gather.*").unwrap();
    ForgeParser::parse(Rule::field_access_expr, "synthesize.draft").unwrap();
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
