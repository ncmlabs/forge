    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.26s
     Running `target/debug/forge run examples/docgen.forge`
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

Reserved keywords: `task`, `pure`, `flow`, `stage`, `fn`, `needs`, `gives`, `do`, `give`, `say`, `use`, `when`, `else`, `if`, `match`, `for`, `in`, `try`, `or`, `reason`, `classify`, `search`, `not`, `and`, `true`, `false`, `agent`, `event`, `states`, `emit`, `transition`, `escalate`, `forward`

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
- `call` — invoke other tasks
- `give` — return a value
- `say` — print to stdout
- Control flow: `if/else`, `match`, `for`

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
    result = reason with "Compare these two documents and identify key differences: {doc1} and {doc2}"
    give result
```

**Task calling another task:**

```
task process_user_input
  needs user_text: Text
  gives Text
  do
    cleaned = call normalize_text with user_text
    analyzed = reason with "Analyze this text for sentiment: {cleaned}"
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
      category = classify with "Categorize as positive, negative, or neutral: {feedback}"
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
- `call` (calling other tasks)

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
| `call` (task) | ✓ | ✗ | Compiler error in pure function |
| `call` (pure) | ✓ | ✓ | Allowed |
| Arithmetic | ✓ | ✓ | Allowed |
| `give` | ✓ | ✓ | Allowed |

---

## Best Practices

1. **Use pure functions for deterministic logic** — Keep data transformations, formatting, and validation in pure functions
2. **Use tasks for LLM operations** — Reserve tasks for reasoning, classification, and search
3. **Compose tasks and pure functions** — Tasks can call pure functions; pure functions cannot call tasks
4. **Document confidence** — While pure functions always return 1.0, document task confidence expectations in comments

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
sentiment = llm("Classify sentiment: " + text)

when sentiment.sure -> 
  update_database(sentiment)
when sentiment.unsure -> 
  flag_for_review(sentiment)
when sentiment.unreliable -> 
  request_clarification(text)
else -> 
  log_error("No confidence data available")
```

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
