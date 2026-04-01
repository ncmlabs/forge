# FORGE Language — POC Implementation Plan v3

> Feed this to Claude Code. This supersedes v2.
> v3 adds six new primitives discovered by stress-testing the language
> against a real distributed system (multiplayer tic-tac-toe).
> These are not libraries — they are language-level additions.

---

## What changed from v2

v2 gave FORGE: `task`, `flow`, `agent`, `pool`, `system`, `use`, `when` confidence predicates,
and `>>` composition.

v3 adds six primitives that games — and distributed systems generally — require:

| New primitive | What it solves |
|---|---|
| `pure` | Deterministic functions with zero LLM/cost/uncertainty |
| `event` | Broadcast streams, not just point-to-point channels |
| `states` | Typed lifecycle machines on agents |
| `timer` | First-class timers that fire handlers independently |
| `boundary` | Server vs client code split, enforced by compiler |
| `requires` | Precondition guards on handlers, separate from body logic |

All six generalize beyond games. Orders have lifecycles. Chat needs broadcast.
Auctions need timers. Any client-server system needs boundary enforcement.

---

## Full language primitives (v3)

```
task        stochastic LLM computation → uncertain<T>
flow        multi-stage pipeline, auto-parallelized by stage deps
agent       stateful long-running supervised process
pool        worker group with resolution strategy
system      top-level wiring of all above
use         capability declaration (llm.reason, ws.server, data.store)
pure        deterministic function — no LLM, no side effects, compile-enforced
event       typed broadcast stream — emit/subscribe model
states      lifecycle state machine on an agent — illegal transitions = compile error
timer       named countdown that fires a handler — cancellable
boundary    server | client | shared — enforced code split
requires    precondition guards on handlers — separate from body
when        confidence-based branching on uncertain<T> values
>>          composition operator — any primitive composes with any other
```

---

## Project structure (v3)

```
forge/
├── Cargo.toml
├── grammar/
│   └── forge.pest              # PEG grammar — extended for new primitives
├── src/
│   ├── main.rs                 # CLI: parse / check / run / cost
│   ├── ast.rs                  # AST — adds PureDecl, EventDecl, StatesDecl,
│   │                           #        TimerDecl, BoundaryDecl, RequiresClause
│   ├── parser.rs               # pest → AST
│   ├── resolver.rs             # capability resolution + composition type check
│   ├── checker/
│   │   ├── mod.rs              # type checker coordinator
│   │   ├── uncertain.rs        # uncertain<T> must be matched before use
│   │   ├── pure_checker.rs     # NEW: no think/tool calls inside pure
│   │   ├── states_checker.rs   # NEW: invalid transitions = compile error
│   │   ├── boundary_checker.rs # NEW: server code can't leak into client
│   │   └── requires_checker.rs # NEW: requires guards are satisfiable
│   ├── planner.rs              # DAG builder from flow stages
│   ├── runtime/
│   │   ├── mod.rs
│   │   ├── executor.rs         # task + flow + pure execution
│   │   ├── agent.rs            # stateful agent process
│   │   ├── pool.rs             # worker pool with strategies
│   │   ├── confidence.rs       # confidence model + predicates
│   │   ├── memory.rs           # agent memory with auto-compaction
│   │   ├── event_bus.rs        # NEW: broadcast event streams
│   │   ├── timer_engine.rs     # NEW: named timer management
│   │   └── state_machine.rs    # NEW: lifecycle enforcement at runtime
│   ├── llm/
│   │   ├── mod.rs              # LLMBackend trait
│   │   ├── anthropic.rs        # Anthropic API
│   │   └── mock.rs             # deterministic mock for tests
│   └── tracer.rs               # structured execution traces
├── examples/
│   ├── hello.forge             # simplest task
│   ├── classify.forge          # confidence-aware classification
│   ├── research.forge          # multi-stage flow
│   ├── tictactoe/
│   │   ├── room_agent.forge    # uses: states, timer, requires, pure
│   │   ├── matchmaking.forge   # uses: flow, event
│   │   ├── ai_opponent.forge   # uses: agent, think, when
│   │   └── platform.forge      # uses: system, boundary, pool
│   └── support_bot.forge       # stateful agent with memory
└── tests/
    ├── parser_tests.rs
    ├── pure_checker_tests.rs   # NEW
    ├── states_checker_tests.rs # NEW
    ├── boundary_tests.rs       # NEW
    ├── timer_tests.rs          # NEW
    └── runtime_tests.rs
```

---

## New primitive 1 — `pure`

### What it is

A deterministic function. No LLM calls, no tool calls, no network, no side effects.
Always returns the same output for the same input. Zero cost. Instantaneous.

The compiler enforces all of these constraints. If you write `reason` inside a `pure`
block, it is a compile error, not a runtime error.

### Syntax

```forge
pure check_winner
  needs board: Text[9]
  gives WinResult

  do
    WIN = [[0,1,2],[3,4,5],[6,7,8],[0,3,6],[1,4,7],[2,5,8],[0,4,8],[2,4,6]]
    for line in WIN
      sym = board[line[0]]
      if sym != "" and sym == board[line[1]] and sym == board[line[2]]
        give Winner(sym)
    if board.none(empty)
      give Draw
    give Ongoing

pure valid_move
  needs board: Text[9], cell: Number
  gives Bool
  do
    give cell >= 0 and cell <= 8 and board[cell] == ""

pure next_turn
  needs current: Number
  gives Number
  do
    give 1 - current
```

### AST node

```rust
pub struct PureDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: TypeExpr,
    pub body: Vec<Stmt>,    // no ThinkExpr, no ToolCall allowed inside
}
```

### Checker rules (`src/checker/pure_checker.rs`)

Walk the body AST. If any of these appear inside a `pure` decl, emit a compile error:
- `Expr::Think(_)` — LLM call
- `Expr::Reason(_)` — LLM call
- `Expr::Search(_)` — tool call
- `Stmt::Escalate(_)` — side effect
- `Expr::Call(name, _)` where `name` is a `task` — tasks may be stochastic

`pure` can call other `pure` functions freely.

### Runtime execution

`pure` functions execute synchronously, inline, with no async. They never touch the
LLM backend or tracer. They are compiled to the most efficient native code path.

---

## New primitive 2 — `event`

### What it is

A typed broadcast stream. Any agent or system component can `emit` an event.
Any number of subscribers receive it. Unlike `channel<T>` (point-to-point, one sender,
one receiver), `event<T>` is one-to-many.

### Syntax

```forge
# declare the event type
event MoveEvent
  room_id:   Text
  player:    Player
  cell:      Number
  symbol:    Text
  board:     Text[9]
  next_turn: Number

event GameEndEvent
  room_id: Text
  result:  WinResult
  ratings: RatingUpdate[]

# emit from an agent
agent RoomAgent
  on move(player: Player, cell: Number)
    requires lifecycle == playing
    memory.board[cell] = player.symbol
    winner = check_winner(memory.board)
    emit MoveEvent(memory.id, player, cell, player.symbol, memory.board, memory.turn)
    when winner == Winner(_)
      delta = compute_elo(memory.players, winner)
      emit GameEndEvent(memory.id, winner, delta)
      transition to done

# subscribe in another agent
agent LeaderboardAgent
  subscribe GameEndEvent
  on GameEndEvent(e)
    update_ratings(e.ratings)
    say "Leaderboard updated after room {e.room_id}"

# subscribe with filter
agent SpectatorAgent
  memory
    watching: Text    # room_id being watched

  subscribe MoveEvent where event.room_id == memory.watching
  on MoveEvent(e)
    forward e to memory.client_socket
```

### AST additions

```rust
pub struct EventDecl {
    pub name: String,
    pub fields: Vec<EventField>,
}

pub enum Stmt {
    // ... existing
    Emit(String, Vec<Expr>),   // emit EventName(args...)
    Subscribe(SubscribeDecl),  // subscribe EventName [where filter]
    Forward(Expr, Expr),       // forward event to target
}

pub struct SubscribeDecl {
    pub event_name: String,
    pub filter: Option<Expr>,      // the `where` clause
    pub handler_name: String,      // which `on` block handles it
}
```

### Runtime implementation (`src/runtime/event_bus.rs`)

```rust
pub struct EventBus {
    // event_name → list of (filter, sender) pairs
    pub subscribers: HashMap<String, Vec<Subscriber>>,
}

pub struct Subscriber {
    pub agent_id: String,
    pub filter: Option<FilterFn>,
    pub channel: tokio::sync::mpsc::Sender<EventPayload>,
}

impl EventBus {
    pub async fn emit(&self, event_name: &str, payload: EventPayload) {
        let subs = self.subscribers.get(event_name).unwrap_or(&vec![]);
        for sub in subs {
            if sub.filter.as_ref().map_or(true, |f| f(&payload)) {
                let _ = sub.channel.send(payload.clone()).await;
            }
        }
    }

    pub fn subscribe(
        &mut self,
        event_name: &str,
        agent_id: &str,
        filter: Option<FilterFn>,
    ) -> tokio::sync::mpsc::Receiver<EventPayload>;
}
```

Event delivery is async and non-blocking. An agent that subscribes to an event gets
its own channel. The event bus delivers to all subscribers concurrently.

---

## New primitive 3 — `states`

### What it is

A typed state machine declaration on an agent. Defines legal states, legal transitions
between them, and the conditions that trigger transitions. Illegal transitions are
compile errors. Illegal `on` handlers for the current state are runtime guards.

### Syntax

```forge
states RoomLifecycle
  waiting  -> playing   when players.count == 2
  playing  -> done      when winner_found or board_full
  playing  -> abandoned when both_disconnected and timer.expired
  # any other transition is illegal — compiler enforces

agent RoomAgent
  lifecycle: RoomLifecycle   # agent has a lifecycle

  memory
    board:   Text[9]
    players: Player[]
    turn:    Number

  on join(player: Player)
    requires lifecycle == waiting     # handler only valid in this state
    memory.players = memory.players + player
    if memory.players.count == 2
      transition to playing           # compiler checks: waiting → playing is legal
      emit GameStartEvent(memory.id, memory.players)

  on move(player: Player, cell: Number)
    requires lifecycle == playing     # compile error if called in wrong state
    memory.board[cell] = player.symbol
    result = check_winner(memory.board)
    when result == Winner(_)
      transition to done              # compiler checks: playing → done is legal
    when result == Draw
      transition to done
```

### AST additions

```rust
pub struct StatesDecl {
    pub name: String,
    pub transitions: Vec<Transition>,
}

pub struct Transition {
    pub from: String,
    pub to: String,
    pub condition: Option<Expr>,   // the `when` guard
}

// In AgentDecl:
pub struct AgentDecl {
    pub name: String,
    pub lifecycle: Option<String>,   // states declaration name
    // ... rest of fields
}

// In Stmt:
pub enum Stmt {
    // ... existing
    TransitionTo(String),   // transition to <state_name>
}

// In HandleDecl — a requires on the lifecycle state:
pub struct HandleDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub lifecycle_requires: Option<String>,   // requires lifecycle == X
    pub requires: Vec<RequiresClause>,
    pub body: Vec<Stmt>,
}
```

### Checker (`src/checker/states_checker.rs`)

Build the transition graph from `StatesDecl`. Then for every agent that declares a
lifecycle:

1. Every `transition to X` in handler H must have a legal edge from H's
   `requires lifecycle == Y` to X. If not — compile error.
2. Every `on handler` that declares `requires lifecycle == X` cannot be called
   from a state where X is not the current lifecycle. Verify statically where possible.
3. States with no outgoing transitions are terminal — check that the agent handles
   the terminal state gracefully.

### Runtime (`src/runtime/state_machine.rs`)

```rust
pub struct StateMachine {
    pub current: String,
    pub graph: HashMap<String, Vec<TransitionEdge>>,
}

impl StateMachine {
    pub fn transition(&mut self, to: &str) -> Result<(), StateError> {
        let legal = self.graph.get(&self.current)
            .map_or(false, |edges| edges.iter().any(|e| e.to == to));
        if legal {
            self.current = to.to_string();
            Ok(())
        } else {
            Err(StateError::IllegalTransition {
                from: self.current.clone(),
                to: to.to_string(),
            })
        }
    }

    pub fn is_in(&self, state: &str) -> bool {
        self.current == state
    }
}
```

---

## New primitive 4 — `timer`

### What it is

A named countdown timer that lives on an agent. When it expires, it fires a specific
`on` handler. Timers can be started, reset, and cancelled from within handlers.
They fire independently — the agent does not block while a timer is running.

### Syntax

```forge
agent RoomAgent
  lifecycle: RoomLifecycle

  timer reconnect_window: 30s    # fires on_reconnect_window_expired
  timer turn_limit: 15s          # fires on_turn_limit_expired
  timer idle_check: 2min         # fires on_idle_check_expired

  on disconnect(player: Player)
    requires lifecycle == playing
    start reconnect_window for player    # timer starts, handler returns
    broadcast "opponent_disconnected" to other_player(player)

  on reconnect(player: Player)
    cancel reconnect_window for player   # player made it back
    broadcast "opponent_reconnected" to other_player(player)

  on reconnect_window.expired(player: Player)
    forfeit(player)
    transition to done

  on move(player: Player, cell: Number)
    requires lifecycle == playing
    requires valid_move(memory.board, cell)
    reset turn_limit              # player moved, restart the clock
    memory.board[cell] = player.symbol
    ...

  on turn_limit.expired
    requires lifecycle == playing
    idle = memory.players[memory.turn]
    forfeit(idle)
    transition to done
```

### AST additions

```rust
pub struct TimerDecl {
    pub name: String,
    pub duration: Duration,
    // handler name is derived: timer "foo" → handler "on foo.expired"
}

pub enum Stmt {
    // ... existing
    StartTimer { name: String, context: Option<Expr> },  // start X for Y
    CancelTimer { name: String, context: Option<Expr> }, // cancel X for Y
    ResetTimer(String),                                   // reset X
}
```

### Runtime (`src/runtime/timer_engine.rs`)

```rust
pub struct TimerEngine {
    // timer_name → list of active instances (each has an optional context value)
    pub active: HashMap<String, Vec<TimerInstance>>,
    pub sender: tokio::sync::mpsc::Sender<TimerFired>,
}

pub struct TimerInstance {
    pub context: Option<Value>,     // the "for player" context
    pub expires_at: Instant,
    pub cancel_tx: tokio::sync::oneshot::Sender<()>,
}

pub struct TimerFired {
    pub timer_name: String,
    pub context: Option<Value>,
    pub agent_id: String,
}

impl TimerEngine {
    pub fn start(
        &mut self,
        timer_name: &str,
        duration: Duration,
        context: Option<Value>,
        agent_id: &str,
    ) {
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let sender = self.sender.clone();
        let name = timer_name.to_string();
        let ctx = context.clone();
        let aid = agent_id.to_string();

        tokio::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(duration) => {
                    let _ = sender.send(TimerFired {
                        timer_name: name, context: ctx, agent_id: aid
                    }).await;
                }
                _ = cancel_rx => {}   // cancelled, do nothing
            }
        });

        self.active.entry(timer_name.to_string())
            .or_default()
            .push(TimerInstance { context, expires_at: Instant::now() + duration, cancel_tx });
    }

    pub fn cancel(&mut self, timer_name: &str, context: Option<&Value>) {
        // find matching instance, send cancel signal
    }

    pub fn reset(&mut self, timer_name: &str, duration: Duration, agent_id: &str) {
        self.cancel(timer_name, None);
        self.start(timer_name, duration, None, agent_id);
    }
}
```

The agent's main loop listens on both its message channel AND the timer engine's
`TimerFired` channel. When a timer fires, it dispatches to the correct `on X.expired`
handler.

---

## New primitive 5 — `boundary`

### What it is

A code partition that enforces where code runs. `server` code runs on the backend —
it has access to agents, LLMs, databases. `client` code runs in the browser or app —
it renders UI and sends requests. `shared` defines types that cross the wire and are
automatically serialized.

The compiler refuses to include `server` declarations in the client bundle, and refuses
to let `client` code call `server` agents directly. All cross-boundary communication
goes through explicit `shared` message types.

### Syntax

```forge
boundary shared
  # types that cross the wire — auto-serialized to JSON/msgpack
  type MoveRequest
    room_id:  Text
    cell:     Number
    token:    Text        # auth token

  type GameState
    board:    Text[9]
    turn:     Number
    status:   Text
    players:  Player[2]

  type GameEvent
    kind:     Text        # "move" | "game_end" | "disconnect"
    payload:  GameState

boundary server
  # only runs server-side — compiler refuses to bundle into client
  pure check_winner ...
  pure valid_move ...
  agent RoomAgent ...
  flow matchmaking ...
  pool room_pool ...

  # server-side entry points — how client talks to server
  endpoint move(req: MoveRequest, ctx: AuthContext) -> GameState or MoveError
    agent = room_pool.get(req.room_id)
    agent.send(move, ctx.player, req.cell)

  endpoint join_queue(player: Player) -> RoomAssignment
    matchmaking(player)

boundary client
  # only runs in browser / app
  # cannot import anything from boundary server directly

  task render_board
    needs state: GameState
    gives HTML
    do
      ...

  task handle_click
    needs event: ClickEvent, state: GameState
    gives MoveRequest or Nothing
    do
      cell = event.target.cell
      when valid_move(state.board, cell)
        give MoveRequest(state.room_id, cell, auth.token)
      else
        give Nothing

  # websocket subscription — receives GameEvent from server
  on server_event(e: GameEvent)
    new_state = apply_event(current_state, e)
    render_board(new_state)
```

### AST additions

```rust
pub enum BoundaryKind { Server, Client, Shared }

pub struct BoundaryDecl {
    pub kind: BoundaryKind,
    pub items: Vec<TopLevel>,   // tasks, agents, pure, types, endpoints
}

pub struct EndpointDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Vec<TypeExpr>,
    pub body: Vec<Stmt>,
}
```

### Checker (`src/checker/boundary_checker.rs`)

1. Build two symbol tables: `server_symbols` and `client_symbols`.
2. Walk every `client` declaration. If it references any name in `server_symbols` — error.
3. Walk every `server` declaration. If it references any name in `client_symbols` — error.
4. Types in `shared` must be serializable (no agent refs, no function types, no channels).
5. `endpoint` declarations in `server` are the only legal way for `client` code to call
   server-side logic. The compiler generates the wire protocol for each endpoint.

### Compile targets

```bash
forge build --boundary server --target wasm32-wasi    # server binary
forge build --boundary client --target wasm32-browser  # client bundle
forge build --boundary all    --target native          # monolith for testing
```

---

## New primitive 6 — `requires`

### What it is

Precondition guards on `on` handlers, declared separately from the handler body.
Each `requires` clause has an optional `on fail` policy — what to do if the
precondition is not met. The body only runs when all requires clauses pass.

This replaces the `when ... and ... and ...` pattern inside handler bodies, which
mixed preconditions with logic.

### Syntax

```forge
on move(player: Player, cell: Number)
  requires lifecycle == playing              on fail: silent
  requires memory.players[memory.turn] == player  on fail: give OutOfTurn
  requires valid_move(memory.board, cell)    on fail: give InvalidCell

  do
    # body only reached when all requires pass
    memory.board[cell] = player.symbol
    result = check_winner(memory.board)
    ...

on join(player: Player)
  requires lifecycle == waiting    on fail: give RoomFull
  requires memory.players.count < 2   on fail: give RoomFull

  do
    memory.players = memory.players + player
    ...
```

### On fail policies

```
on fail: silent         # reject the message, no response, no log
on fail: give <expr>    # reject and send this value back to caller
on fail: log            # reject and emit a warning to tracer
on fail: escalate       # reject and escalate to supervisor
on fail: crash          # treat as a bug — crash the agent (supervisor restarts)
```

Default when `on fail` is omitted: `silent` for lifecycle requires, `log` for others.

### AST additions

```rust
pub struct RequiresClause {
    pub condition: Expr,
    pub on_fail: FailPolicy,
}

pub enum FailPolicy {
    Silent,
    Give(Expr),
    Log,
    Escalate,
    Crash,
}

pub struct HandleDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub requires: Vec<RequiresClause>,   // evaluated before body
    pub body: Vec<Stmt>,
}
```

### Runtime execution order

For every `on` handler invocation:
1. Evaluate all `requires` clauses in declaration order
2. On first failure: execute its `on fail` policy and return
3. If all pass: execute the body

This is fast — requires clauses on `pure` functions are evaluated synchronously with
no async overhead.

---

## Updated grammar additions (`grammar/forge.pest`)

```pest
// Pure function
pure_decl = {
    "pure" ~ ident ~ NEWLINE ~
    (indent ~ "needs" ~ param_list ~ NEWLINE)? ~
    (indent ~ "gives" ~ type_expr ~ NEWLINE)? ~
    indent ~ "do" ~ NEWLINE ~ stmt_block
}

// Event declaration
event_decl = {
    "event" ~ ident ~ NEWLINE ~
    (indent ~ ident ~ ":" ~ type_expr ~ NEWLINE)+
}

// Emit and subscribe statements
emit_stmt      = { "emit" ~ ident ~ call_args ~ NEWLINE }
subscribe_stmt = { "subscribe" ~ ident ~ ("where" ~ expr)? ~ NEWLINE }
forward_stmt   = { "forward" ~ expr ~ "to" ~ expr ~ NEWLINE }

// States declaration
states_decl = {
    "states" ~ ident ~ NEWLINE ~
    (indent ~ state_transition ~ NEWLINE)+
}
state_transition = {
    ident ~ "->" ~ ident ~
    ("when" ~ expr)?
}
transition_stmt = { "transition" ~ "to" ~ ident ~ NEWLINE }

// Timer declaration (inside agent)
timer_decl  = { "timer" ~ ident ~ ":" ~ duration_lit }
duration_lit = @{ ASCII_DIGIT+ ~ ("s" | "min" | "h") }

// Timer control statements
start_timer_stmt  = { "start" ~ ident ~ ("for" ~ expr)? ~ NEWLINE }
cancel_timer_stmt = { "cancel" ~ ident ~ ("for" ~ expr)? ~ NEWLINE }
reset_timer_stmt  = { "reset" ~ ident ~ NEWLINE }

// Boundary
boundary_decl = {
    "boundary" ~ boundary_kind ~ NEWLINE ~
    boundary_item+
}
boundary_kind = { "server" | "client" | "shared" }

// Endpoint (inside boundary server)
endpoint_decl = {
    "endpoint" ~ ident ~ "(" ~ param_list? ~ ")" ~
    ("->" ~ output_type)? ~ NEWLINE ~
    stmt_block
}

// Requires clause on handlers
requires_clause = {
    indent{2} ~ "requires" ~ expr ~
    ("on" ~ "fail" ~ ":" ~ fail_policy)? ~ NEWLINE
}
fail_policy = {
    "silent" | "log" | "escalate" | "crash" |
    "give" ~ expr
}

// Updated handle declaration includes requires before do
handle_decl = {
    indent ~ "on" ~ ident ~ "(" ~ param_list? ~ ")" ~ NEWLINE ~
    requires_clause* ~
    (indent{2} ~ "do" ~ NEWLINE ~ stmt_block)?
}
```

---

## Implementation phases (v3)

Phases 1–6 from v2 still apply (parser, type system, agent+think, plan+parallelism,
supervision, CLI). These new phases slot in after the base is working.

### Phase 7 — `pure` functions

**Goal:** Parse, check, and execute `pure` declarations.

1. Add `pure_decl` to grammar
2. Add `PureDecl` to AST
3. Implement `pure_checker.rs` — walk body, error on any stochastic expr
4. Execute pure functions synchronously in the runtime (no async, no LLM)
5. Test: `check_winner` and `valid_move` from the tic-tac-toe example

**Acceptance:** `forge check tictactoe/room_agent.forge` catches this:
```forge
pure illegal_example
  needs x: Text
  gives Text
  do
    give reason "this should fail: {x}"   # ← compile error
```

---

### Phase 8 — `states` and `requires`

Implement together — they're tightly coupled.

**Goal:** State machines on agents with precondition guards on handlers.

1. Add `states_decl` and `transition_stmt` to grammar + AST
2. Add `requires_clause` to `handle_decl` in grammar + AST
3. Implement `states_checker.rs`:
   - Build transition graph
   - Verify `transition to X` is legal from handler's `requires lifecycle == Y`
   - Verify terminal states are handled
4. Implement `state_machine.rs` runtime struct
5. Wire state machine into `AgentProcess` — check state on every handler dispatch
6. Implement `requires_checker.rs` — basic satisfiability check
7. Implement `requires` evaluation in runtime handler dispatch

**Acceptance:** 
```forge
# This should compile error:
agent BadAgent
  lifecycle: RoomLifecycle
  on done_action
    requires lifecycle == done
    transition to playing   # illegal: done → playing not in graph
```

```forge
# This should give InvalidCell at runtime:
on move(player: Player, cell: Number)
  requires valid_move(memory.board, cell)  on fail: give InvalidCell
  do
    # never reached if cell is occupied
```

---

### Phase 9 — `event` bus

**Goal:** Typed broadcast events, emit and subscribe.

1. Add `event_decl`, `emit_stmt`, `subscribe_stmt`, `forward_stmt` to grammar + AST
2. Implement `event_bus.rs` with async delivery
3. Wire into runtime: agents register subscriptions at startup, receive events on their
   main loop alongside normal messages
4. Implement filter evaluation for `subscribe X where` clauses
5. Test with two agents: emitter and subscriber

**Acceptance:** Two agents. Agent A emits `MoveEvent`. Agent B subscribes to
`MoveEvent where event.room_id == "room-001"`. Start both. Emit two events — one
matching, one not. Agent B receives only the matching one.

---

### Phase 10 — `timer` engine

**Goal:** Named timers that fire handlers independently.

1. Add timer grammar + AST nodes
2. Implement `timer_engine.rs` with tokio-based timers
3. Wire `TimerFired` events into agent main loop
4. Implement `start`, `cancel`, `reset` in runtime
5. Implement `on X.expired` handler dispatch

**Acceptance:** Agent with `timer reconnect_window: 5s`. Start timer. Let it expire.
Verify handler fires. Start timer again. Cancel it before it fires. Verify handler
does NOT fire.

---

### Phase 11 — `boundary`

**Goal:** Server/client code split enforced at compile time.

1. Add `boundary_decl` and `endpoint_decl` to grammar + AST
2. Implement `boundary_checker.rs`:
   - Build server/client symbol tables
   - Verify no cross-contamination
   - Verify shared types are serializable
3. Add `--boundary` flag to `forge build` CLI command
4. In the output, only include declarations from the requested boundary

For the POC, full wire protocol generation is a stretch goal. The essential thing
is the compiler rejecting illegal cross-boundary references.

**Acceptance:**
```forge
boundary server
  agent SecretAgent
    memory
      api_key: Text   # should never reach client

boundary client
  task bad_task
    do
      give SecretAgent.memory.api_key  # ← compile error
```

---

## Full tic-tac-toe example using all v3 primitives

This is the acceptance test for the complete v3 implementation.

### `tictactoe/room_agent.forge`

```forge
# Uses: pure, states, requires, timer, event

boundary server

states RoomLifecycle
  waiting  -> playing   when players.count == 2
  playing  -> done      when winner_found
  playing  -> done      when board_full
  playing  -> abandoned when all_disconnected

pure check_winner
  needs board: Text[9]
  gives WinResult
  do
    WIN = [[0,1,2],[3,4,5],[6,7,8],[0,3,6],[1,4,7],[2,5,8],[0,4,8],[2,4,6]]
    for line in WIN
      sym = board[line[0]]
      if sym != "" and sym == board[line[1]] and sym == board[line[2]]
        give Winner(sym)
    if board.none(empty)
      give Draw
    give Ongoing

pure valid_move
  needs board: Text[9], cell: Number
  gives Bool
  do
    give cell >= 0 and cell <= 8 and board[cell] == ""

event MoveEvent
  room_id:   Text
  player:    Player
  cell:      Number
  symbol:    Text
  board:     Text[9]

event GameEndEvent
  room_id: Text
  result:  WinResult

agent RoomAgent
  lifecycle: RoomLifecycle

  memory
    id:      Text
    board:   Text[9]
    players: Player[]
    turn:    Number

  timer reconnect_window: 30s
  timer turn_limit: 15s

  on join(player: Player)
    requires lifecycle == waiting    on fail: give RoomFull

    memory.players = memory.players + player
    if memory.players.count == 2
      transition to playing
      reset turn_limit
      emit GameStartEvent(memory.id, memory.players)

  on move(player: Player, cell: Number)
    requires lifecycle == playing              on fail: silent
    requires memory.players[memory.turn] == player  on fail: give OutOfTurn
    requires valid_move(memory.board, cell)    on fail: give InvalidCell

    memory.board[cell] = player.symbol
    reset turn_limit
    emit MoveEvent(memory.id, player, cell, player.symbol, memory.board)
    result = check_winner(memory.board)
    when result == Winner(_) or result == Draw
      transition to done
      emit GameEndEvent(memory.id, result)

  on disconnect(player: Player)
    requires lifecycle == playing    on fail: silent
    start reconnect_window for player

  on reconnect(player: Player)
    cancel reconnect_window for player

  on reconnect_window.expired(player: Player)
    requires lifecycle == playing    on fail: silent
    forfeit(player)
    transition to done
    emit GameEndEvent(memory.id, Forfeit(player))

  on turn_limit.expired
    requires lifecycle == playing    on fail: silent
    idle = memory.players[memory.turn]
    forfeit(idle)
    transition to done
    emit GameEndEvent(memory.id, Forfeit(idle))
```

### `tictactoe/platform.forge`

```forge
boundary shared
  type MoveRequest
    room_id: Text
    cell:    Number
    token:   Text

  type GameState
    board:    Text[9]
    turn:     Number
    status:   Text

boundary server
  pool room_pool
    workers:         RoomAgent * 1000
    strategy:        least_loaded
    spawn_on_demand: true

  agent leaderboard = LeaderboardAgent()

  subscribe GameEndEvent
  on GameEndEvent(e)
    leaderboard.send(update, e)

  endpoint move(req: MoveRequest, ctx: AuthContext) -> GameState or MoveError
    room = room_pool.get(req.room_id)
    room.send(move, ctx.player, req.cell)

boundary client
  task render_board
    needs state: GameState
    gives HTML
    do
      ...

  on server_event(e: GameState)
    render_board(e)
```

---

## What to build first (revised priority)

```
Week 1: Parse + execute pure functions
         → check_winner works as a pure call
Week 2: states + requires
         → RoomAgent lifecycle enforced at compile time
Week 3: event bus
         → LeaderboardAgent receives GameEndEvent
Week 4: timer engine
         → reconnect_window fires forfeit after 30s
Week 5: boundary (compile check only, no wire gen)
         → cross-boundary reference = compile error
Week 6: full tic-tac-toe example runs end-to-end
```

---

## The single most important invariant (updated)

From v2: every primitive composes with `>>`.

Added in v3: **`pure` is the foundation of correctness.**

Every rule in a FORGE system that must be deterministic, fast, and trustworthy
should be `pure`. The game rules. The validation logic. The state transition guards.
If it can be `pure`, it should be `pure`. The compiler guarantee that a `pure`
function cannot call an LLM is the guarantee that your game logic cannot hallucinate.

That guarantee is what makes the whole system trustworthy — not just fast.

---

*FORGE Language POC Plan v3 · Includes: pure, event, states, timer, boundary, requires*
*Supersedes v2 · Ready for Claude Code*
