use crate::fleet::spec_model::*;

pub fn generate(model: &SpecModel) -> String {
    let mut buf = String::new();

    emit_use_block(&mut buf, &model.capabilities);
    for event in &model.events {
        emit_event(&mut buf, event);
    }
    for agent in &model.agents {
        if let Some(states) = &agent.states {
            emit_states(&mut buf, states);
        }
    }
    for ty in &model.types {
        emit_type_decl(&mut buf, ty);
    }
    for agent in &model.agents {
        emit_agent(&mut buf, agent);
    }
    for flow in &model.flows {
        emit_flow(&mut buf, flow);
    }
    if !model.agents.is_empty() {
        emit_system(&mut buf, model);
    }

    buf
}

fn line(buf: &mut String, indent: usize, text: &str) {
    for _ in 0..indent {
        buf.push_str("  ");
    }
    buf.push_str(text);
    buf.push('\n');
}

fn blank(buf: &mut String) {
    buf.push('\n');
}

fn emit_use_block(buf: &mut String, capabilities: &[String]) {
    if capabilities.is_empty() {
        return;
    }
    line(buf, 0, "use");
    for cap in capabilities {
        line(buf, 1, cap);
    }
    blank(buf);
}

fn emit_event(buf: &mut String, event: &EventSpec) {
    line(buf, 0, &format!("event {}", event.name));
    for field in &event.fields {
        line(buf, 1, &format!("{}: {}", field.name, field.type_name));
    }
    blank(buf);
}

fn emit_states(buf: &mut String, states: &StatesSpec) {
    line(buf, 0, &format!("states {}", states.name));
    for (from, to) in &states.transitions {
        line(buf, 1, &format!("{} -> {}", from, to));
    }
    blank(buf);
}

fn emit_type_decl(buf: &mut String, ty: &TypeSpec) {
    line(buf, 0, &format!("type {}", ty.name));
    for field in &ty.fields {
        line(buf, 1, &format!("{}: {}", field.name, field.type_name));
    }
    blank(buf);
}

fn emit_agent(buf: &mut String, agent: &AgentSpec) {
    line(buf, 0, &format!("agent {}", agent.name));

    if let Some(states) = &agent.states {
        line(buf, 1, &format!("lifecycle: {}", states.name));
    }

    if !agent.memory_fields.is_empty() {
        line(buf, 1, "memory");
        for field in &agent.memory_fields {
            line(buf, 2, &format!("{}: {}", field.name, field.type_name));
        }
    }

    for sub in &agent.subscriptions {
        line(buf, 1, &format!("subscribe {}", sub));
    }

    for handler in &agent.handlers {
        blank(buf);
        emit_handler(buf, handler, &agent.name);
    }
    blank(buf);
}

fn emit_handler(buf: &mut String, handler: &HandlerSpec, agent_name: &str) {
    let params_str = if handler.params.is_empty() {
        String::new()
    } else {
        let parts: Vec<String> = handler
            .params
            .iter()
            .map(|p| format!("{}: {}", p.name, p.type_name))
            .collect();
        format!("({})", parts.join(", "))
    };
    line(buf, 1, &format!("on {}{}", handler.event_name, params_str));

    // Stub body: a reason call + say so the handler parses.
    // TODO hint is inline because standalone comment lines don't parse inside handlers.
    let hint = handler
        .todo_hint
        .as_deref()
        .unwrap_or("implement logic");
    let first_param = handler
        .params
        .first()
        .map(|p| format!("{{{}}}", p.name))
        .unwrap_or_else(|| "request".to_string());
    line(
        buf,
        2,
        &format!(
            "response = reason \"TODO {}: Handle as {}: {}\"",
            hint, agent_name, first_param
        ),
    );
    line(buf, 2, "say response");
}

fn emit_flow(buf: &mut String, flow: &FlowSpec) {
    line(buf, 0, &format!("flow {}", flow.name));

    if let Some(input) = &flow.input {
        line(buf, 1, &format!("needs {}: {}", input.name, input.type_name));
    }
    line(buf, 1, "gives Text");
    blank(buf);

    for (i, stage) in flow.stages.iter().enumerate() {
        line(buf, 1, &format!("stage {}", stage.name));
        if !stage.needs_refs.is_empty() {
            let refs: Vec<String> = stage
                .needs_refs
                .iter()
                .map(|r| format!("{}.result", r))
                .collect();
            line(buf, 2, &format!("needs {}", refs.join(", ")));
        }
        if stage.needs_refs.is_empty() {
            // First stage or independent stage — use input or generic prompt
            if let Some(input) = &flow.input {
                line(
                    buf,
                    2,
                    &format!(
                        "result = reason \"Process {}: {{{}}}\"",
                        stage.name, input.name
                    ),
                );
            } else {
                line(
                    buf,
                    2,
                    &format!("result = reason \"Process {} stage\"", stage.name),
                );
            }
        } else {
            // Dependent stage — reference upstream
            let upstream_refs: Vec<String> = stage
                .needs_refs
                .iter()
                .map(|r| format!("{{{}.result}}", r))
                .collect();
            line(
                buf,
                2,
                &format!(
                    "result = reason \"Process {}: {}\"",
                    stage.name,
                    upstream_refs.join(" ")
                ),
            );
        }

        if i < flow.stages.len() - 1 {
            blank(buf);
        }
    }

    // Last stage gives its result
    if let Some(last) = flow.stages.last() {
        blank(buf);
        line(buf, 1, "stage output");
        let last_ref = format!("{}.result", last.name);
        line(buf, 2, &format!("needs {}", last_ref));
        line(buf, 2, &format!("give {}", last_ref));
    }
    blank(buf);
}

fn emit_system(buf: &mut String, model: &SpecModel) {
    line(buf, 0, &format!("system {}", model.system_name));

    if !model.agents.is_empty() {
        line(buf, 1, "use");
        for agent in &model.agents {
            let alias = format!("{}_node", agent.name);
            line(buf, 2, &format!("{}: {}", alias, agent.name));
        }

        // Wire agents in sequence with >>
        if model.agents.len() > 1 {
            let chain: Vec<String> = model
                .agents
                .iter()
                .map(|a| format!("{}_node", a.name))
                .collect();
            line(buf, 1, &chain.join(" >> "));
        }
    }
    blank(buf);
}
