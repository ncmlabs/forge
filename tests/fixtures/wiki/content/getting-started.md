# Getting Started

## Installation

```bash
git clone https://github.com/ncmlabs/forge.git
cd forge
cargo build --release
```

The `forge` binary will be at `target/release/forge`.

## Your First Program

Create `hello.forge`:

```forge
use
  llm.reason

task greet
  needs name: Text
  gives Text
  do
    result = reason "Say hello to {name} in a creative way"
    when result.sure -> give result
    else -> give "Hello, {name}!"

fn main
  greeting = greet("world")
  say greeting
```

Run it:

```bash
forge run hello.forge
```

## Configuration

Create `forge.config.toml` to set your LLM provider:

```toml
[provider]
name = "anthropic"
model = "claude-sonnet-4-20250514"
```

Supported providers: Anthropic, OpenAI, Ollama, Groq.

## Key Concepts

### Tasks
The fundamental unit of LLM-powered computation. Every `task` call returns an uncertain value.

### Pure Functions
Deterministic functions that the compiler guarantees will never call an LLM.

### When Blocks
Confidence-aware branching. `.sure`, `.unsure`, `.unreliable` — handle uncertainty explicitly.

### Agents
Stateful processes with memory, lifecycles, event handling, and knowledge stores.

## Next Steps

- Read about the [First Principles](/docs?slug=principles)
- Explore the [task](/docs?slug=task) reference
- Learn about [agents](/docs?slug=agent)
