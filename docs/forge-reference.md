# FORGE Language Reference

This document is a complete reference for the FORGE programming language, designed for teaching LLM agents how to write correct FORGE code.

FORGE is a language for oracle-augmented computation where LLM calls are first-class primitives, uncertainty is tracked structurally, and parallelism is automatic.

## 1. Syntax Basics

# FORGE Language Reference: Syntax Basics

## 1. Indentation

FORGE uses fixed 2-space indentation levels to define code blocks. There are no braces or explicit block delimiters—indentation alone determines scope.

- **i1 (Level 1)**: 2 spaces
- **i2 (Level 2)**: 4 spaces
- **i3 (Level 3)**: 6 spaces
- **i4 (Level 4)**: 8 spaces (maximum nesting depth)

```forge
task process_order
  do
    say "Starting order processing"
    when order_valid
      say "Order is valid"
      for item in order_items
        say "Processing item"
```

## 2. Comments

Line comments are denoted with the `#` symbol. Everything after `#` on that line is ignored.

```forge
# This is a comment
task calculate_total
  do
    # Calculate the sum of all items
    say "Computing total"
```

## 3. Keywords

FORGE has reserved keywords that define control flow, task definitions, and logic. These cannot be used as identifiers.

Reserved keywords: `task`, `pure`, `flow`, `stage`, `fn`, `needs`, `gives`, `do`, `is`, `give`, `say`, `use`, `when`, `else`, `if`, `match`, `for`, `in`, `try`, `or`, `with`, `above`, `reason`, `classify`, `into`, `not`, `and`, `true`, `false`, `agent`, `pool`, `warden`, `contract`, `system`, `event`, `states`, `type`, `endpoint`, `timer`, `emit`, `transition`, `escalate`, `downgrade`, `forward`, `subscribe`, `start`, `cancel`, `reset`, `requires`, `to`, `recall`, `learn`, `spawn`, `find`, `retire`, `exec`, `exportable`, `import`, `from`, `as`, `command`, `background`, `session`, `schedule`

Additional contextual keywords (reserved in specific contexts): `search`, `memory`, `lifecycle`, `manages`, `can`, `workers`, `strategy`, `on`, `fail`, `where`, `stuck`, `crash`, `hallucination`, `budget`, `timeout`, `nudge`, `restart`, `replace`, `self`, `downstream`, `all`, `after`, `max_retries`, `per`, `then`

```forge
task validate_user
  needs user_id
  gives Bool
  do
    when user_exists
      give true
    else
      give false
```

## 4. Identifiers

Identifiers must start with a lowercase letter and can contain alphanumeric characters and underscores. Type names must start with an uppercase letter.

```forge
fn calculate_user_score user_id account_age
  do
    say "User score calculation"

# Valid identifiers: user_id, account_age, score_123, _internal
# Invalid: User_Id (starts uppercase), 123user (starts with number)
```

## 5. Template Strings

String literals use double quotes. Interpolation is performed using curly braces around variable names within the string.

```forge
task greet_user
  needs user_name
  do
    say "Welcome, {user_name}!"
    say "The answer is {answer_value}"
```

## 6. Literals

FORGE supports numeric, boolean, and array literals.

```forge
task demonstrate_literals
  do
    # Numbers
    say 42
    say 3.14
    
    # Booleans
    say true
    say false
    
    # Arrays
    say [1, 2, 3]
    say ["apple", "banana", "cherry"]
```

## 7. Type Names

Type names are capitalized and represent data types used in function signatures and declarations.

Common types: `Text`, `Number`, `Bool`, `Results`, `Report`, `Intent`, `Summary`, `Classification`

```forge
fn process_data input_value
  needs value Text
  gives Number
  do
    say "Processing: {value}"
```

## 8. Array Types

Arrays can be fixed-size (with a number in brackets) or dynamic (with empty brackets).

```forge
fn process_scores
  needs fixed_scores Text[9]
  needs dynamic_scores Number[]
  do
    say "Fixed array has 9 elements"
    say "Dynamic array is variable length"
```

## 9. No Semicolons, Braces, or Condition Parentheses

FORGE eliminates semicolons, braces, and parentheses around conditions. Statements are separated by newlines.

```forge
task check_status
  do
    when status_active
      say "Active"
    else
      say "Inactive"
    
    if value > 10
      say "Greater than 10"
```

## 10. Statement Separation

Statements at the same indentation level are separated by newlines. Each line represents one logical statement.

```forge
task multi_step
  do
    say "Step one"
    say "Step two"
    say "Step three"
```

## 11. No Blank Lines Within Blocks

Blank lines are not permitted within `fn` or `do` blocks. All statements must be consecutive without empty lines.

```forge
fn valid_function
  do
    say "First statement"
    say "Second statement"
    say "Third statement"

# Incorrect - blank line not allowed:
# fn invalid_function
#   do
#     say "First"
#
#     say "Second"
```

---

## 2. Tasks and Pure Functions

# Task and Pure Declarations

## Overview

FORGE supports two types of function declarations: **tasks** and **pure functions**. Tasks can invoke LLMs through oracle queries, while pure functions are deterministic and forbid all LLM operations.

## Task Declaration

A task is a function that can call LLMs. Tasks are the primary mechanism for incorporating oracle reasoning into your FORGE programs.

### Syntax

```
task <name>
  needs <param>: <Type>, <param>: <Type>
  gives <Type>
  do
    <statements at 4-space indent>
```

### Components

| Component | Description |
|-----------|-------------|
| `task <name>` | Declares a task with the given identifier |
| `needs` | Parameter list with type annotations (comma-separated) |
| `gives` | Return type declaration |
| `do` | Body block containing statements at 4-space indent |

### Allowed Operations

Tasks may use:
- `reason` — invoke LLM reasoning with a prompt
- `classify` — invoke LLM classification
- `search` — invoke information retrieval
- `task_name(args)` — invoke other tasks or pure functions
- `give` — return a value
- `say` — print to stdout
- Control flow: `if/else`, `when/else`, `match`, `for`

### Examples

**Basic task with single parameter:**

```
task greet
  needs name: Text
  gives Text
  do
    give the template string "Hello, {name}!"
```

**Task with multiple parameters:**

```
task compare_documents
  needs doc1: Text, doc2: Text
  gives Text
  do
    result = reason "Compare these two documents and identify key differences: {doc1} and {doc2}"
    give result
```

**Task calling another task:**

```forge
task process_user_input
  needs user_text: Text
  gives Text
  do
    cleaned = normalize_text(user_text)
    analyzed = reason "Analyze this text for sentiment: {cleaned}"
    give analyzed
```

**Task with conditional logic:**

```
task categorize_feedback
  needs feedback: Text
  gives Text
  do
    if feedback == ""
      give "empty"
    else
      category = classify feedback into ["positive", "negative", "neutral"]
      give category
```

---

## Pure Function Declaration

A pure function is deterministic and cannot invoke LLMs. The compiler enforces this restriction, rejecting any LLM operations within pure function bodies.

### Syntax

```
pure <name>
  needs <param>: <Type>
  gives <Type>
  do
    <statements at 4-space indent>
```

### Components

| Component | Description |
|-----------|-------------|
| `pure <name>` | Declares a pure function with the given identifier |
| `needs` | Parameter list with type annotations |
| `gives` | Return type declaration |
| `do` | Body block containing statements at 4-space indent |

### Allowed Operations

Pure functions may use:
- Arithmetic and logical operations
- String manipulation
- `give` — return a value
- `say` — print to stdout
- Control flow: `if/else`, `match`, `for`

### Forbidden Operations

The following cause **compiler errors** in pure function bodies:
- `reason`
- `classify`
- `search`
- `escalate`
- `try ... or` (error recovery implies non-determinism)

### Confidence

Pure functions always return confidence **1.0** (deterministic). No confidence annotation is required.

### Examples

**Simple pure function:**

```
pure next_player
  needs current: Text
  gives Text
  do
    if current == "X"
      give "O"
    else
      give "X"
```

**Pure function with string operations:**

```
pure format_name
  needs first: Text, last: Text
  gives Text
  do
    give the template string "{last}, {first}"
```

**Pure function with arithmetic:**

```
pure calculate_discount
  needs price: Number, discount_percent: Number
  gives Number
  do
    discount_amount = price * (discount_percent / 100)
    give price - discount_amount
```

**Pure function with control flow:**

```
pure validate_email
  needs email: Text
  gives Boolean
  do
    if email contains "@" and email contains "."
      give true
    else
      say "Invalid email format"
      give false
```

---

## Return Statements

Both tasks and pure functions use `give` to return values:

```
give <expression>
```

The `give` statement:
- Can appear anywhere in the function body
- Works inside `if/else`, `match`, and `for` blocks
- Immediately returns the specified value
- Type must match the declared `gives` type

### Example with Multiple Returns

```
task classify_number
  needs value: Number
  gives Text
  do
    if value < 0
      give "negative"
    else if value == 0
      give "zero"
    else
      give "positive"
```

---

## Compiler Enforcement

The FORGE compiler enforces strict separation between tasks and pure functions:

| Operation | Task | Pure | Error |
|-----------|------|------|-------|
| `reason` | ✓ | ✗ | Compiler error in pure function |
| `classify` | ✓ | ✗ | Compiler error in pure function |
| `search` | ✓ | ✗ | Compiler error in pure function |
| `escalate` | ✓ | ✗ | Compiler error in pure function |
| `try ... or` | ✓ | ✗ | Compiler error in pure function |
| Call functions | ✓ | ✓ | Allowed (pure can call other pure functions) |
| Arithmetic | ✓ | ✓ | Allowed |
| `give` | ✓ | ✓ | Allowed |

---

## Best Practices

1. **Use pure functions for deterministic logic** — Keep data transformations, formatting, and validation in pure functions
2. **Use tasks for LLM operations** — Reserve tasks for reasoning, classification, and search
3. **Compose tasks and pure functions** — Tasks can call pure functions; pure functions cannot call tasks
4. **Document confidence** — While pure functions always return 1.0, document task confidence expectations in comments

---

## Entry Point: `fn main`

Every FORGE program needs a `fn main` declaration as its entry point. Unlike tasks and pure functions, `fn main` does not use a `do` block — its body is indented at i1 (2 spaces) directly.

### Syntax

```
fn main
  <statements at 2-space indent>
```

### Example

```forge
fn main
  topic = "AI should be open source"
  arguments = (argue_for(topic) | argue_against(topic))
  say "FOR: {arguments[0]}"
  say "AGAINST: {arguments[1]}"
  verdict = judge(arguments[0], arguments[1])
  say verdict
```

Note: `fn main` has no `needs`, `gives`, or `do` clauses.

---

## 3. Flows and Parallel Stages

# Flow Declarations and Parallel Stages

## Overview

A **flow** is a multi-stage pipeline that processes data through sequential or parallel stages. The FORGE compiler automatically analyzes stage dependencies and determines which stages can execute concurrently, organizing them into execution waves via topological sort.

## Flow Syntax

```
flow <name>
  needs <param>: <Type>
  gives <Type>

  stage <name>
    <statements at 4-space indent>

  stage <name>
    needs <prev_stage>.<field>, <other_stage>.<field>
    <statements at 4-space indent>
```

### Components

| Element | Purpose |
|---------|---------|
| `flow <name>` | Declares a new flow pipeline |
| `needs <param>: <Type>` | Declares input parameters required by the flow |
| `gives <Type>` | Declares the return type of the flow |
| `stage <name>` | Declares a processing stage within the flow |
| `needs <dependency>` | Declares which stages or parameters this stage depends on |

## Execution Waves and Parallelization

The compiler builds a directed acyclic graph (DAG) of stage dependencies and computes execution waves through topological sort.

### Wave Assignment Rules

- **Wave 1**: Stages without `needs` clauses, or that only depend on flow parameters
- **Wave N**: Stages whose dependencies are satisfied after Wave N-1 completes
- All stages in the same wave execute concurrently as independent tokio tasks

### Dependency Declaration

Access stage outputs using dot notation:

```
needs <stage_name>.<field>
```

To depend on all variables from a stage:

```
needs <stage_name>.*
```

## Stage Variables and Scope

Variables defined in a stage are automatically exposed to dependent stages as records:

```
<stage_name>.<variable_name>
```

A stage can depend on multiple other stages and access any of their exposed variables.

## Return Value

The flow returns the value from the last `give` statement encountered during execution. Only one `give` statement should exist per flow (typically in the final stage).

## Example: Code Review Pipeline

```
flow review_code
  needs code: Text
  gives Text

  stage detect
    lang = detect_language(code)

  stage quality
    needs detect.lang
    report = analyze_quality(code, detect.lang)

  stage security
    needs detect.lang
    report = analyze_security(code, detect.lang)

  stage verdict
    needs detect.lang, quality.report, security.report
    give format_verdict(detect.lang, quality.report, security.report)
```

### Execution Plan

| Wave | Stages | Notes |
|------|--------|-------|
| Wave 1 | `detect` | No dependencies; runs first |
| Wave 2 | `quality`, `security` | Both depend only on `detect`; run concurrently |
| Wave 3 | `verdict` | Depends on all previous stages; runs last |

## Best Practices

- **Declare all dependencies explicitly**: The compiler cannot infer unstated dependencies
- **Minimize cross-stage coupling**: Depend only on required fields to maximize parallelization
- **Order stages logically**: Place independent work early to enable Wave 1 execution
- **Use `.*` sparingly**: Explicit field dependencies are more maintainable and enable better optimization
- **Single return point**: Place the `give` statement in the final stage for clarity

---

## 4. Control Flow

# CONTROL FLOW

FORGE provides four control flow constructs, each designed for specific branching and iteration scenarios. All constructs support the `give` statement, which propagates results up to the enclosing function boundary.

## WHEN/ELSE — Confidence-Based Dispatch

Use `WHEN/ELSE` to branch based on the confidence level of LLM-generated values. This construct is essential for handling uncertainty in language model outputs.

### Syntax

```forge
when <var>.sure -> <statement>
when <var>.unsure -> <statement>
when <var>.unreliable -> <statement>
else -> <statement>
```

### Confidence Thresholds

- **sure**: confidence ≥ 0.8
- **unsure**: confidence 0.5–0.8
- **unreliable**: confidence < 0.5
- **conflicted**: conflicting predictions from the oracle

### Behavior

- Clauses are **mutually exclusive**; the first matching clause executes
- The `else` clause is optional and executes if no `when` clause matches
- Each branch contains a single statement (use indentation for compound statements)

### Custom Thresholds

Override default thresholds using the `above:` parameter:

```forge
when result.sure(above: 0.9) -> give result
```

### Example

```forge
sentiment = classify text into ["positive", "negative", "neutral"]

when sentiment.sure -> give sentiment
when sentiment.unsure -> give "needs review"
when sentiment.unreliable -> give "unknown"
else -> escalate to human
```

Each `when`/`else` clause takes a single inline statement after the `->` arrow. Multiple `when` clauses appear at the same indentation level, one per line.

---

## IF/ELSE IF/ELSE — Boolean Branching

Use `IF/ELSE IF/ELSE` for traditional conditional branching based on boolean expressions.

### Syntax

```forge
if <condition>
  <statements at next indent>
else if <condition>
  <statements>
else
  <statements>
```

### Operators

Boolean conditions support: `==`, `!=`, `<`, `>`, `<=`, `>=`, `and`, `or`, `not`

### Example

```forge
if user.age >= 18 and user.verified == true
  grant_access(user)
else if user.age >= 13
  grant_limited_access(user)
else
  deny_access(user)
```

---

## MATCH — Structural Pattern Matching

Use `MATCH` to branch based on the structure or type of a value. This is particularly useful for discriminated unions and constructor-based types.

### Syntax

```forge
match <expr>
  <Pattern> -> <statement>
  <Pattern> -> <statement>
  _ -> <statement>
```

### Patterns

- **Wildcard** (`_`): Matches any value; typically used as a catch-all
- **Constructor names** (`Active`, `Inactive`): Match specific type constructors
- **Binding variables** (`x`, `result`): Bind matched values to variables for use in the statement

### Restrictions

- String literals **cannot** be used as patterns; use `IF/ELSE` for string comparison instead

### Example

```forge
match account.status
  Active -> process_payment(account)
  Inactive -> send_reactivation_email(account)
  Suspended -> notify_admin(account)
  _ -> log_unknown_status(account)
```

### Pattern with Binding

```forge
match parsed_response
  Success(data) -> give data
  Error(msg) -> log_error(msg)
  _ -> give null
```

---

## FOR — Iteration Over Collections

Use `FOR` to iterate over arrays or lists.

### Syntax

```forge
for <binding> in <expr>
  <statements at next indent>
```

### Behavior

- Iterates over `Array` or `List` values
- The binding variable is scoped to the loop body
- Supports the `give` statement to return early from the enclosing function

### Example

```forge
results = []
for item in items
  processed = transform(item)
  when processed.sure ->
    results.append(processed)

give results
```

### Early Exit with `give`

```forge
for user in users
  if user.is_admin == true
    give user
```

---

## `give` in Control Flow

The `give` statement propagates results up to the enclosing function boundary and works inside all control flow blocks:

```forge
for item in batch
  if item.error != null
    give error(item.error)
  process(item)
```

---

## 5. Expressions and Operators

# EXPRESSIONS AND OPERATORS

This section describes all expression types in FORGE, ordered by precedence from lowest to highest.

## 1. COMPOSE (Pipe)

**Syntax:** `expr >> expr >> expr`

Chains expressions left-to-right, passing the result of each expression as input to the next. The previous result is bound to the `_pipe` variable.

**Use case:** Create data transformation pipelines and multi-step workflows.

**Examples:**
```
argue_for(topic) >> score_argument(_pipe)

fetch_data(source) >> filter(_pipe, status: "active") >> count(_pipe)

user_input >> reason "Summarize this in one sentence: {_pipe}" >> classify _pipe into ["positive", "negative", "neutral"]
```

## 2. BOOLEAN

**Syntax:** `expr and expr` | `expr or expr`

Logical AND and OR operations. Short-circuit evaluation applies.

**Examples:**
```
is_valid(input) and has_permission(user)

user.age >= 18 or user.has_parental_consent
```

## 3. COMPARISON

**Syntax:** `==` | `!=` | `<` | `>` | `<=` | `>=`

Compare Numbers and Text values. Text equality is an exact match.

**Examples:**
```
score > 0.8

response == "approved"

user.age >= 21

timestamp != null
```

## 4. ARITHMETIC

**Syntax:** `+` | `-` | `*` | `/`

Numeric operations. The `+` operator also concatenates Text values.

**Examples:**
```
total_score = argument_score + evidence_score

confidence * 100

user.first_name + " " + user.last_name

remaining_tokens = budget - used
```

## 5. UNARY

**Syntax:** `not expr` | `-expr`

Logical negation and numeric negation.

**Examples:**
```
not is_spam(message)

-temperature
```

## 6. FAN-OUT (Parallel)

**Syntax:** `(expr1 | expr2 | expr3)`

Evaluates all expressions in parallel and returns an Array containing all results in order.

**Use case:** Run multiple independent operations simultaneously and collect results.

**Examples:**
```
(argue_for(topic) | argue_against(topic))

(search "climate change impacts" | search "climate change solutions" | search "climate change policy")

(validate_email(user.email) | validate_phone(user.phone) | validate_address(user.address))
```

## 7. TRY-OR (Error Recovery)

**Syntax:** `try <expr> or <fallback>`

Attempts to evaluate the primary expression. If any error occurs, evaluates the fallback expression instead.

**Use case:** Graceful error handling and fallback logic.

**Examples:**
```
try fetch_from_api(url) or "API unavailable"

try parse_json(raw_data) or {}

try external_service(request) or use_cached_result(request_id)
```

## 8. LLM OPERATIONS

### reason

**Syntax:** `reason <template_string>`

Sends a prompt to the LLM and returns a Text response with confidence metadata.

**Examples:**
```
reason "Analyze the sentiment of: {input}"

reason "Is this a valid email address? {email}"

reason "Summarize the key points from: {document}"
```

### classify

**Syntax:** `classify <expr> into [label1, label2, label3]`

Classifies the input expression into one of the provided string labels.

**Examples:**
```
classify user_feedback into ["bug", "feature_request", "documentation"]

classify sentiment_score into ["positive", "neutral", "negative"]

classify document into ["technical", "marketing", "legal", "other"]
```

### search

**Syntax:** `search <template_string>`

Performs a web search using the template string as the query. *(Currently stubbed; full implementation pending)*

**Examples:**
```
search "latest developments in {topic}"

search "{company_name} financial results {year}"
```

## 9. FUNCTION CALLS

**Syntax:** `task_name(arg1, arg2)` | `task_name(label: arg1, label: arg2)`

Invokes a task or function with positional or labeled arguments.

**Examples:**
```
score_argument(argument_text)

evaluate(input: user_response, rubric: evaluation_criteria)

fetch_data(source: "database", filter: active_only)

calculate_score(evidence, confidence, weight: 0.8)
```

## 10. POSTFIX

Field access, array indexing, and method calls.

**Syntax:** `.field` | `[n]` | `.method()`

- **Field access:** Access properties of objects
- **Array indexing:** Access elements by zero-based index
- **Method calls:** Invoke methods on values

**Examples:**
```
user.email

results[0]

text.uppercase()

arguments[2].score

response.data.items[0].name
```

## 11. ATOMS

Fundamental expressions: literals, variables, array literals, and parenthesized expressions.

**Syntax:** 
- Numbers: `42`, `3.14`, `-5`
- Text: `"hello"`, `'world'`
- Booleans: `true`, `false`
- Variables: `variable_name`, `_pipe`
- Arrays: `[expr1, expr2, expr3]`
- Parenthesized: `(expr)`

**Examples:**
```
42

"approval required"

true

user_input

_pipe

[score1, score2, score3]

(argument >> evaluate(_pipe))
```

---

## Precedence Summary

When combining operators, evaluation follows this precedence (lowest to highest):

1. COMPOSE (`>>`)
2. BOOLEAN (`and`, `or`)
3. COMPARISON (`==`, `!=`, `<`, `>`, `<=`, `>=`)
4. ARITHMETIC (`+`, `-`, `*`, `/`)
5. UNARY (`not`, `-`)
6. FAN-OUT (`|`)
7. TRY-OR (`try...or`)
8. LLM OPERATIONS (`reason`, `classify`, `search`)
9. FUNCTION CALLS
10. POSTFIX (`.`, `[]`, `()`)
11. ATOMS

Use parentheses to override precedence when needed.

---

## 6. Methods and Configuration

# FORGE Reference Documentation

## PART 1 - Built-In Methods

### Overview

FORGE provides a set of built-in methods for operating on values. Methods are called using dot notation and can be chained together for complex operations.

### String Methods

#### `.lower()`
Converts text to lowercase.

- **Returns:** Text
- **Preserves:** Confidence metadata
- **Example:**
  ```
  "HELLO".lower()  // "hello"
  ```

#### `.upper()`
Converts text to uppercase.

- **Returns:** Text
- **Preserves:** Confidence metadata
- **Example:**
  ```
  "hello".upper()  // "HELLO"
  ```

#### `.trim()`
Strips leading and trailing whitespace from text.

- **Returns:** Text
- **Preserves:** Confidence metadata
- **Example:**
  ```
  "  hello world  ".trim()  // "hello world"
  ```

### Array and Collection Methods

#### `.len()` / `.count()`
Returns the length or element count.

- **Returns:** Number
- **Works on:** Array, List, Text (character count)
- **Example:**
  ```
  [1, 2, 3].len()       // 3
  "hello".len()         // 5
  ```

#### `.contains(x)` / `.any(x)`
Checks if a value contains an element or substring.

- **Returns:** Bool
- **Works on:** 
  - Array (element match)
  - Text (substring match)
- **Example:**
  ```
  [1, 2, 3].contains(2)           // true
  "hello world".contains("world") // true
  ```

#### `.none(x)`
Returns true if an array does **NOT** contain the specified element.

- **Returns:** Bool
- **Works on:** Array
- **Example:**
  ```
  [1, 2, 3].none(4)  // true
  [1, 2, 3].none(2)  // false
  ```

### Method Chaining

Methods can be chained together for concise, readable operations:

```
result.lower().contains(search_term)
user_input.trim().upper().len()
```

### Syntax Notes

- **Zero-argument methods require parentheses:** `arr.len()` not `arr.len`
- Methods preserve metadata (such as confidence scores) where applicable

---

## PART 2 - Configuration and Providers

### Overview

FORGE programs are provider-agnostic—the LLM provider is specified through configuration rather than in code. This enables flexible deployment and testing without code changes.

### Configuration File (TOML Format)

Configuration is stored in TOML format. The default location is `forge.toml` in the working directory.

#### Basic Structure

```toml
[llm]
default = provider-name

[providers.name]
type = anthropic | openai-compat | mock
model = model-name
api_key = ANTHROPIC_API_KEY
base_url = https://api.example.com  # Required for openai-compat
fallback = another-provider-name
timeout_secs = 120
```

#### Configuration Fields

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `[llm] default` | Yes | String | Name of the default provider |
| `type` | Yes | String | Provider type: `anthropic`, `openai-compat`, or `mock` |
| `model` | Yes | String | Model identifier (e.g., `claude-3-sonnet`) |
| `api_key` | Conditional | String | API key; supports `$VARIABLE_NAME` syntax for environment variables |
| `base_url` | Conditional | String | API endpoint URL; **required for `openai-compat`** |
| `fallback` | No | String | Fallback provider if primary fails |
| `timeout_secs` | No | Number | Request timeout in seconds (default: 120) |

#### Example Configuration

```toml
[llm]
default = anthropic-prod

[providers.anthropic-prod]
type = anthropic
model = claude-3-sonnet-20240229
api_key = ${ANTHROPIC_API_KEY}
timeout_secs = 120

[providers.openai-local]
type = openai-compat
model = gpt-4
api_key = ${OPENAI_API_KEY}
base_url = http://localhost:8000/v1
fallback = anthropic-prod

[providers.test]
type = mock
model = mock-model
```

### Environment Variable Overrides

Environment variables override configuration file settings:

| Variable | Purpose |
|----------|---------|
| `FORGE_CONFIG=path` | Specify a custom configuration file path |
| `FORGE_MOCK=1` | Enable mock provider (deterministic, for testing) |
| `FORGE_PROVIDER=name` | Override the default provider at runtime |

**Example:**
```bash
FORGE_PROVIDER=openai-local forge run script.forge
FORGE_CONFIG=/etc/forge/prod.toml forge run script.forge
FORGE_MOCK=1 forge test suite.forge
```

### CLI Commands

#### `forge parse <file>`
Parse a FORGE program and print its abstract syntax tree (AST).

```bash
forge parse program.forge
```

#### `forge check <file>`
Type-check a FORGE program without execution.

```bash
forge check program.forge
```

#### `forge run <file>`
Execute a FORGE program using the configured provider.

```bash
forge run program.forge
```

#### `forge trace <file>`
Execute a FORGE program and emit a detailed JSON trace to stderr for debugging.

```bash
forge trace program.forge 2> trace.json
```

---

## 7. Events

Events are named, typed messages that agents emit and subscribe to. They form the backbone of inter-agent communication in FORGE.

### Syntax

```
event <Name>
  <field>: <Type>
  <field>: <Type>
```

### Components

| Component | Description |
|-----------|-------------|
| `event <Name>` | Declares an event with an uppercase identifier |
| `<field>: <Type>` | One or more typed fields at 2-space indent (i1) |

Event names must start with an uppercase letter. Each field occupies its own line at i1 indentation, with a name, colon, and type annotation.

### Emitting Events

Events are emitted from agent handlers using the `emit` statement with named arguments:

```
emit <EventName>(<field>: <expr>, <field>: <expr>)
```

### Subscribing to Events

Agents receive events via the `subscribe` clause, with an optional `where` filter:

```
subscribe <EventName>
subscribe <EventName> where <expr>
```

### Examples

**Declaring events:**

```forge
event CustomerMessage
  customer: Text
  content: Text

event Resolved
  customer: Text
  summary: Text
```

**Emitting an event from a handler:**

```forge
emit Resolved(customer: customer, summary: memory.topic)
```

**Subscribing with a filter:**

```forge
subscribe CustomerMessage where customer == memory.customer
```

**Events with numeric fields:**

```forge
event TopicCompleted
  topic: Text
  score: Number

event SessionSummary
  total_questions: Number
  correct: Number
  level: Text
```

### Best Practices

1. **Name events as nouns or past-tense verbs** -- `CustomerMessage`, `Resolved`, `TopicCompleted`
2. **Keep event payloads minimal** -- include only the data subscribers need
3. **Use `where` filters to scope subscriptions** -- avoid processing irrelevant events

---

## 8. States

States declare finite state machines that govern agent lifecycles. Each `states` block defines a set of named states and the legal transitions between them, with optional guard conditions.

### Syntax

```
states <Name>
  <from> -> <to>
  <from> -> <to> when <expr>
```

### Components

| Component | Description |
|-----------|-------------|
| `states <Name>` | Declares a state machine with an uppercase identifier |
| `<from> -> <to>` | Declares a legal transition between two states |
| `when <expr>` | Optional guard condition for the transition |

Each transition occupies its own line at i1 indentation. A state can have multiple outgoing transitions, and multiple transitions can target the same state (including self-transitions).

### Binding to an Agent

An agent binds to a state machine via the `lifecycle` clause:

```forge
agent my_agent
  lifecycle: MyStates
```

The agent then uses `transition to <state>` to move between states. The compiler validates that all transitions are legal with respect to the declared state machine.

### Compiler Enforcement

The states checker produces **errors** for:

| Error | Description |
|-------|-------------|
| Unknown lifecycle | Agent references a `states` block that does not exist |
| Unknown state in guard | A `requires lifecycle == X` references a state not in the block |
| Unknown state in transition | A `transition to X` targets a state not in the block |
| Illegal transition | A `transition to X` from a guarded state has no matching edge in the block |
| Unguarded transition | A handler contains `transition to X` without a `requires lifecycle == ...` guard |
| Conflicting guards | A handler has more than one lifecycle guard |

The states checker produces **warnings** for:

| Warning | Description |
|---------|-------------|
| Terminal state | A state has no outgoing transitions (may be intentional) |
| Unreachable state | A state has no incoming transitions and is not an initial state |
| Opaque guard | A lifecycle guard is too complex for static analysis (e.g., `lifecycle != X`) |

### Examples

**Basic state machine:**

```forge
states SupportPhase
  greeting -> active when message_count > 0
  active -> resolved
  active -> escalated
```

**Game lifecycle:**

```forge
states GamePhase
  waiting -> playing when player_count == 2
  playing -> finished
```

**Multi-level progression with self-transition:**

```forge
states TutorPhase
  beginner -> intermediate when score >= 3
  intermediate -> advanced when score >= 7
  advanced -> advanced
```

### Best Practices

1. **Declare all states explicitly** -- every state name must appear in at least one transition
2. **Guard transitions that depend on conditions** -- use `when` clauses to document the invariant
3. **Use `requires lifecycle == <state>` in handlers** -- this enables the compiler to verify transition legality
4. **Terminal states are intentional** -- if a state has no outgoing edges, the compiler warns; this is correct for final states like `resolved` or `finished`

---

## 9. Agents

Agents are long-lived, event-driven actors with persistent memory, lifecycle state, timers, and supervision. They are the primary construct for modeling autonomous behavior in FORGE.

### Syntax

```
agent <name>
  lifecycle: <StatesName>
  memory
    <field>: <Type>
  timer <name>: <duration>
  subscribe <EventName> where <expr>
  warden_override
    on <failure>: <response>, <scope>

  on <event>(<param>: <Type>)
    requires <expr> on fail: <policy>
    <statements>

  if stuck for <N> turns
    <statements>
```

All clauses are optional except for at least one `on` handler.

### Components

| Component | Description |
|-----------|-------------|
| `agent <name>` | Declares an agent with a lowercase identifier |
| `lifecycle: <Name>` | Binds the agent to a `states` block for lifecycle management |
| `memory` | Declares persistent fields that survive across handler invocations |
| `timer <name>: <duration>` | Declares a named timer with a duration (e.g., `10m`, `30s`, `1h`) |
| `subscribe <Event>` | Subscribes to events from the event bus |
| `warden_override` | Overrides warden policies for this specific agent |
| `on <event>` | Declares a handler that runs when the named event occurs |
| `if stuck` | Declares a recovery policy when the agent is stuck |

### Memory

Memory fields persist across handler invocations within the same agent instance. They are accessed and updated using dot notation:

```forge
memory
  score: Number
  topic: Text
  history: Text[]
```

Reading: `memory.score`, `memory.topic`

Writing: `memory.score = memory.score + 1`

Array indexing: `memory.board[cell] = player`

### Lifecycle and State Tracking

When an agent declares `lifecycle: <StatesName>`, the compiler-verified way to change state is the `transition to <state>` statement, paired with `requires lifecycle == <state>` guards. This combination enables static analysis: the compiler checks that every transition is legal in the declared state machine and that handlers guard which state they operate in.

Memory fields can track additional agent state (e.g., counters, flags, context), but memory writes are **not checked** against the state machine. Using `memory.status` to track phase instead of `transition to` bypasses all compiler guarantees — the states checker cannot verify transition legality, detect illegal transitions, or warn about unreachable states.

**Rule:** If you declare a `lifecycle`, use `transition to` and `requires lifecycle ==` for state changes. Use memory for data that is orthogonal to the lifecycle (scores, names, accumulated context).

### Timers

Timers are declared with a name and a duration. Duration suffixes are `s` (seconds), `m` or `min` (minutes), and `h` (hours).

```forge
timer session_timeout: 10m
timer reconnect_window: 30s
```

Declaring a timer makes it available but **does not start it**. Timers are inert until a handler explicitly calls `start <timer>`. An un-started timer never fires. This is intentional — the agent controls exactly when the countdown begins.

```forge
on open(service: Text)
  memory.service = service
  start ack_deadline        # arms the 30s countdown

on acknowledge(owner: Text)
  cancel ack_deadline       # disarms — the timer will not fire
```

Timer events are handled with the `on <timer_name>.expired` handler:

```forge
on session_timeout.expired
  say "Session timed out"
  escalate to human
```

Timers can be controlled with statements:

| Statement | Description |
|-----------|-------------|
| `start <timer>` | Arms the timer — begins the countdown from its declared duration |
| `start <timer> for <expr>` | Arms the timer for a specific context (e.g., a player or session) |
| `cancel <timer>` | Stops the timer — it will not fire unless started again |
| `cancel <timer> for <expr>` | Cancels the timer for a specific context |
| `reset <timer>` | Cancels and re-starts the timer from its full declared duration |

### Schedules

Schedules are durable, cross-session wall-clock triggers. Where a `timer` is agent-local, cancellable, and protocol-scoped (a countdown the agent arms inside a flow), a `schedule` is declarative and persistent: it fires on its own cadence, independently of any particular session.

```forge
agent forge_sensei
  memory persistent
    current_level: Text
  schedule mastery_review
    when: daily at "09:00"
    mode: spawn
    prompt: "Reassess specialist mastery from last 24h TaskCompleted signals."
  schedule drift_check
    when: every 6h
    mode: wake
    emit: DriftCheckDue

  on DriftCheckDue
    say "drift check fired"
```

> **Status:** Fully shipped. Grammar, AST, compile-time checker, `WakeService`, and `CronDriver` are all available. Both `mode: spawn` and `mode: wake` are runtime-dispatched.

A schedule block must declare `when:` and `mode:`. Extra options depend on the mode.

#### The `when:` clause — three forms

| Form | Example | Meaning |
|------|---------|---------|
| `daily at "HH:MM"` | `when: daily at "09:00"` | Fire once per day at the given 24-hour clock time. Hour must be 0–23, minute 0–59. |
| `every <duration>` | `when: every 6h` | Fire at a fixed interval. Uses the same duration suffixes as `timer` (`s`, `m`, `min`, `h`, `d`). Duration must be positive. |
| `cron "..."` | `when: cron "0 9 * * *"` | Fire on a standard 5-field Unix cron expression (`m h dom mon dow`). |

FORGE cron is strict 5-field. Seconds-first (6-field) cron strings are a compile error.

#### The `mode:` clause — two modes

| Mode | Requires | Effect |
|------|----------|--------|
| `mode: spawn` | `prompt: "..."` | Starts a **stateless turn**. The prompt text is the agent's only input for that turn — no prior memory state, no conversation history. Use when the work is a self-contained reassessment. |
| `mode: wake` | `emit: EventName` **or** an `on <schedule_name>.tick` handler | **Rehydrates** the agent's `memory persistent` state and publishes an event. Use when the scheduled work must consult or update long-lived memory. |

The pairing requirement for `mode: wake` is compile-time enforced:

- Provide `emit: SomeEvent` and a matching `on SomeEvent` handler in the same agent, **or**
- Provide no `emit:` and a handler named `on <schedule_name>.tick` (default sink).

Either satisfies the compiler; declaring both an `emit:` and a default tick handler is fine.

#### Optional: `precision: high`

By default the dispatcher evaluates schedules at minute granularity. `precision: high` enables per-second firing for this schedule (e.g. `when: every 30s`).

```forge
schedule heartbeat
  when: every 30s
  mode: wake
  emit: Heartbeat
  precision: high
```

#### The `prompt:` template

For `mode: spawn`, `prompt:` uses FORGE's standard template string, so `{expr}` interpolation against agent memory is available at fire time:

```forge
schedule review
  when: daily at "09:00"
  mode: spawn
  prompt: "Review the last 24h, compare against {memory.current_level} targets."
```

#### Compile-time guarantees

The schedule checker catches all of the following at `forge check` time:

- Missing `when:` or `mode:`
- `mode: spawn` without `prompt:`; `mode: wake` without an `emit:` or matching `.tick` handler
- Duplicate schedule names inside one agent
- Duplicate option lines (e.g. two `when:` lines) inside one block
- Malformed cron strings (5-field Unix)
- Time literals outside `00:00`–`23:59`
- `every 0s` / `every 0m` / `every 0h`
- Schedule name colliding with a timer name or handler event name in the same agent

Extraneous options — `emit:` under `mode: spawn`, or `prompt:` under `mode: wake` — produce warnings rather than errors.

#### When to reach for `schedule` vs `timer`

Use `timer` when the countdown is **inside a flow**: "if the user doesn't respond in 30 seconds, escalate." The agent arms, cancels, and resets the timer explicitly; the lifetime is the session.

Use `schedule` when the cadence is **orthogonal to any session**: "every morning at 09:00, reassess mastery," or "every 6 hours, publish a drift check." The dispatcher owns the cadence; the agent only declares it.

### Handlers

Handlers are the core of agent behavior. Each handler responds to a named event and may declare parameters and a return type.

```forge
on <event>(<param>: <Type>, <param>: <Type>): <ReturnType>
  <statements>
```

Handler names support dot notation for scoped events (e.g., `session_timeout.expired`).

#### Requires Guards

Handlers can declare preconditions using `requires` clauses. Each guard specifies an expression that must be true and a fail policy for when it is not:

```forge
requires <expr> on fail: <policy>
```

| Fail Policy | Description |
|-------------|-------------|
| `silent` | Silently drop the event |
| `log` | Log the failure and drop the event |
| `escalate` | Escalate to the agent's supervisor |
| `crash` | Crash the agent |
| `give <expr>` | Return an expression as the handler result |

### Agent Statements

Inside handlers, agents have access to the following statements in addition to standard control flow:

| Statement | Syntax | Description |
|-----------|--------|-------------|
| Emit | `emit Event(field: val)` | Publish an event to the bus |
| Transition | `transition to <state>` | Move to a new lifecycle state |
| Escalate | `escalate to <target>` | Escalate to a named supervisor or entity |
| Forward | `forward <expr> to <expr>` | Forward a message to another agent |
| Memory update | `memory.field = <expr>` | Update a persistent memory field |
| Memory array update | `memory.field[idx] = <expr>` | Update a specific array element |

The `escalate to <target>` target is an **unresolved name** — FORGE does not validate it at compile time. In a supervised context (under a warden), escalation signals are delivered to the warden, which decides how to respond. The target name is metadata the supervisor or runtime can use for routing. Common conventions: `human` (hand off to a human operator), a declared agent name (forward to that agent), or a domain-specific label like `oncall_manager`.

### Stuck Policy

The `if stuck` block defines recovery behavior when the agent cannot make progress. The optional `for N turns` clause specifies how many idle turns trigger the policy:

```forge
if stuck for 3 turns
  say "Escalating after repeated stuck state"
  escalate to human
```

Without a turn count, the stuck policy triggers on any stuck detection:

```forge
if stuck
  escalate to human
```

### Warden Override

An agent can override its warden's default policies using a `warden_override` block:

```forge
agent classifier
  warden_override
    on stuck: replace, self

  on start
    say "ready"
```

This takes precedence over the warden's policy for the specified failure type.

### Examples

**Support bot with lifecycle, memory, timers, and events:**

```forge
agent support_bot
  lifecycle: SupportPhase
  memory
    topic: Text
    message_count: Number
    escalation_count: Number
  timer session_timeout: 10m
  subscribe CustomerMessage where customer == memory.customer

  on message(customer: Text, content: Text)
    memory.message_count = memory.message_count + 1
    reset session_timeout
    intent = classify content into ["question", "complaint", "feedback", "urgent"]
    response = reason "Help this customer with their {intent} about: {content}"
    when response.sure -> say response
    when response.unsure -> say "Let me look into that for you."
    else -> escalate to human

  on resolve(customer: Text)
    cancel session_timeout
    emit Resolved(customer: customer, summary: memory.topic)
    transition to resolved

  on session_timeout.expired
    say "Session timed out"
    escalate to human

  if stuck for 3 turns
    memory.escalation_count = memory.escalation_count + 1
    say "Escalating after repeated stuck state"
    escalate to human
```

**Game room with requires guards and pattern matching:**

```forge
agent room_agent
  lifecycle: GamePhase
  memory
    board: Text[9]
    current_turn: Text
    player_count: Number
  timer reconnect_window: 30s
  subscribe PlayerJoined where room == memory.room

  on join(player: Text)
    requires lifecycle == waiting on fail: give "game already started"
    requires memory.player_count < 2 on fail: give "room full"
    memory.player_count = memory.player_count + 1
    emit PlayerJoined(player: player, room: "main")
    if memory.player_count == 2
      transition to playing

  on disconnect(player: Text)
    start reconnect_window for player

  on reconnect(player: Text)
    cancel reconnect_window for player

  on move(player: Text, cell: Number)
    requires player == memory.current_turn on fail: silent
    requires memory.board[cell] == "_" on fail: give "cell taken"
    memory.board[cell] = player
    result = check_winner(memory.board)
    match result
      Winner(who) -> give GameResult(winner: who, detail: "three in a row")
      Draw -> give GameResult(winner: "none", detail: "draw")
      _ -> say "next turn"
    memory.current_turn = next_player(memory.current_turn)

  on reconnect_window.expired
    escalate to lobby

  if stuck for 5 turns
    say "game appears stuck"
    escalate to lobby
```

### Testing Agents with the REPL

The `forge agent <file>` command launches an interactive REPL for manual handler testing. Type an event name with optional arguments to dispatch it:

```
$ forge agent examples/agents/support_bot.forge
FORGE Agent: support_bot
  memory: topic, message_count, escalation_count
  handlers: message, resolve, session_timeout.expired

Type an event name with optional arguments. Examples:
  message "alice" "I need help with billing"
  resolve "alice"
  quit

> message "alice" "I need help with billing"
→ Let me help you with your billing question...

> quit
bye!
```

The REPL is **dispatch-only**: it processes typed events through handlers but does not run the full agent event loop. This means:

- **Timers** do not fire -- `start <timer>` is accepted but the countdown runs with no listener
- **Event bus subscriptions** are inactive -- `subscribe` and `emit` have no bus to publish to
- **Warden supervision** is not active -- stuck detection and failure policies do not apply

For full runtime behavior including timers, subscriptions, and warden supervision, use `forge run` with a system that orchestrates the agent.

### Best Practices

1. **Always declare a lifecycle** -- state machines make agent behavior predictable and verifiable
2. **Guard handlers with `requires`** -- use lifecycle guards (`requires lifecycle == waiting`) so the compiler can verify transition legality
3. **Keep handlers focused** -- each handler should respond to one event with a clear purpose
4. **Use `give` in fail policies** -- prefer `give <message>` over `silent` to provide callers with feedback
5. **Define stuck policies** -- agents that interact with LLMs should have stuck recovery to prevent infinite loops
6. **Start timers explicitly** -- declare the timer for its name and duration, then call `start` in the handler where the countdown should begin

---

## 10. Type Definitions

Type definitions declare named record types with typed fields. They are used as structured return values, event payloads, and handler results.

### Syntax

```
type <Name>
  <field>: <Type>
  <field>: <Type>
```

### Components

| Component | Description |
|-----------|-------------|
| `type <Name>` | Declares a type with an uppercase identifier |
| `<field>: <Type>` | One or more typed fields at 2-space indent (i1) |

### Constructing Values

Type instances are constructed using the type name with named arguments:

```
<TypeName>(<field>: <expr>, <field>: <expr>)
```

### Matching on Types

Types work with `match` for structural pattern matching. Constructor patterns bind inner values to variables:

```forge
match result
  Winner(who) -> give GameResult(winner: who, detail: "three in a row")
  Draw -> give GameResult(winner: "none", detail: "draw")
  _ -> say "next turn"
```

### Examples

**Declaring and using a type:**

```forge
type GameResult
  winner: Text
  detail: Text
```

Used as a return value:

```forge
give GameResult(winner: who, detail: "three in a row")
```

### Best Practices

1. **Use types for structured returns** -- prefer `give GameResult(winner: who, detail: msg)` over returning raw text
2. **Name types as nouns** -- `GameResult`, `SessionSummary`, `Classification`
3. **Keep types small** -- two to four fields is typical; split larger structures into separate types

---

## 11. Pools

Pools manage groups of identical worker agents and dispatch work using a specified strategy. They provide built-in consensus, redundancy, and timeout handling.

### Syntax

```
pool <name>
  workers: <AgentOrTask> * <count>
  strategy: <strategy>
  timeout: <duration>
  fallback: <handler>
```

### Components

| Component | Required | Description |
|-----------|----------|-------------|
| `pool <name>` | Yes | Declares a pool with a lowercase identifier |
| `workers: <Name> * <N>` | Yes | Specifies the worker type and instance count |
| `strategy: <strategy>` | Yes | Specifies how results are aggregated |
| `timeout: <duration>` | No | Maximum wait time before fallback (e.g., `15s`, `30s`) |
| `fallback: <handler>` | No | Handler to invoke if all workers fail or timeout |

### Strategies

| Strategy | Description |
|----------|-------------|
| `fastest` | Return the first result from any worker |
| `all` | Wait for all workers and return all results |
| `majority` | Return the result agreed upon by a majority of workers |
| `quorum(N)` | Return when at least N workers agree |
| `first(N)` | Return the first N results |

### Sending Work to a Pool

Invoke a pool using dot-method syntax:

```
result = <pool_name>.send(<method>, <args>)
```

### Examples

**Fact-checking pool with majority consensus:**

```forge
task FactChecker
  needs claim: Text
  gives Text
  do
    result = reason "Is this claim factually accurate? Yes or no, explain briefly: {claim}"
    when result.sure     -> give result
    when result.unsure   -> give "uncertain"
    else                 -> give "could not verify"

pool fact_checkers
  workers: FactChecker * 3
  strategy: majority
  timeout: 15s
```

Sending work:

```forge
verdict = fact_checkers.send("check", "The speed of light is 299,792 km/s")
say "Verdict: {verdict}"
```

**Game room pool with fallback:**

```forge
pool rooms
  workers: room_agent * 10
  strategy: fastest
  timeout: 30s
  fallback: lobby_handler
```

### Best Practices

1. **Use `majority` or `quorum` for correctness-critical decisions** -- multiple workers reduce hallucination risk
2. **Use `fastest` for latency-sensitive work** -- when any single result is acceptable
3. **Always set a timeout** -- prevent indefinite blocking when workers are slow
4. **Provide a fallback** -- graceful degradation when the pool cannot produce a result

---

## 12. Wardens

Wardens are supervision controllers that monitor agents and enforce failure recovery policies. They implement escalation ladders -- sequences of increasingly severe responses to repeated failures.

### Syntax

```
warden <name>
  manages [<agent1>, <agent2>]

  on <failure_type>: <response>, <scope>
    after <N>: <response>
    after <M>: <response>

  max_retries <N> per <duration> then escalate
```

### Components

| Component | Required | Description |
|-----------|----------|-------------|
| `warden <name>` | Yes | Declares a warden with a lowercase identifier |
| `manages [...]` | Yes | List of agent or task names this warden supervises |
| `on <failure>: <response>, <scope>` | No | Policy for a specific failure type |
| `after <N>: <response>` | No | Escalation step after N occurrences |
| `max_retries <N> per <duration> then escalate` | No | Rate-limited retry cap |

### Failure Types

| Type | Description |
|------|-------------|
| `stuck` | Agent cannot make progress |
| `crash` | Agent encountered an unrecoverable error |
| `hallucination` | Agent produced output that failed validation |
| `contradiction` | Agent output contradicted a verified claim or prior state |
| `budget` | Agent exceeded its token or cost budget |
| `timeout` | Agent did not respond within the allowed time |

### Responses

Responses are ordered by severity. Escalation ladders must increase in severity:

| Response | Severity | Description |
|----------|----------|-------------|
| `nudge` | 1 (lowest) | Send a hint to the agent to retry |
| `downgrade` | 2 | Switch the agent to a lower model tier |
| `restart` | 3 | Restart the agent from its initial state |
| `replace` | 4 | Replace the agent with a fresh instance |
| `escalate` | 5 (highest) | Escalate to a higher-level supervisor or human |

### Scopes

| Scope | Description |
|-------|-------------|
| `self` | Apply the response only to the failing agent |
| `downstream` | Apply to the failing agent and its dependents |
| `all` | Apply to all agents managed by this warden |

### Escalation Ladders

The `after` clause defines progressive responses. Both the count and the severity must increase with each step:

```forge
on stuck: nudge, self
  after 3: restart
  after 5: escalate
```

This means: on first stuck, nudge the agent. After 3 stuck occurrences, restart it. After 5, escalate.

### Compiler Enforcement

The warden checker produces **errors** for:

| Error | Description |
|-------|-------------|
| Unknown managed name | A name in the `manages` list does not match any declared agent, warden, flow, or pool |
| Non-increasing `after` count | An `after` clause has a count less than or equal to the previous one |
| Non-increasing severity | An `after` clause has a response that is not more severe than the previous one |

The warden checker produces **warnings** for:

| Warning | Description |
|---------|-------------|
| Incomplete failure coverage | The warden does not define policies for all six failure types |

### Agent-Level Overrides

Agents can override their warden's policies using a `warden_override` block inside the agent declaration. The override takes precedence for the specified failure type:

```forge
agent classifier
  warden_override
    on stuck: replace, self

  on start
    say "ready"
```

### Examples

**Basic warden with escalation ladder:**

```forge
agent bot
  on handle(msg: Text)
    say msg

warden supervisor
  manages [bot]
  on stuck: nudge, self
    after 3: restart
```

**Comprehensive warden with rate limiting:**

```forge
warden supervisor
  manages [bot]
  on stuck: nudge, self
    after 3: restart
    after 5: escalate
  on crash: restart, all
    after 3: escalate
  on hallucination: replace, downstream
  on budget: escalate, self
  on timeout: restart, self
    after 2: replace
  max_retries 10 per 60s then escalate
```

### Best Practices

1. **Cover all six failure types** -- the compiler warns about incomplete coverage for a reason
2. **Start with `nudge`** -- give agents a chance to self-correct before restarting or replacing
3. **Use `after` for progressive escalation** -- a single severe response on first failure is usually too aggressive
4. **Scope responses narrowly** -- prefer `self` over `all` unless the failure genuinely affects the entire group
5. **Pair wardens with agent stuck policies** -- the agent's `if stuck` block handles local recovery; the warden handles systemic failure

---

## 13. Contracts

Contracts define behavioral interfaces that agents or other implementations must satisfy. A contract declares a set of capabilities using `can` signatures.

### Syntax

```
contract <Name>
  can <method>(<param>: <Type>, ...) -> <ReturnType>
  can <method>(<param>: <Type>, ...) -> <ReturnType>
```

### Components

| Component | Description |
|-----------|-------------|
| `contract <Name>` | Declares a named contract interface |
| `can <method>(...)` | Declares a capability with typed parameters and return type |
| `-> <ReturnType>` | Required return type for each capability |

### Rules

- A contract must contain at least one `can` signature
- Each `can` signature requires an explicit return type after `->`
- Parameters follow the same `name: Type` syntax as task and pure function parameters
- Contracts do not contain implementation bodies -- they declare only the interface

### Example

```forge
contract GameRoom
  can join(player: Text) -> Text
  can move(player: Text, cell: Number) -> GameResult
```

This contract declares that any implementation of `GameRoom` must provide a `join` capability that accepts a `Text` parameter and returns `Text`, and a `move` capability that accepts a player name and cell number and returns a `GameResult`.

---

## 14. Systems

Systems are top-level composition units that wire named components together using the `use` block and the compose (`>>`) operator. A system declares which implementations to use and how data flows between them.

### Syntax

```
system <name>
  use
    <alias>: <implementation>
    <alias>: <implementation>
  <alias> >> <alias>
```

### Components

| Component | Description |
|-----------|-------------|
| `system <name>` | Declares a named system |
| `use` | Block that binds implementation names to local aliases |
| `<alias>: <impl>` | Maps a local name to a concrete implementation (agent, handler, etc.) |
| `<alias> >> <alias>` | Compose operator wiring data flow between components |

### Rules

- The `use` block is optional but typical -- it binds aliases to implementations
- Each binding in `use` is indented at level 2 (4 spaces) and follows `alias: implementation` format
- Compose expressions (`>>`) at level 1 (2 spaces) define the data flow topology between aliases
- A system may contain zero or more compose expressions

### Example

```forge
system tictactoe
  use
    game: room_agent
    lobby: lobby_handler
  game >> lobby
```

This system wires the `room_agent` implementation as `game` and `lobby_handler` as `lobby`, then declares that `game` feeds into `lobby` via the compose operator.

---

## 15. Endpoints

Endpoints are server-side entry points that expose functionality over a network boundary. They follow function-like syntax with parameters and an optional return type, and contain a body of statements.

### Syntax

```
endpoint <name>(<param>: <Type>, ...) -> <ReturnType>
  <statements at 2-space indent>
```

### Components

| Component | Description |
|-----------|-------------|
| `endpoint <name>` | Declares a named endpoint |
| `(<param>: <Type>)` | Parameter list (may be empty) |
| `-> <ReturnType>` | Optional return type |
| Body | One or more statements at level 1 (2-space indent) |

### Compiler Enforcement

Endpoints are restricted by the boundary system:

- Endpoints are **only allowed** in files with `#! boundary: server`
- Declaring an endpoint in a `shared` boundary (the default) or a `client` boundary produces a compile error

### Example

```forge
#! boundary: server

endpoint login(user: Text, pass: Text) -> Text
  give "ok"
```

---

## 16. Boundary Directives

Boundary directives declare which execution context a file belongs to: `server`, `client`, or `shared`. This controls which constructs are allowed and which cross-file references are valid.

### Syntax

```
#! boundary: <kind>
```

Where `<kind>` is one of: `server`, `client`, `shared`.

The boundary directive must appear on the first line of the file, before any other declarations.

### Rules

| Rule | Description |
|------|-------------|
| Default boundary | Files without a directive are `shared` |
| `endpoint` restriction | Only allowed in `server` boundary |
| Server-client isolation | Server code cannot reference client symbols |
| Client-server isolation | Client code cannot reference server symbols |
| Shared access | Both `server` and `client` code may reference `shared` symbols |
| Serializable types in shared | Shared boundary must not contain agent, pool, or flow references -- only serializable types |

### Cross-Boundary Reference Rules

```
server.forge  -->  shared.forge   OK
client.forge  -->  shared.forge   OK
server.forge  -->  client.forge   COMPILE ERROR
client.forge  -->  server.forge   COMPILE ERROR
shared.forge  -->  shared.forge   OK
```

### Examples

**Shared boundary** (default -- no directive needed, or explicit):

```forge
#! boundary: shared

pure is_api_key
  needs line: Text
  gives Bool
  do
    if line.contains("sk-ant-")
      give true
    give false
```

**Server boundary:**

```forge
#! boundary: server

event PlayerJoined
  player: Text
  room: Text

agent room_agent
  lifecycle: GamePhase
  memory
    board: Text[9]
  ...
```

---

## 17. Uncertain Value Handling (Principle I: Honesty)

FORGE enforces **taint tracking** on LLM oracle outputs to guarantee that uncertain values are never silently trusted. This is a core language invariant derived from Principle I (Honesty): the system must never present an LLM guess as a known fact.

### Taint Rules

| Rule | Description |
|------|-------------|
| Oracle expressions produce taint | `reason`, `classify`, `search`, `recall`, `exec`, `command`, `session`, and `command.*`/`session.*` methods return tainted (uncertain) values |
| Taint blocks `give` | A tainted value **cannot** be passed directly to `give` -- this is a compile error |
| Clearing taint | The value must be dispatched through `when` or `match` before it can be used in `give` |
| Taint propagates through assignment | Assigning a tainted value to a new variable keeps it tainted |
| Taint propagates through field access | Accessing a field on a tainted value remains tainted |
| Reassignment clears taint | Reassigning a variable to a non-oracle expression clears its taint |

### Compiler Enforcement

The uncertain checker rejects any code path where a tainted value reaches `give` without passing through `when` or `match`. The compile error message contains: `unhandled uncertain`.

### Correct Pattern: Taint Cleared via `when`

The `when` construct dispatches on confidence levels, forcing the programmer to explicitly handle the uncertain nature of the oracle result:

```forge
task analyze
  needs text: Text
  gives Text
  do
    result = reason "analyze {text}"
    when result.sure -> give result
    when result.unsure -> give "uncertain"
    else -> give "unknown"
```

This compiles successfully because every path through `when` explicitly acknowledges the confidence level before reaching `give`.

### Incorrect Pattern: Tainted Value Given Directly

```forge
task bad_analyze
  needs text: Text
  gives Text
  do
    result = reason "analyze {text}"
    give result
```

This produces: **compile error -- unhandled uncertain value**. The variable `result` carries taint from `reason` and has not been dispatched through `when` or `match`.

### Incorrect Pattern: Inline Oracle in `give`

```forge
task also_bad
  needs text: Text
  gives Text
  do
    give reason "analyze {text}"
```

This also produces a compile error. The oracle result flows directly into `give` with no confidence dispatch.

### Incorrect Pattern: Taint Survives Reassignment

```forge
task still_bad
  needs text: Text
  gives Text
  do
    result = reason "analyze {text}"
    copy = result
    give copy
```

This produces a compile error. Assigning a tainted value to another variable does not clear the taint -- `copy` inherits the taint from `result`.

### Correct Pattern: Taint Cleared via `match`

```forge
task classify_text
  needs text: Text
  gives Text
  do
    result = classify text into ["positive", "negative", "neutral"]
    match result
      Positive -> give "positive"
      Negative -> give "negative"
      _ -> give "neutral"
```

The `match` construct forces structural dispatch, which clears the taint on each branch.

### Design Rationale

Without this enforcement, an LLM hallucination could propagate silently through a program and be returned as authoritative output. By requiring explicit confidence dispatch, FORGE guarantees that every oracle result is acknowledged as uncertain before it can influence program output. This is a compile-time guarantee, not a runtime check.

---

## 18. Command, Exec, and Process Handles

FORGE has two process-execution surfaces:

- `exec` is the older direct CLI primitive. It executes a shell command and returns `uncertain<Text>`.
- `command` is the structured process primitive. It returns a record with `stdout`, `stderr`, `exit_code`, and `success`, or a background handle when `background true` is used.

Both are side-effecting runtime operations. They are allowed in `task` bodies and agent handlers, rejected inside `pure`, and must be handled through confidence-aware control flow before being returned with `give`.

### Exec

```forge
task git_status_summary
  gives Text
  do
    result = exec "git status --short"
    when result.sure -> give result
    when result.unsure -> give "status uncertain: {result}"
    else -> give "could not read status"
```

### Command

`command` accepts either a shell string or a structured argv array. Prefer the array form when interpolating user or model-provided values.

```forge
task run_tests
  gives Text
  do
    result = command ["cargo", "test"] timeout 10m
    when result.sure -> give result.stdout
    when result.unsure -> give result.stderr
    else -> give "test command failed"
```

Supported modifiers:

| Modifier | Example |
|----------|---------|
| Working directory | `command ["cargo", "test"] in "crates/core"` |
| Timeout | `command "npm test" timeout 2m` |
| Environment | `command "cargo test" env { RUST_LOG: "debug" }` |
| Background execution | `command "cargo watch -x test" background true` |

Background commands return a handle. Use the imperative command methods to inspect or cancel it:

```forge
task watch_once
  gives Text
  do
    handle = command "cargo watch -x test" background true
    status = command.status(handle)
    output = command.output(handle)
    command.cancel(handle)
    when output.sure -> give output.stdout
    else -> give "no output"
```

---

## 19. Sessions, AgentResult, and Verification Metadata

`session` starts or resumes a long-running external agent session through a configured adapter. It supports Claude Code, Codex, generic CLI adapters, and project-local adapter configuration.

### Syntax

```forge
task review_patch
  gives AgentResult
  do
    result = session "code-review" agent "claude" prompt "Review this patch for bugs" tools ["Read", "Grep"] timeout 5m budget 0.50 gives AgentResult
    when result.sure -> give result
    when result.unsure -> give result
    else -> give AgentResult(plan: "review failed", confidence: 0.0)
```

Supported modifiers:

| Modifier | Purpose |
|----------|---------|
| `agent "name"` | Selects an adapter by name |
| `prompt "text"` | Supplies the agent prompt |
| `tools [...]` | Requests adapter-specific tools |
| `timeout 5m` | Bounds runtime |
| `budget 0.50` | Bounds spend |
| `gives AgentResult` | Requests the typed agent result contract |
| `on progress -> emit Event(...)` | Emits progress events |
| `on complete -> emit Event(...)` | Emits completion events |
| `isolate worktree "branch-name"` | Runs the session in an isolated git worktree |

### Hooks

```forge
event ReviewUpdate
  payload: Text

event ReviewDone
  payload: Text

task run_review
  gives Text
  do
    result = session "code-review" agent "codex" prompt "Review src/runtime for bugs" on progress -> emit ReviewUpdate(it) on complete -> emit ReviewDone(it)
    when result.sure -> give result
    else -> give "review failed"
```

### Session Methods

`session.status(id)` returns a record with `status`, `cost_usd`, `started_at`, `updated_at`, and `error`. Session method results are uncertain and must be handled before `give`.

### AgentResult

`AgentResult` is a built-in typed result contract for external agent work:

| Field | Meaning |
|-------|---------|
| `plan` | The intended or executed plan |
| `patch_summary` | Summary of code changes |
| `files_changed` | Files changed by the agent |
| `tests_run` | Verification commands attempted |
| `tests_passed` | Whether verification passed |
| `cost_usd` | Session cost |
| `confidence` | Agent-reported confidence |
| `approval_needed` | Whether human approval is required |
| `metadata` | Extensible runtime metadata |

`AgentResult()` builds a default result. Field initializers override defaults:

```forge
AgentResult(plan: "fix parser example", confidence: 0.8)
```

When runtime verification is enabled, `AgentResult.metadata.verification` contains a `VerificationResult` with `status`, `claims`, `evidence`, `contradictions`, and `risk_class`. Contradictions are reported to wardens that declare an `on contradiction:` policy, or handled by the runtime fallback policy.

---

## 20. Knowledge, Recall, Spawn, Find, and Retire

FORGE agents can own persistent searchable knowledge and spawn specialized child agents.

### Knowledge Store

```forge
exportable agent forge_sensei
  lifecycle: MasteryLevel
  memory
    interaction_count: Number
  knowledge store: ".forge-knowledge/sensei"
    max_entries: 50000
    retention: 365d
```

`recall "query"` searches the agent knowledge store and returns an uncertain value:

```forge
task answer_from_memory
  needs question: Text
  gives Text
  do
    prior = recall "FORGE {question}"
    when prior.sure -> give prior
    when prior.unsure -> give "partial memory: {prior}"
    else -> give "no matching memory"
```

`learn` writes knowledge:

```forge
learn "Tasks can call oracle primitives" category: "TASKS"
learn from interaction(question, answer, 0.7)
learn from document("docs/forge-reference.md")
```

### Spawn and Find

`spawn` creates a runtime agent instance from an agent template. It can assign an alias, filter knowledge, cap confidence, initialize memory, and request worktree isolation:

```forge
child = spawn specialist as "specialist_{topic}"
  with knowledge where category == topic
  with confidence_cap: 0.8
  with memory topic: topic
  isolate worktree "specialist-{topic}"
```

`find "alias"` locates one runtime instance. `find all template where lifecycle == ready` locates matching instances by template and optional lifecycle.

```forge
existing = find "specialist_{topic}"
ready_specialists = find all specialist where lifecycle == ready
```

`retire` shuts down an agent instance and can export knowledge:

```forge
retire "specialist_FORGE"
  with knowledge export: "exports/specialist.forgepkg.json"
```

---

## 21. Skills, Built-In Capabilities, and Project Manifests

FORGE source declares capabilities with `use`. Built-ins include LLM operations, web/data/html/markdown helpers, assets, command execution, and project skills.

```forge
#! boundary: server

use
  llm.reason
  web.fetch
  data.store
  markdown.render
  skill.repo_check
```

Built-in capability families:

| Family | Examples |
|--------|----------|
| LLM | `llm.reason`, `llm.classify` |
| Web | `web.fetch`, `web.post` |
| Data | `data.store`, `data.get`, `data.list`, `data.delete`, `data.embed`, `data.search` |
| HTML | `html.layout`, `html.escape` |
| Markdown | `markdown.render` |
| Assets | `asset` |
| Command | `command.status`, `command.output`, `command.cancel` |
| Skills | `skill.<namespace>.<capability>(...)` |

`search "query"` is restricted to `#! boundary: server`.

Project skills are declared in `forge.project.toml`. Validate them through the project/manifest path, not as isolated single-file examples, because the checker needs the manifest-provided skill registry.

```toml
[project]
name = "skill-project-demo"

[build]
entry = "main.forge"

[skills]
repo_check = {}
```

Then a FORGE file can use `skill.repo_check` and call declared capabilities. Skill calls are side-effecting and rejected inside `pure`.

Typed skill capabilities may optionally declare deterministic execution metadata in `SKILL.md`. When present, the runtime expands capability arguments into a structured `argv` command and executes it directly; when absent, the existing LLM-mediated SKILL.md agentic loop remains the fallback. Deterministic skill executors do not make LLM requests and have zero token cost, but they still return skill confidence rather than deterministic confidence because they perform external side effects.

```yaml
capabilities:
  - name: add_reaction
    inputs: [Text, Text, Text]
    output: Text
    params: [channel, timestamp, emoji]
    executor:
      kind: command
      argv: [curl, -s, -X, POST, "https://slack.com/api/reactions.add", -H, "Authorization: Bearer {env:SLACK_BOT_TOKEN}", --data-urlencode, "channel={channel}", --data-urlencode, "timestamp={timestamp}", --data-urlencode, "name={emoji}"]
      result:
        success_path: ok
        error_path: error
```

Use `{param}` placeholders for capability arguments and `{env:NAME}` for environment variables. Use `{{` and `}}` for literal braces inside argv templates.

---

## 22. Templates and Raw Interpolation

Normal template interpolation uses `{expr}` and is escaped in HTML contexts. Raw interpolation uses `{!expr}` and skips HTML escaping. Use raw interpolation only for trusted or already-sanitized HTML.

```forge
endpoint page() -> Html
  body = markdown.render("# Hello")
  give html.layout("Docs", "{!body}")
```

---

## 23. Example Validation Buckets

Examples are not all validated the same way:

| Bucket | Validation |
|--------|------------|
| Positive single-file examples | `cargo run -- check <file>` |
| Expected-error examples | Keep the expected diagnostic in the example name, comment, or test fixture |
| Live LLM/session examples | Check syntax locally; run only when real provider credentials and CLI adapters are available |
| Manifest skill examples | Use `cargo run -- run --manifest <forge.project.toml>` or the checker path that loads the manifest registry |
| Multi-file examples | Validate through `forge.project.toml` or a merged-source command, not by checking dependent files in isolation |

When an example intentionally exercises a checker limitation, document the limitation beside the example or in the issue verification notes rather than treating a nonzero checker result as a passing smoke test.
