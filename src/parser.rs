// FORGE parser: pest Pairs → AST transformation
// One build_X function per grammar rule, recursive descent over pest output.

use pest::Parser;
use pest_derive::Parser;

use crate::ast::*;

#[derive(Parser)]
#[grammar = "grammar/forge.pest"]
pub struct ForgeParser;

type Pair<'a> = pest::iterators::Pair<'a, Rule>;
type Pairs<'a> = pest::iterators::Pairs<'a, Rule>;

// ── Helpers ──────────────────────────────────────────────────

fn to_span(pair: &Pair) -> Span {
    let s = pair.as_span();
    Span {
        start: s.start(),
        end: s.end(),
    }
}

fn spanned<T>(node: T, pair: &Pair) -> Spanned<T> {
    Spanned::new(node, to_span(pair))
}

fn parse_error(pair: &Pair, msg: &str) -> anyhow::Error {
    let (line, col) = pair.as_span().start_pos().line_col();
    anyhow::anyhow!("{}:{}: {}", line, col, msg)
}

// ── Leaf parsers ─────────────────────────────────────────────

fn build_number_lit(pair: Pair) -> anyhow::Result<f64> {
    pair.as_str()
        .parse::<f64>()
        .map_err(|e| parse_error(&pair, &format!("invalid number: {}", e)))
}

fn build_bool_lit(pair: Pair) -> bool {
    pair.as_str() == "true"
}

fn decode_template_escape(pair: &Pair) -> anyhow::Result<char> {
    match pair.as_str() {
        "\\n" => Ok('\n'),
        "\\r" => Ok('\r'),
        "\\t" => Ok('\t'),
        "\\\"" => Ok('"'),
        "\\\\" => Ok('\\'),
        other => Err(parse_error(pair, &format!("unsupported escape sequence: {}", other))),
    }
}

fn push_text_part(parts: &mut Vec<Spanned<TemplatePart>>, text: String, span: Span) {
    if text.is_empty() {
        return;
    }

    if let Some(last) = parts.last_mut() {
        if let TemplatePart::Text(existing) = &mut last.node {
            existing.push_str(&text);
            last.span.end = span.end;
            return;
        }
    }

    parts.push(Spanned::new(TemplatePart::Text(text), span));
}

fn build_template_string(pair: Pair) -> anyhow::Result<Vec<Spanned<TemplatePart>>> {
    let mut parts = Vec::new();
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::template_part => {
                let inner = child.into_inner().next().unwrap();
                match inner.as_rule() {
                    Rule::template_text => {
                        push_text_part(&mut parts, inner.as_str().to_string(), to_span(&inner));
                    }
                    Rule::template_escape => {
                        let decoded = decode_template_escape(&inner)?;
                        push_text_part(&mut parts, decoded.to_string(), to_span(&inner));
                    }
                    Rule::template_interp => {
                        let expr_pair = inner.into_inner().next().unwrap();
                        let expr = build_expr(expr_pair)?;
                        parts.push(Spanned::new(
                            TemplatePart::Interp(Box::new(expr.clone())),
                            expr.span,
                        ));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(parts)
}

fn build_plain_template_string(pair: Pair) -> anyhow::Result<String> {
    let parts = build_template_string(pair)?;
    let mut text = String::new();
    for part in parts {
        match part.node {
            TemplatePart::Text(part_text) => text.push_str(&part_text),
            TemplatePart::Interp(_) => {
                return Err(anyhow::anyhow!(
                    "classify labels must be plain strings without interpolation"
                ));
            }
        }
    }
    Ok(text)
}

fn build_type_name(pair: Pair) -> anyhow::Result<Spanned<TypeName>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().unwrap();

    let base = match first.as_rule() {
        Rule::builtin_type => match first.as_str() {
            "Text" => TypeName::Text,
            "Number" => TypeName::Number,
            "Bool" => TypeName::Bool,
            "Results" => TypeName::Results,
            "Report" => TypeName::Report,
            "Intent" => TypeName::Intent,
            "Summary" => TypeName::Summary,
            "Failure" => TypeName::Failure,
            "Classification" => TypeName::Classification,
            "Conversation" => TypeName::Conversation,
            "Profile" => TypeName::Profile,
            "SearchResults" => TypeName::SearchResults,
            other => TypeName::Custom(other.to_string()),
        },
        Rule::ident => TypeName::Custom(first.as_str().to_string()),
        _ => return Err(parse_error(&first, "expected type name")),
    };

    // Check for array suffix
    if let Some(suffix) = inner.next() {
        if suffix.as_rule() == Rule::array_suffix {
            let size = suffix
                .into_inner()
                .next()
                .map(|n| n.as_str().parse::<usize>())
                .transpose()
                .map_err(|e| anyhow::anyhow!("invalid array size: {}", e))?;
            return Ok(Spanned::new(TypeName::Array(Box::new(base), size), span));
        }
    }

    Ok(Spanned::new(base, span))
}

fn build_param(pair: Pair) -> anyhow::Result<Spanned<Param>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let type_name = build_type_name(inner.next().unwrap())?;
    Ok(Spanned::new(Param { name, type_name }, span))
}

fn build_output_type(pair: Pair) -> anyhow::Result<Spanned<OutputType>> {
    let span = to_span(&pair);
    let mut types = Vec::new();
    for child in pair.into_inner() {
        if child.as_rule() == Rule::type_name {
            types.push(build_type_name(child)?);
        }
    }
    Ok(Spanned::new(OutputType { types }, span))
}

fn build_duration(pair: Pair) -> anyhow::Result<Spanned<Duration>> {
    let span = to_span(&pair);
    let s = pair.as_str();

    let (value, unit) = if s.ends_with("min") {
        (s[..s.len() - 3].parse::<u64>()?, DurationUnit::Minutes)
    } else if s.ends_with('h') {
        (s[..s.len() - 1].parse::<u64>()?, DurationUnit::Hours)
    } else if s.ends_with('m') {
        (s[..s.len() - 1].parse::<u64>()?, DurationUnit::Minutes)
    } else if s.ends_with('s') {
        (s[..s.len() - 1].parse::<u64>()?, DurationUnit::Seconds)
    } else {
        return Err(anyhow::anyhow!("invalid duration: {}", s));
    };

    Ok(Spanned::new(Duration { value, unit }, span))
}

fn build_field_def(ident_pair: Pair, type_pair: Pair) -> anyhow::Result<Spanned<FieldDef>> {
    let start = ident_pair.as_span().start();
    let end = type_pair.as_span().end();
    let name = ident_pair.as_str().to_string();
    let type_name = build_type_name(type_pair)?;
    Ok(Spanned::new(
        FieldDef { name, type_name },
        Span { start, end },
    ))
}

// ── Expressions ──────────────────────────────────────────────

fn build_atom(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::template_string => {
            let parts = build_template_string(inner.clone())?;
            Ok(Spanned::new(Expr::Template(parts), span))
        }
        Rule::number_lit => {
            let n = build_number_lit(inner)?;
            Ok(Spanned::new(Expr::NumberLit(n), span))
        }
        Rule::bool_lit => {
            let b = build_bool_lit(inner);
            Ok(Spanned::new(Expr::BoolLit(b), span))
        }
        Rule::array_lit => {
            let mut elems = Vec::new();
            for child in inner.into_inner() {
                if child.as_rule() == Rule::expr {
                    elems.push(build_expr(child)?);
                }
            }
            Ok(Spanned::new(Expr::ArrayLit(elems), span))
        }
        Rule::type_dot_access => {
            let mut children = inner.into_inner();
            let type_pair = children.next().unwrap();
            let field_pair = children.next().unwrap();
            let type_name = match type_pair.as_str() {
                "Text" => TypeName::Text,
                "Number" => TypeName::Number,
                "Bool" => TypeName::Bool,
                "Results" => TypeName::Results,
                "Report" => TypeName::Report,
                "Intent" => TypeName::Intent,
                "Summary" => TypeName::Summary,
                "Failure" => TypeName::Failure,
                "Classification" => TypeName::Classification,
                "Conversation" => TypeName::Conversation,
                "Profile" => TypeName::Profile,
                "SearchResults" => TypeName::SearchResults,
                other => TypeName::Custom(other.to_string()),
            };
            Ok(Spanned::new(
                Expr::TypeAccess(
                    spanned(type_name, &type_pair),
                    spanned(field_pair.as_str().to_string(), &field_pair),
                ),
                span,
            ))
        }
        Rule::ident => Ok(Spanned::new(
            Expr::Ident(inner.as_str().to_string()),
            span,
        )),
        Rule::expr => {
            // Parenthesized expression
            let inner_expr = build_expr(inner)?;
            Ok(Spanned::new(Expr::Paren(Box::new(inner_expr)), span))
        }
        _ => Err(parse_error(&inner, &format!("unexpected atom rule: {:?}", inner.as_rule()))),
    }
}

fn build_call_arg(pair: Pair) -> anyhow::Result<Spanned<CallArg>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();

    // Check if first child is a label (ident followed by expr) or just an expr
    let first = inner.next().unwrap();
    if first.as_rule() == Rule::ident {
        if let Some(second) = inner.next() {
            // labeled argument: ident : expr
            let value = build_expr(second)?;
            return Ok(Spanned::new(
                CallArg {
                    label: Some(spanned(first.as_str().to_string(), &first)),
                    value,
                },
                span,
            ));
        }
    }

    // Unlabeled — first is the expr
    let value = build_expr(first)?;
    Ok(Spanned::new(
        CallArg {
            label: None,
            value,
        },
        span,
    ))
}

fn build_arg_list(pair: Pair) -> anyhow::Result<Vec<Spanned<CallArg>>> {
    let mut args = Vec::new();
    for child in pair.into_inner() {
        if child.as_rule() == Rule::call_arg {
            args.push(build_call_arg(child)?);
        }
    }
    Ok(args)
}

fn build_call_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);
    let args = if let Some(arg_list) = inner.next() {
        build_arg_list(arg_list)?
    } else {
        Vec::new()
    };
    Ok(Spanned::new(Expr::Call(CallExpr { name, args }), span))
}

fn build_constructor_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let type_pair = inner.next().unwrap();
    let type_name = match type_pair.as_str() {
        "Text" => TypeName::Text,
        "Number" => TypeName::Number,
        "Bool" => TypeName::Bool,
        "Results" => TypeName::Results,
        "Report" => TypeName::Report,
        "Intent" => TypeName::Intent,
        "Summary" => TypeName::Summary,
        "Failure" => TypeName::Failure,
        "Classification" => TypeName::Classification,
        "Conversation" => TypeName::Conversation,
        "Profile" => TypeName::Profile,
        "SearchResults" => TypeName::SearchResults,
        other => TypeName::Custom(other.to_string()),
    };
    let args = if let Some(arg_list) = inner.next() {
        build_arg_list(arg_list)?
    } else {
        Vec::new()
    };
    Ok(Spanned::new(
        Expr::Constructor(ConstructorExpr {
            type_name: spanned(type_name, &type_pair),
            args,
        }),
        span,
    ))
}

fn build_string_arg(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::template_string => {
            let span = to_span(&inner);
            let parts = build_template_string(inner)?;
            Ok(Spanned::new(Expr::Template(parts), span))
        }
        Rule::ident => Ok(spanned(Expr::Ident(inner.as_str().to_string()), &inner)),
        _ => Err(parse_error(&inner, "expected string or ident")),
    }
}

fn build_reason_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let inner = pair.into_inner().next().unwrap();
    let arg = build_string_arg(inner)?;
    Ok(Spanned::new(Expr::Reason(Box::new(arg)), span))
}

fn build_search_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let inner = pair.into_inner().next().unwrap();
    let arg = build_string_arg(inner)?;
    Ok(Spanned::new(Expr::Search(Box::new(arg)), span))
}

fn build_classify_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let input_pair = inner.next().unwrap();
    let input = build_atom(input_pair)?;
    let label_list = inner.next().unwrap();
    let mut labels = Vec::new();
    for child in label_list.into_inner() {
        if child.as_rule() == Rule::template_string {
            let text = build_plain_template_string(child.clone())?;
            labels.push(spanned(text.to_string(), &child));
        }
    }
    Ok(Spanned::new(
        Expr::Classify(ClassifyExpr {
            input: Box::new(input),
            labels,
        }),
        span,
    ))
}

fn build_try_or_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let try_expr = build_and_expr(inner.next().unwrap())?;
    let or_expr = build_expr(inner.next().unwrap())?;
    Ok(Spanned::new(
        Expr::TryOr(Box::new(try_expr), Box::new(or_expr)),
        span,
    ))
}

fn build_pipe_term(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::reason_expr => build_reason_expr(inner),
        Rule::classify_expr => build_classify_expr(inner),
        Rule::search_expr => build_search_expr(inner),
        Rule::try_or_expr => build_try_or_expr(inner),
        Rule::constructor_expr => build_constructor_expr(inner),
        Rule::call_expr => build_call_expr(inner),
        Rule::atom => build_atom(inner),
        _ => Err(parse_error(&inner, &format!("unexpected pipe_term rule: {:?}", inner.as_rule()))),
    }
}

fn build_postfix_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let base_pair = inner.next().unwrap();
    let mut result = build_pipe_term(base_pair)?;

    for op_pair in inner {
        if op_pair.as_rule() != Rule::postfix_op {
            continue;
        }
        let op_str = op_pair.as_str();
        let op_span = Span {
            start: result.span.start,
            end: op_pair.as_span().end(),
        };
        let mut op_inner = op_pair.into_inner();

        if op_str.starts_with('[') {
            // Index: [expr]
            let idx_expr = build_expr(op_inner.next().unwrap())?;
            result = Spanned::new(
                Expr::Index(Box::new(result), Box::new(idx_expr)),
                op_span,
            );
        } else if op_str.starts_with('.') {
            if let Some(first_child) = op_inner.next() {
                if first_child.as_rule() == Rule::ident {
                    // Check if method call (has parens) or field access.
                    // Zero-arg method calls like .len() have no arg_list child,
                    // so we also check whether the raw text contains '('.
                    if let Some(arg_list) = op_inner.next() {
                        let args = build_arg_list(arg_list)?;
                        result = Spanned::new(
                            Expr::MethodCall(
                                Box::new(result),
                                spanned(first_child.as_str().to_string(), &first_child),
                                args,
                            ),
                            op_span,
                        );
                    } else if op_str.contains('(') {
                        // Zero-arg method call: .len(), .count()
                        result = Spanned::new(
                            Expr::MethodCall(
                                Box::new(result),
                                spanned(first_child.as_str().to_string(), &first_child),
                                vec![],
                            ),
                            op_span,
                        );
                    } else {
                        result = Spanned::new(
                            Expr::FieldAccess(
                                Box::new(result),
                                spanned(first_child.as_str().to_string(), &first_child),
                            ),
                            op_span,
                        );
                    }
                } else {
                    // Unexpected child — treat as glob
                    result = Spanned::new(Expr::GlobAccess(Box::new(result)), op_span);
                }
            } else {
                // No children — must be glob: .*
                result = Spanned::new(Expr::GlobAccess(Box::new(result)), op_span);
            }
        }
    }

    // Preserve original span if no postfix ops
    if result.span.start == span.start && result.span.end != span.end {
        result.span = span;
    }

    Ok(result)
}

fn build_fan_out_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let children: Vec<Pair> = pair.into_inner().collect();
    if children.len() == 1 {
        return build_postfix_expr(children.into_iter().next().unwrap());
    }
    let mut exprs = Vec::new();
    for child in children {
        if child.as_rule() == Rule::postfix_expr {
            exprs.push(build_postfix_expr(child)?);
        }
    }
    Ok(Spanned::new(Expr::FanOut(exprs), span))
}

fn build_unary_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().unwrap();

    if first.as_rule() == Rule::unary_op {
        let op = match first.as_str().trim() {
            "not" => UnaryOp::Not,
            "-" => UnaryOp::Neg,
            _ => return Err(parse_error(&first, "unexpected unary op")),
        };
        let operand = build_fan_out_expr(inner.next().unwrap())?;
        Ok(Spanned::new(
            Expr::UnaryOp(spanned(op, &first), Box::new(operand)),
            span,
        ))
    } else {
        build_fan_out_expr(first)
    }
}

fn build_multiplicative_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let mut left = build_unary_expr(inner.next().unwrap())?;

    while let Some(op_pair) = inner.next() {
        let op = match op_pair.as_str() {
            "*" => BinOp::Mul,
            "/" => BinOp::Div,
            _ => return Err(parse_error(&op_pair, "expected * or /")),
        };
        let right = build_unary_expr(inner.next().unwrap())?;
        left = Spanned::new(
            Expr::BinOp(
                Box::new(left),
                spanned(op, &op_pair),
                Box::new(right),
            ),
            span,
        );
    }
    Ok(left)
}

fn build_additive_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let mut left = build_multiplicative_expr(inner.next().unwrap())?;

    while let Some(op_pair) = inner.next() {
        let op = match op_pair.as_str() {
            "+" => BinOp::Add,
            "-" => BinOp::Sub,
            _ => return Err(parse_error(&op_pair, "expected + or -")),
        };
        let right = build_multiplicative_expr(inner.next().unwrap())?;
        left = Spanned::new(
            Expr::BinOp(
                Box::new(left),
                spanned(op, &op_pair),
                Box::new(right),
            ),
            span,
        );
    }
    Ok(left)
}

fn build_comparison_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let left = build_additive_expr(inner.next().unwrap())?;

    if let Some(op_pair) = inner.next() {
        let op = match op_pair.as_str() {
            "==" => BinOp::Eq,
            "!=" => BinOp::Ne,
            ">=" => BinOp::Ge,
            "<=" => BinOp::Le,
            ">" => BinOp::Gt,
            "<" => BinOp::Lt,
            _ => return Err(parse_error(&op_pair, "expected comparison op")),
        };
        let right = build_additive_expr(inner.next().unwrap())?;
        Ok(Spanned::new(
            Expr::BinOp(
                Box::new(left),
                spanned(op, &op_pair),
                Box::new(right),
            ),
            span,
        ))
    } else {
        Ok(left)
    }
}

fn build_and_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let mut left = build_comparison_expr(inner.next().unwrap())?;

    while let Some(right_pair) = inner.next() {
        let right = build_comparison_expr(right_pair)?;
        left = Spanned::new(
            Expr::BinOp(
                Box::new(left),
                Spanned::new(BinOp::And, span),
                Box::new(right),
            ),
            span,
        );
    }
    Ok(left)
}

fn build_or_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let mut left = build_and_expr(inner.next().unwrap())?;

    while let Some(right_pair) = inner.next() {
        let right = build_and_expr(right_pair)?;
        left = Spanned::new(
            Expr::BinOp(
                Box::new(left),
                Spanned::new(BinOp::Or, span),
                Box::new(right),
            ),
            span,
        );
    }
    Ok(left)
}

fn build_compose_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    let span = to_span(&pair);
    let children: Vec<Pair> = pair.into_inner().collect();
    if children.len() == 1 {
        return build_or_expr(children.into_iter().next().unwrap());
    }
    let mut exprs = Vec::new();
    for child in children {
        exprs.push(build_or_expr(child)?);
    }
    Ok(Spanned::new(Expr::Compose(exprs), span))
}

fn build_expr(pair: Pair) -> anyhow::Result<Spanned<Expr>> {
    // expr wraps compose_expr
    let inner = pair.into_inner().next().unwrap();
    build_compose_expr(inner)
}

// ── Confidence predicates ────────────────────────────────────

fn build_conf_predicate(pair: Pair) -> anyhow::Result<Spanned<ConfidencePred>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let subject_pair = inner.next().unwrap();
    let subject = spanned(subject_pair.as_str().to_string(), &subject_pair);

    let second = inner.next().unwrap();
    let level = if second.as_rule() == Rule::conf_level {
        match second.as_str() {
            "sure" => ConfLevel::Sure(None),
            "unsure" => ConfLevel::Unsure,
            "unreliable" => ConfLevel::Unreliable,
            "conflicted" => ConfLevel::Conflicted,
            _ => return Err(parse_error(&second, "unexpected confidence level")),
        }
    } else {
        // sure(above: N) — second is the number_lit
        let threshold = build_number_lit(second)?;
        ConfLevel::Sure(Some(threshold))
    };

    Ok(Spanned::new(
        ConfidencePred {
            subject,
            level: Spanned::new(level, span),
        },
        span,
    ))
}

// ── Statements ───────────────────────────────────────────────

fn build_bind_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);
    let expr = build_expr(inner.next().unwrap())?;
    Ok(Spanned::new(Stmt::Bind(name, expr), span))
}

fn build_give_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let expr = build_expr(inner.next().unwrap())?;
    let with_expr = if let Some(call_pair) = inner.next() {
        Some(build_call_expr(call_pair)?)
    } else {
        None
    };
    Ok(Spanned::new(Stmt::Give(expr, with_expr), span))
}

fn build_say_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let expr = build_expr(pair.into_inner().next().unwrap())?;
    Ok(Spanned::new(Stmt::Say(expr), span))
}

fn build_escalate_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let target_pair = pair.into_inner().next().unwrap();
    Ok(Spanned::new(
        Stmt::Escalate(spanned(target_pair.as_str().to_string(), &target_pair)),
        span,
    ))
}

fn build_memory_update_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let field_pair = inner.next().unwrap();
    let field = spanned(field_pair.as_str().to_string(), &field_pair);

    // Collect remaining children to disambiguate index vs value
    let remaining: Vec<Pair> = inner.collect();
    let (index, value) = if remaining.len() == 2 {
        // memory.field[idx] = value
        let idx = build_expr(remaining[0].clone())?;
        let val = build_expr(remaining[1].clone())?;
        (Some(idx), val)
    } else {
        // memory.field = value
        let val = build_expr(remaining[0].clone())?;
        (None, val)
    };

    Ok(Spanned::new(Stmt::MemoryUpdate(field, index, value), span))
}

fn build_expr_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let expr = build_expr(pair.into_inner().next().unwrap())?;
    Ok(Spanned::new(Stmt::ExprStmt(expr), span))
}

fn build_emit_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);
    let args = if let Some(arg_list) = inner.next() {
        build_arg_list(arg_list)?
    } else {
        Vec::new()
    };
    Ok(Spanned::new(Stmt::Emit(name, args), span))
}

fn build_transition_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let target_pair = pair.into_inner().next().unwrap();
    Ok(Spanned::new(
        Stmt::TransitionTo(spanned(target_pair.as_str().to_string(), &target_pair)),
        span,
    ))
}

fn build_start_timer_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);
    let context = inner.next().map(|p| build_expr(p)).transpose()?;
    Ok(Spanned::new(Stmt::StartTimer { name, context }, span))
}

fn build_cancel_timer_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);
    let context = inner.next().map(|p| build_expr(p)).transpose()?;
    Ok(Spanned::new(Stmt::CancelTimer { name, context }, span))
}

fn build_reset_timer_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let name_pair = pair.into_inner().next().unwrap();
    Ok(Spanned::new(
        Stmt::ResetTimer(spanned(name_pair.as_str().to_string(), &name_pair)),
        span,
    ))
}

fn build_forward_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let what = build_expr(inner.next().unwrap())?;
    let to = build_expr(inner.next().unwrap())?;
    Ok(Spanned::new(Stmt::Forward(what, to), span))
}

// ── Control flow statements ──────────────────────────────────

fn build_when_clause(pair: Pair) -> anyhow::Result<Spanned<WhenClause>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let pred = build_conf_predicate(inner.next().unwrap())?;
    let body_pair = inner.next().unwrap();
    let body = build_statement(body_pair)?;
    Ok(Spanned::new(
        WhenClause {
            predicate: pred,
            body,
        },
        span,
    ))
}

fn build_else_clause(pair: Pair) -> anyhow::Result<Spanned<ElseClause>> {
    let span = to_span(&pair);
    let body_pair = pair.into_inner().next().unwrap();
    let body = build_statement(body_pair)?;
    Ok(Spanned::new(ElseClause { body }, span))
}

fn build_when_block(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let mut clauses = Vec::new();
    let mut else_body = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::when_clause | Rule::when_clause_i3 => {
                clauses.push(build_when_clause(child)?);
            }
            Rule::else_clause => {
                else_body = Some(build_else_clause(child)?);
            }
            _ => {}
        }
    }

    Ok(Spanned::new(
        Stmt::When(Box::new(WhenBlock { clauses, else_body })),
        span,
    ))
}

fn build_pattern(pair: Pair) -> anyhow::Result<Spanned<Pattern>> {
    let span = to_span(&pair);
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::wildcard_pattern => Ok(Spanned::new(Pattern::Wildcard, span)),
        Rule::binding_pattern => Ok(Spanned::new(
            Pattern::Binding(inner.as_str().to_string()),
            span,
        )),
        Rule::constructor_pattern => {
            let s = inner.as_str();
            let mut children = inner.into_inner();
            // Constructor name is not a separate rule — it's part of the atomic pattern
            // Parse from string: name is uppercase start, then optional (patterns)
            if let Some(first_child) = children.next() {
                // Has sub-patterns
                let name_end = s.find('(').unwrap_or(s.len());
                let name = s[..name_end].to_string();
                let mut patterns = vec![build_pattern(first_child)?];
                for child in children {
                    if child.as_rule() == Rule::pattern {
                        patterns.push(build_pattern(child)?);
                    }
                }
                Ok(Spanned::new(Pattern::Constructor(name, patterns), span))
            } else {
                // No sub-patterns
                Ok(Spanned::new(
                    Pattern::Constructor(s.to_string(), Vec::new()),
                    span,
                ))
            }
        }
        _ => Err(parse_error(&inner, "unexpected pattern rule")),
    }
}

fn build_match_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let subject = build_expr(inner.next().unwrap())?;
    let mut arms = Vec::new();

    for child in inner {
        if child.as_rule() == Rule::match_arm {
            let arm_span = to_span(&child);
            let mut arm_inner = child.into_inner();
            let pattern = build_pattern(arm_inner.next().unwrap())?;
            let body = build_statement(arm_inner.next().unwrap())?;
            arms.push(Spanned::new(MatchArm { pattern, body }, arm_span));
        }
    }

    Ok(Spanned::new(
        Stmt::Match(Box::new(MatchBlock { subject, arms })),
        span,
    ))
}

fn build_if_else_stmt(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let source = pair.as_str();
    let base_offset = pair.as_span().start();
    let children: Vec<Pair> = pair.into_inner().collect();

    // Find the position of each child relative to the if_else_stmt span.
    // Between consecutive children, check the source text for "else if" or "else"
    // keywords to determine grouping.

    // Group children by scanning the source text gaps between them.
    // Groups: [condition, then_stmts...], [else_if_cond, stmts...], ..., [else_stmts...]
    let mut groups: Vec<(bool, Vec<Pair>)> = Vec::new(); // (has_condition, children)
    let mut current_group: Vec<Pair> = Vec::new();
    let mut current_has_cond = true; // First group has the "if" condition

    for (i, child) in children.into_iter().enumerate() {
        if i > 0 {
            // Check source text between previous child's end and this child's start
            let prev_end = current_group.last().map(|p: &Pair| p.as_span().end()).unwrap();
            let this_start = child.as_span().start();
            let gap = &source[(prev_end - base_offset)..(this_start - base_offset)];

            // Look for "else if" or "else" in the gap
            let gap_trimmed = gap.replace(['\n', '\r', ' '], "");
            if gap_trimmed.contains("elseif") {
                // New else-if group
                groups.push((current_has_cond, std::mem::take(&mut current_group)));
                current_has_cond = true;
            } else if gap_trimmed.contains("else") {
                // New else group (no condition)
                groups.push((current_has_cond, std::mem::take(&mut current_group)));
                current_has_cond = false;
            }
        }
        current_group.push(child);
    }
    if !current_group.is_empty() {
        groups.push((current_has_cond, current_group));
    }

    // First group: condition + then body
    let first = groups.remove(0);
    let mut first_iter = first.1.into_iter();
    let condition = build_expr(first_iter.next().unwrap())?;
    let mut then_body = Vec::new();
    for child in first_iter {
        then_body.push(build_statement(child)?);
    }

    let mut else_ifs = Vec::new();
    let mut else_body = None;

    for (has_cond, group_children) in groups {
        if has_cond {
            let mut iter = group_children.into_iter();
            let cond = build_expr(iter.next().unwrap())?;
            let mut body = Vec::new();
            for child in iter {
                body.push(build_statement(child)?);
            }
            else_ifs.push((cond, body));
        } else {
            let mut body = Vec::new();
            for child in group_children {
                body.push(build_statement(child)?);
            }
            else_body = Some(body);
        }
    }

    Ok(Spanned::new(
        Stmt::IfElse(Box::new(IfElseBlock {
            condition,
            then_body,
            else_ifs,
            else_body,
        })),
        span,
    ))
}

fn build_for_loop(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let binding_pair = inner.next().unwrap();
    let binding = spanned(binding_pair.as_str().to_string(), &binding_pair);
    let iterable = build_expr(inner.next().unwrap())?;
    let mut body = Vec::new();
    for child in inner {
        body.push(build_statement(child)?);
    }
    Ok(Spanned::new(
        Stmt::For(Box::new(ForLoop {
            binding,
            iterable,
            body,
        })),
        span,
    ))
}

fn build_statement(pair: Pair) -> anyhow::Result<Spanned<Stmt>> {
    // statement / statement_i3 / statement_i4 / statement_inline all have
    // a single inner child that is the actual statement variant
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::when_block | Rule::when_block_i3 => build_when_block(inner),
        Rule::match_stmt | Rule::match_stmt_i3 => build_match_stmt(inner),
        Rule::if_else_stmt | Rule::if_else_stmt_i3 => build_if_else_stmt(inner),
        Rule::for_loop | Rule::for_loop_i3 => build_for_loop(inner),
        Rule::bind_stmt => build_bind_stmt(inner),
        Rule::give_stmt => build_give_stmt(inner),
        Rule::say_stmt => build_say_stmt(inner),
        Rule::escalate_stmt => build_escalate_stmt(inner),
        Rule::memory_update_stmt => build_memory_update_stmt(inner),
        Rule::emit_stmt => build_emit_stmt(inner),
        Rule::transition_stmt => build_transition_stmt(inner),
        Rule::start_timer_stmt => build_start_timer_stmt(inner),
        Rule::cancel_timer_stmt => build_cancel_timer_stmt(inner),
        Rule::reset_timer_stmt => build_reset_timer_stmt(inner),
        Rule::forward_stmt => build_forward_stmt(inner),
        Rule::expr_stmt => build_expr_stmt(inner),
        _ => Err(parse_error(
            &inner,
            &format!("unexpected statement rule: {:?}", inner.as_rule()),
        )),
    }
}

// ── Declarations ─────────────────────────────────────────────

fn build_use_decl(pair: Pair) -> anyhow::Result<Spanned<TopLevel>> {
    let span = to_span(&pair);
    let mut capabilities = Vec::new();
    for child in pair.into_inner() {
        if child.as_rule() == Rule::cap_path {
            capabilities.push(spanned(child.as_str().to_string(), &child));
        }
    }
    Ok(Spanned::new(
        TopLevel::Use(UseDecl { capabilities }),
        span,
    ))
}

fn build_event_decl(pair: Pair) -> anyhow::Result<Spanned<TopLevel>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);
    let mut fields = Vec::new();

    let remaining: Vec<Pair> = inner.collect();
    let mut i = 0;
    while i + 1 < remaining.len() {
        if remaining[i].as_rule() == Rule::ident && remaining[i + 1].as_rule() == Rule::type_name {
            fields.push(build_field_def(remaining[i].clone(), remaining[i + 1].clone())?);
            i += 2;
        } else {
            i += 1;
        }
    }

    Ok(Spanned::new(
        TopLevel::Event(EventDecl { name, fields }),
        span,
    ))
}

fn build_states_decl(pair: Pair) -> anyhow::Result<Spanned<TopLevel>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);
    let mut transitions = Vec::new();

    for child in inner {
        if child.as_rule() == Rule::state_transition {
            let t_span = to_span(&child);
            let mut t_inner = child.into_inner();
            let from_pair = t_inner.next().unwrap();
            let to_pair = t_inner.next().unwrap();
            let condition = t_inner.next().map(|p| build_expr(p)).transpose()?;
            transitions.push(Spanned::new(
                StateTransition {
                    from: spanned(from_pair.as_str().to_string(), &from_pair),
                    to: spanned(to_pair.as_str().to_string(), &to_pair),
                    condition,
                },
                t_span,
            ));
        }
    }

    Ok(Spanned::new(
        TopLevel::States(StatesDecl { name, transitions }),
        span,
    ))
}

fn build_type_def_decl(pair: Pair) -> anyhow::Result<Spanned<TopLevel>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);
    let mut fields = Vec::new();

    let remaining: Vec<Pair> = inner.collect();
    let mut i = 0;
    while i + 1 < remaining.len() {
        if remaining[i].as_rule() == Rule::ident && remaining[i + 1].as_rule() == Rule::type_name {
            fields.push(build_field_def(remaining[i].clone(), remaining[i + 1].clone())?);
            i += 2;
        } else {
            i += 1;
        }
    }

    Ok(Spanned::new(
        TopLevel::TypeDef(TypeDefDecl { name, fields }),
        span,
    ))
}

fn build_needs_clause(pair: Pair) -> anyhow::Result<Vec<Spanned<Param>>> {
    let mut params = Vec::new();
    for child in pair.into_inner() {
        if child.as_rule() == Rule::param {
            params.push(build_param(child)?);
        }
    }
    Ok(params)
}

fn build_do_block(pair: Pair) -> anyhow::Result<Vec<Spanned<Stmt>>> {
    let mut stmts = Vec::new();
    for child in pair.into_inner() {
        if child.as_rule() == Rule::statement {
            stmts.push(build_statement(child)?);
        }
    }
    Ok(stmts)
}

fn build_task_decl(pair: Pair) -> anyhow::Result<Spanned<TopLevel>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);

    let mut needs = Vec::new();
    let mut gives = None;
    let mut body = None;
    let mut if_fails = None;

    for child in inner {
        match child.as_rule() {
            Rule::needs_clause => {
                needs = build_needs_clause(child)?;
            }
            Rule::gives_clause => {
                let gc_inner = child.into_inner().next().unwrap();
                gives = Some(build_output_type(gc_inner)?);
            }
            Rule::do_block => {
                let stmts = build_do_block(child.clone())?;
                body = Some(spanned(TaskBody::Do(stmts), &child));
            }
            Rule::is_clause => {
                let expr_pair = child.clone().into_inner().next().unwrap();
                let expr = build_compose_expr(expr_pair)?;
                body = Some(spanned(TaskBody::Is(Box::new(expr)), &child));
            }
            Rule::if_fails_block => {
                let mut stmts = Vec::new();
                for stmt_child in child.into_inner() {
                    if stmt_child.as_rule() == Rule::statement {
                        stmts.push(build_statement(stmt_child)?);
                    }
                }
                if_fails = Some(stmts);
            }
            _ => {}
        }
    }

    Ok(Spanned::new(
        TopLevel::Task(TaskDecl {
            name,
            needs,
            gives,
            body: body.unwrap(),
            if_fails,
        }),
        span,
    ))
}

fn build_pure_decl(pair: Pair) -> anyhow::Result<Spanned<TopLevel>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);

    let mut needs = Vec::new();
    let mut gives = None;
    let mut body = Vec::new();

    for child in inner {
        match child.as_rule() {
            Rule::needs_clause => {
                needs = build_needs_clause(child)?;
            }
            Rule::gives_clause => {
                let gc_inner = child.into_inner().next().unwrap();
                gives = Some(build_output_type(gc_inner)?);
            }
            Rule::do_block => {
                body = build_do_block(child)?;
            }
            _ => {}
        }
    }

    Ok(Spanned::new(
        TopLevel::Pure(PureDecl {
            name,
            needs,
            gives,
            body,
        }),
        span,
    ))
}

fn build_endpoint_decl(pair: Pair) -> anyhow::Result<Spanned<TopLevel>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);

    let mut params = Vec::new();
    let mut return_type = None;
    let mut body = Vec::new();

    for child in inner {
        match child.as_rule() {
            Rule::param_list => {
                for p in child.into_inner() {
                    if p.as_rule() == Rule::param {
                        params.push(build_param(p)?);
                    }
                }
            }
            Rule::output_type => {
                return_type = Some(build_output_type(child)?);
            }
            Rule::statement => {
                body.push(build_statement(child)?);
            }
            _ => {}
        }
    }

    Ok(Spanned::new(
        TopLevel::Endpoint(EndpointDecl {
            name,
            params,
            return_type,
            body,
        }),
        span,
    ))
}

fn build_needs_ref(pair: Pair) -> NeedsRef {
    let s = pair.as_str();
    let dot = s.find('.').unwrap();
    let stage = s[..dot].to_string();
    let field_str = &s[dot + 1..];
    let field = if field_str == "*" {
        NeedsRefField::Glob
    } else {
        NeedsRefField::Named(field_str.to_string())
    };
    NeedsRef { stage, field }
}

fn build_stage_block(pair: Pair) -> anyhow::Result<Spanned<StageDecl>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);

    let mut needs = Vec::new();
    let mut body = Vec::new();

    for child in inner {
        match child.as_rule() {
            Rule::needs_ref_list => {
                for ref_child in child.into_inner() {
                    if ref_child.as_rule() == Rule::needs_ref {
                        let ref_span = to_span(&ref_child);
                        needs.push(Spanned::new(build_needs_ref(ref_child), ref_span));
                    }
                }
            }
            Rule::statement => {
                body.push(build_statement(child)?);
            }
            _ => {}
        }
    }

    Ok(Spanned::new(StageDecl { name, needs, body }, span))
}

fn build_flow_decl(pair: Pair) -> anyhow::Result<Spanned<TopLevel>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);

    let mut needs = Vec::new();
    let mut gives = None;
    let mut stages = Vec::new();

    for child in inner {
        match child.as_rule() {
            Rule::needs_clause => {
                needs = build_needs_clause(child)?;
            }
            Rule::gives_clause => {
                let gc_inner = child.into_inner().next().unwrap();
                gives = Some(build_output_type(gc_inner)?);
            }
            Rule::stage_block => {
                stages.push(build_stage_block(child)?);
            }
            _ => {}
        }
    }

    Ok(Spanned::new(
        TopLevel::Flow(FlowDecl {
            name,
            needs,
            gives,
            stages,
        }),
        span,
    ))
}

fn build_fail_policy(pair: Pair) -> anyhow::Result<Spanned<FailPolicy>> {
    let span = to_span(&pair);
    let s = pair.as_str().trim();

    if s == "silent" {
        return Ok(Spanned::new(FailPolicy::Silent, span));
    }
    if s == "log" {
        return Ok(Spanned::new(FailPolicy::Log, span));
    }
    if s == "escalate" {
        return Ok(Spanned::new(FailPolicy::Escalate, span));
    }
    if s == "crash" {
        return Ok(Spanned::new(FailPolicy::Crash, span));
    }

    // "give expr"
    let expr_pair = pair.into_inner().next().unwrap();
    let expr = build_expr(expr_pair)?;
    Ok(Spanned::new(FailPolicy::Give(expr), span))
}

fn build_requires_clause(pair: Pair) -> anyhow::Result<Spanned<RequiresClause>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let condition = build_expr(inner.next().unwrap())?;
    let on_fail = inner.next().map(|p| build_fail_policy(p)).transpose()?;
    Ok(Spanned::new(
        RequiresClause { condition, on_fail },
        span,
    ))
}

fn build_on_handler(pair: Pair) -> anyhow::Result<Spanned<OnHandler>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let event_pair = inner.next().unwrap();
    let event = spanned(event_pair.as_str().to_string(), &event_pair);

    let mut params = Vec::new();
    let mut payload_type = None;
    let mut requires = Vec::new();
    let mut body = Vec::new();

    for child in inner {
        match child.as_rule() {
            Rule::param_list => {
                for p in child.into_inner() {
                    if p.as_rule() == Rule::param {
                        params.push(build_param(p)?);
                    }
                }
            }
            Rule::type_name => {
                payload_type = Some(build_type_name(child)?);
            }
            Rule::requires_clause => {
                requires.push(build_requires_clause(child)?);
            }
            Rule::statement => {
                body.push(build_statement(child)?);
            }
            _ => {}
        }
    }

    Ok(Spanned::new(
        OnHandler {
            event,
            params,
            payload_type,
            requires,
            body,
        },
        span,
    ))
}

fn build_agent_decl(pair: Pair) -> anyhow::Result<Spanned<TopLevel>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);

    let mut lifecycle = None;
    let mut memory = Vec::new();
    let mut timers = Vec::new();
    let mut subscriptions = Vec::new();
    let mut handlers = Vec::new();
    let mut stuck_policy = None;

    for child in inner {
        match child.as_rule() {
            Rule::lifecycle_clause => {
                let lc_ident = child.into_inner().next().unwrap();
                lifecycle = Some(spanned(lc_ident.as_str().to_string(), &lc_ident));
            }
            Rule::memory_block => {
                let mem_children: Vec<Pair> = child.into_inner().collect();
                let mut i = 0;
                while i + 1 < mem_children.len() {
                    if mem_children[i].as_rule() == Rule::ident
                        && mem_children[i + 1].as_rule() == Rule::type_name
                    {
                        memory.push(build_field_def(
                            mem_children[i].clone(),
                            mem_children[i + 1].clone(),
                        )?);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            Rule::timer_field => {
                let tf_span = to_span(&child);
                let mut tf_inner = child.into_inner();
                let timer_name_pair = tf_inner.next().unwrap();
                let timer_name = spanned(timer_name_pair.as_str().to_string(), &timer_name_pair);
                let dur = build_duration(tf_inner.next().unwrap())?;
                timers.push(Spanned::new(
                    TimerField {
                        name: timer_name,
                        duration: dur,
                    },
                    tf_span,
                ));
            }
            Rule::subscribe_line => {
                let sl_span = to_span(&child);
                let mut sl_inner = child.into_inner();
                let ev_pair = sl_inner.next().unwrap();
                let event_name = spanned(ev_pair.as_str().to_string(), &ev_pair);
                let filter = sl_inner.next().map(|p| build_expr(p)).transpose()?;
                subscriptions.push(Spanned::new(
                    SubscribeDecl {
                        event_name,
                        filter,
                    },
                    sl_span,
                ));
            }
            Rule::on_handler => {
                handlers.push(build_on_handler(child)?);
            }
            Rule::stuck_policy => {
                let sp_span = to_span(&child);
                let sp_inner = child.into_inner();
                let mut turns = None;
                let mut sp_body = Vec::new();

                for sp_child in sp_inner {
                    match sp_child.as_rule() {
                        Rule::number_lit => {
                            turns = Some(sp_child.as_str().parse::<u64>()?);
                        }
                        Rule::statement => {
                            sp_body.push(build_statement(sp_child)?);
                        }
                        _ => {}
                    }
                }

                stuck_policy = Some(Spanned::new(
                    StuckPolicy {
                        turns,
                        body: sp_body,
                    },
                    sp_span,
                ));
            }
            _ => {}
        }
    }

    Ok(Spanned::new(
        TopLevel::Agent(AgentDecl {
            name,
            lifecycle,
            memory,
            timers,
            subscriptions,
            handlers,
            stuck_policy,
        }),
        span,
    ))
}

fn build_pool_decl(pair: Pair) -> anyhow::Result<Spanned<TopLevel>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);

    // workers: Type * count
    let worker_type_pair = inner.next().unwrap();
    let worker_type = spanned(worker_type_pair.as_str().to_string(), &worker_type_pair);
    let count_pair = inner.next().unwrap();
    let worker_count = spanned(build_number_lit(count_pair.clone())?, &count_pair);

    // strategy
    let strategy_pair = inner.next().unwrap();
    let strategy = build_pool_strategy(strategy_pair)?;

    let mut timeout = None;
    let mut fallback = None;

    for child in inner {
        match child.as_rule() {
            Rule::duration => {
                timeout = Some(build_duration(child)?);
            }
            Rule::ident => {
                fallback = Some(spanned(child.as_str().to_string(), &child));
            }
            _ => {}
        }
    }

    Ok(Spanned::new(
        TopLevel::Pool(PoolDecl {
            name,
            worker_type,
            worker_count,
            strategy,
            timeout,
            fallback,
        }),
        span,
    ))
}

fn build_pool_strategy(pair: Pair) -> anyhow::Result<Spanned<PoolStrategy>> {
    let span = to_span(&pair);
    let s = pair.as_str().trim();

    if s == "fastest" {
        return Ok(Spanned::new(PoolStrategy::Fastest, span));
    }
    if s == "all" {
        return Ok(Spanned::new(PoolStrategy::All, span));
    }
    if s == "majority" {
        return Ok(Spanned::new(PoolStrategy::Majority, span));
    }
    if s.starts_with("quorum(") {
        let n_pair = pair.into_inner().next().unwrap();
        let n = build_number_lit(n_pair)?;
        return Ok(Spanned::new(PoolStrategy::Quorum(n), span));
    }
    if s.starts_with("first(") {
        let n_pair = pair.into_inner().next().unwrap();
        let n = build_number_lit(n_pair)?;
        return Ok(Spanned::new(PoolStrategy::First(n), span));
    }

    Err(anyhow::anyhow!("unknown pool strategy: {}", s))
}

fn build_contract_decl(pair: Pair) -> anyhow::Result<Spanned<TopLevel>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);
    let mut methods = Vec::new();

    for child in inner {
        if child.as_rule() == Rule::can_signature {
            let sig_span = to_span(&child);
            let mut sig_inner = child.into_inner();
            let method_name_pair = sig_inner.next().unwrap();
            let method_name = method_name_pair.as_str().to_string();

            let mut params = Vec::new();
            let mut return_type = None;

            for sig_child in sig_inner {
                match sig_child.as_rule() {
                    Rule::param_list => {
                        for p in sig_child.into_inner() {
                            if p.as_rule() == Rule::param {
                                params.push(build_param(p)?);
                            }
                        }
                    }
                    Rule::type_name => {
                        return_type = Some(build_type_name(sig_child)?);
                    }
                    _ => {}
                }
            }

            methods.push(Spanned::new(
                CanSignature {
                    name: method_name,
                    params,
                    return_type: return_type.unwrap(),
                },
                sig_span,
            ));
        }
    }

    Ok(Spanned::new(
        TopLevel::Contract(ContractDecl { name, methods }),
        span,
    ))
}

fn build_system_decl(pair: Pair) -> anyhow::Result<Spanned<TopLevel>> {
    let span = to_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().unwrap();
    let name = spanned(name_pair.as_str().to_string(), &name_pair);

    let mut bindings = Vec::new();
    let mut wiring = Vec::new();

    // Collect remaining children
    let remaining: Vec<Pair> = inner.collect();
    let mut i = 0;

    // Look for pairs of ident : ident (bindings from the use block)
    while i + 1 < remaining.len() {
        if remaining[i].as_rule() == Rule::ident && remaining[i + 1].as_rule() == Rule::ident {
            let alias_span = to_span(&remaining[i]);
            bindings.push(Spanned::new(
                SystemBinding {
                    alias: remaining[i].as_str().to_string(),
                    target: remaining[i + 1].as_str().to_string(),
                },
                alias_span,
            ));
            i += 2;
        } else {
            break;
        }
    }

    // Remaining are wiring expressions (compose_expr)
    while i < remaining.len() {
        let child = &remaining[i];
        if child.as_rule() == Rule::compose_expr {
            wiring.push(build_compose_expr(child.clone())?);
        }
        i += 1;
    }

    Ok(Spanned::new(
        TopLevel::System(SystemDecl {
            name,
            bindings,
            wiring,
        }),
        span,
    ))
}

fn build_fn_main_decl(pair: Pair) -> anyhow::Result<Spanned<TopLevel>> {
    let span = to_span(&pair);
    let mut body = Vec::new();
    for child in pair.into_inner() {
        if child.as_rule() == Rule::statement {
            body.push(build_statement(child)?);
        }
    }
    Ok(Spanned::new(
        TopLevel::FnMain(FnMainDecl { body }),
        span,
    ))
}

fn build_boundary_directive(pair: Pair) -> anyhow::Result<Spanned<BoundaryDirective>> {
    let span = to_span(&pair);
    let kind_pair = pair.into_inner().next().unwrap();
    let kind = match kind_pair.as_str() {
        "server" => BoundaryKind::Server,
        "client" => BoundaryKind::Client,
        "shared" => BoundaryKind::Shared,
        other => return Err(parse_error(&kind_pair, &format!("unknown boundary: {}", other))),
    };
    Ok(Spanned::new(
        BoundaryDirective {
            kind: spanned(kind, &kind_pair),
        },
        span,
    ))
}

// ── Top-level ────────────────────────────────────────────────

fn build_program(pairs: Pairs) -> anyhow::Result<Program> {
    let mut boundary = None;
    let mut items = Vec::new();

    // The top-level Pairs has one `program` pair — unwrap into its children
    let program_pair = pairs.into_iter().next().unwrap();
    for pair in program_pair.into_inner() {
        match pair.as_rule() {
            Rule::boundary_directive => {
                boundary = Some(build_boundary_directive(pair)?);
            }
            Rule::top_level => {
                let inner = pair.into_inner().next().unwrap();
                let item = match inner.as_rule() {
                    Rule::use_decl => build_use_decl(inner)?,
                    Rule::task_decl => build_task_decl(inner)?,
                    Rule::pure_decl => build_pure_decl(inner)?,
                    Rule::event_decl => build_event_decl(inner)?,
                    Rule::states_decl => build_states_decl(inner)?,
                    Rule::type_decl => build_type_def_decl(inner)?,
                    Rule::endpoint_decl => build_endpoint_decl(inner)?,
                    Rule::flow_decl => build_flow_decl(inner)?,
                    Rule::agent_decl => build_agent_decl(inner)?,
                    Rule::pool_decl => build_pool_decl(inner)?,
                    Rule::contract_decl => build_contract_decl(inner)?,
                    Rule::system_decl => build_system_decl(inner)?,
                    Rule::fn_main_decl => build_fn_main_decl(inner)?,
                    _ => {
                        return Err(parse_error(
                            &inner,
                            &format!("unexpected top-level rule: {:?}", inner.as_rule()),
                        ))
                    }
                };
                items.push(item);
            }
            Rule::EOI => {}
            _ => {}
        }
    }

    Ok(Program { boundary, items })
}

// ── Public API ───────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("{message}")]
    Syntax {
        message: String,
        span_start: usize,
        span_end: usize,
    },
    #[error("{0}")]
    Internal(String),
}

impl ParseError {
    pub fn to_diagnostic(&self, file: &str) -> crate::diagnostic::Diagnostic {
        match self {
            ParseError::Syntax { message, span_start, span_end } => {
                crate::diagnostic::Diagnostic::error(
                    file,
                    message.clone(),
                    *span_start..*span_end,
                    "parse error here",
                )
            }
            ParseError::Internal(msg) => {
                crate::diagnostic::Diagnostic::error(
                    file,
                    msg.clone(),
                    0..0,
                    msg.clone(),
                )
            }
        }
    }
}

pub fn parse(source: &str) -> Result<Program, ParseError> {
    let pairs = ForgeParser::parse(Rule::program, source)
        .map_err(|e| {
            let (start, end) = match e.location {
                pest::error::InputLocation::Pos(p) => (p, (p + 1).min(source.len())),
                pest::error::InputLocation::Span((s, e)) => (s, e),
            };
            // Extract just the variant description (e.g. "expected statement")
            let message = match &e.variant {
                pest::error::ErrorVariant::ParsingError { positives, negatives } => {
                    let mut parts = Vec::new();
                    if !positives.is_empty() {
                        let names: Vec<String> = positives.iter()
                            .map(|r| format!("{:?}", r).to_lowercase().replace('_', " "))
                            .collect();
                        parts.push(format!("expected {}", names.join(", ")));
                    }
                    if !negatives.is_empty() {
                        let names: Vec<String> = negatives.iter()
                            .map(|r| format!("{:?}", r).to_lowercase().replace('_', " "))
                            .collect();
                        parts.push(format!("unexpected {}", names.join(", ")));
                    }
                    if parts.is_empty() { "syntax error".to_string() } else { parts.join("; ") }
                }
                pest::error::ErrorVariant::CustomError { message } => message.clone(),
            };
            ParseError::Syntax {
                message,
                span_start: start,
                span_end: end,
            }
        })?;

    build_program(pairs).map_err(|e| ParseError::Internal(e.to_string()))
}
