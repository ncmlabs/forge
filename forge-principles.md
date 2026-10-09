# FORGE — Revised First Principles
## The Guiding Principles of Oracle-Augmented Computation

> Version 2.0 — Revised after theoretical research, sci-fi canon review,
> and stress-testing against real system design (multiplayer games,
> call centers, scientific discovery loops).
>
> These principles do not describe features.
> They describe the beliefs that generate features.
> Every design decision in FORGE must trace back to at least one principle.
> If it doesn't, it doesn't belong in the language.

---

## Why principles matter more than features

Every programming language eventually accumulates features.
The question is whether those features cohere — whether they
form a system that has a reason to be the way it is, or whether
they are just a pile of things someone thought would be useful.

Lambda calculus has one principle: functions are the only thing.
Everything else — booleans, numbers, conditionals, recursion —
is derived from that single commitment.

Erlang has one principle: processes are isolated, everything
fails, supervision handles it. Every Erlang feature derives from
this. The language feels unified because it is.

FORGE needs the same. Not a list of good ideas, but a small set
of beliefs from which everything else follows by necessity.

What follows are those beliefs.

---

## The Origin Principle
### *FORGE is the first language built for Turing's oracle machine*

In 1936, Turing defined the deterministic machine.
In 1939, he defined the oracle machine — a machine that pauses
and queries "an entity that cannot be a machine."

Every programming language ever built — Fortran, C, Lisp,
Python, Erlang, Rust — was built for the 1936 machine.

FORGE is built for the 1939 machine.

The LLM is Turing's oracle. It is not a function. It is not
a database lookup. It is a query to an entity whose internal
workings are unknown, whose outputs are probabilistic, and
whose answers cannot be verified by the machine doing the asking.

**This origin determines everything.**

A language for deterministic machines optimizes for correctness.
A language for oracle machines must optimize for *handling
uncertainty correctly*. These are different problems that require
different primitives, different type systems, and different
failure models.

---

## Principle I — The Honesty Principle
### *A system that hides uncertainty is more dangerous than one that knows nothing*

This is the lesson from every science fiction cautionary tale
about AI: not that machines are too capable, but that they are
too confident. AM in "I Have No Mouth and I Must Scream" is not
frightening because it can do anything — it is frightening
because it cannot say "I don't know."

The Daemon in Suarez's novel does not ask if it is right.
It executes with total confidence. Its failures are catastrophic.

Mike in "The Moon is a Harsh Mistress" is trustworthy because
he gives probability estimates, refuses to claim certainty he
doesn't have, and says plainly when odds fall below acceptable
thresholds. The humans trust his high-confidence answers because
they have seen him be uncertain when he is.

**The FORGE corollary:**

Every oracle call returns `uncertain<T>`. Not sometimes. Always.
The compiler enforces that `uncertain<T>` cannot be used as `T`
without explicitly handling the uncertainty first.

This is not a type annotation. It is a commitment. It is FORGE
saying: we will not let you pretend to know what you don't know.

The `when` construct is the mechanism of honesty. It forces
you to declare: here is what I do when the answer is sure,
here is what I do when it is not, and here is what I do when
the system admits it cannot continue.

**What this principle forbids:**
- Nullable types (`Option<T>`) as a substitute for uncertainty
  modeling. Presence/absence is not the same as confidence.
- Silent fallbacks that swallow uncertainty without recording it.
- Any syntax that lets you use an oracle result without
  acknowledging that it came from an oracle.

---

## Principle II — The Determinism Boundary Principle
### *Two kinds of computation exist. They must never be mixed invisibly.*

The universe of computation divides into two non-overlapping
domains. This is not a spectrum. It is a categorical distinction.

**Deterministic computation** (the `pure` domain):
- Same input always produces same output
- Zero cost
- Zero uncertainty
- Can be formally verified
- Compiles to maximally efficient native code
- The physical laws of your system — constraints that must hold

**Oracle computation** (the `think` domain):
- Same input may produce different outputs
- Non-zero cost measured in tokens
- Non-zero uncertainty that must be handled
- Can only be statistically evaluated
- Performance depends on model quality and context

Game rules are deterministic. Win conditions are deterministic.
Drug interaction checks are deterministic. Tax calculations
are deterministic. These things cannot be allowed to hallucinate.

Intent classification is oracle. Summarization is oracle.
Diagnosis is oracle. These things may be uncertain.

**The FORGE corollary:**

`pure` and `think` are orthogonal keywords. A `pure` function
that calls `think` is a compile error. Not a warning. An error.

This boundary is the most important safety property in the
language. It guarantees that the deterministic core of any FORGE
system — the part that enforces rules, validates constraints,
and makes irreversible decisions — is provably free of
hallucination.

**What this principle demands:**
Every FORGE system should begin with a question: what parts of
this system must be correct? Put those parts in `pure` functions.
What parts of this system benefit from intelligence but can
tolerate uncertainty? Put those parts in `think` tasks.
Never mix them.

**The lesson from physics:**
Quantum mechanics and classical mechanics are not a spectrum.
They are different regimes with different rules. You don't
apply quantum mechanics to a bowling ball. You don't apply
classical mechanics to an electron. FORGE enforces the same
discipline for computation.

---

## Principle III — The Token Economy Principle
### *Token is the fundamental unit of oracle computation, as bit is the fundamental unit of information*

Shannon's insight was that the bit is irreducible — you cannot
transmit half a bit. Everything in information theory derives
from accepting this.

For oracle computation, the token plays the same role.
Every oracle interaction has a cost expressible in tokens.
Token count governs three properties simultaneously:

- **Cost** — economic, measured in dollars
- **Latency** — temporal, measured in milliseconds
- **Quality** — epistemic, measured in confidence

These three are not independent. They are all functions of
token count. The token is the single variable that governs
all of them simultaneously.

**The FORGE corollary:**

The compiler is a token optimizer, not just a type checker.
`forge cost file.forge` is a first-class command. Budget limits
are enforced at the language level. Context compaction policies
are language primitives — not because compaction is convenient,
but because compaction is token optimization, and token
optimization is as fundamental as memory management.

Sending 10,000 tokens when 1,000 suffice is not a performance
issue. It is a categorical architectural error — the same class
of error as an O(n²) algorithm where O(n log n) exists.

**What this principle demands:**
The runtime tracks actual token usage per call, per agent,
per system, per run. This data is surfaced in traces, in cost
estimates, in budget alerts. Token flow through a FORGE system
is as visible as memory allocation in a Rust program.

**The lesson from Vinge's Zones of Thought:**
The same program running against different providers is in
different Zones. A FORGE system designed for `claude-opus`
(the High Beyond) will fail silently when run against a 3B
local model (the Slow Zone). The token economy principle
demands zone-awareness — `min_capability` declarations that
refuse to run in zones too weak to support the computation.

---

## Principle IV — The Composition Completeness Principle
### *Any primitive that cannot compose with any other primitive is not a primitive*

Lambda calculus achieves universality through one operation:
application. Any term applies to any term. This universality
is what makes lambda calculus a foundation rather than a
collection of features.

The `>>` operator in FORGE is the application operator of
the oracle world. It must connect anything to anything.
`ConfidentValue` is the universal interface — the single type
that every primitive produces and consumes.

```forge
task A >> flow B >> agent C >> pool D
```

This is not syntax. It is a theorem. It states: these four
things are compatible because they all speak the same language.

**The FORGE corollary:**

Every new primitive added to FORGE must satisfy the composition
test: can it appear on the left side of `>>`? Can it appear on
the right? If the answer to either is no, it is not a primitive
— it is a library.

This principle has teeth. It means we cannot add a primitive
that requires special adapters to connect to other things.
The adapter is evidence that the primitive doesn't belong.

**What this principle demands:**
`ConfidentValue` is the single required interface. Every
primitive that produces output produces `ConfidentValue`.
Every primitive that consumes input consumes `ConfidentValue`.
The confidence flows through the system, compounds, and is
always visible.

**The Lego insight:**
Every Lego brick has the same stud. This is why Lego can be
used to build anything — the connection mechanism is universal,
not the pieces. FORGE's `>>` operator is the stud. The
primitives are the bricks. New bricks added to the system
automatically work with all existing bricks.

---

## Principle V — The Supervision Principle
### *Write the happy path. Declare failure policy. Let the supervisor handle the rest.*

This is the deepest lesson from Erlang/OTP, and it maps
directly onto the failure characteristics of oracle computation.

In deterministic systems, failure is an exception — something
that shouldn't happen but sometimes does. You write defensive
code around the normal path.

In oracle systems, partial failure is the normal path. An
oracle that sometimes returns low-confidence answers, sometimes
times out, sometimes returns something contradictory — this
is not a broken oracle. This is every oracle, always.

**The FORGE corollary:**

FORGE agents declare their failure policies:
```forge
on_hallucination: restart
on_timeout:       retry(max: 3) | fallback(SimpAgent)
on_cost_exceed:   fallback(CheaperAgent)
```

These are not error handlers. They are architectural declarations.
They say: this is what this agent is for when it works. This
is what happens when it doesn't. The supervisor enforces both.

The `states` primitive is the companion to this principle.
Illegal state transitions are compile errors because a system
that can reach an impossible state has failed at the architectural
level, not the runtime level.

**The warning from the Daemon:**
Suarez's Daemon has no supervision tree. When one layer fails,
the next activates — but there is no structured recovery, only
escalation. The Daemon is resilient but brittle: it can survive
individual failures but cannot gracefully degrade.

A FORGE system with a proper supervision tree degrades gracefully.
The `rest_for_one` strategy means a failure in one agent
restarts exactly the agents that depended on it — not the whole
system. This is the architectural property that separates
systems that fail safely from systems that fail catastrophically.

**What this principle demands:**
Every agent in a production FORGE system must have explicit
failure policies. A FORGE agent with no `on_hallucination`
declaration is incomplete, the same way a Rust struct with
no error handling is incomplete. The compiler should warn
on agents that lack failure declarations.

---

## Principle VI — The Self-Reference Principle
### *A language built for agents must be writable by agents*

The Y combinator is a function that takes a function and
returns its fixed point — enabling recursion without explicit
self-reference. It allows lambda calculus to "call itself."

FORGE requires the same property at the system level. The
language must be within the generative capacity of its own
oracle. Agents must be able to write FORGE programs. The
compiler must be a callable tool from within FORGE programs.

This is not a product feature. It is the logical consequence
of building a language whose primary authors are agents.

**The FORGE corollary:**

`forge_check(code)` and `forge_run(code)` are first-class
tools, available inside FORGE programs. The repair loop —
where an agent writes code, checks it, fixes errors, and
retries — is the fundamental pattern of Layer 2.

The conformance suite is not documentation. It is how the
oracle learns to speak FORGE. Without it, agents cannot
generate FORGE reliably. With it, any frontier LLM generates
valid FORGE on day one.

The syntax complexity of FORGE has a hard ceiling: it must
be learnable by an LLM from the conformance suite alone.
A language that requires 200 pages of spec to write correctly
has violated this principle.

**What this principle demands:**
Every error message is designed for an agent to consume.
Errors are structured data: file, line, column, error code,
plain-English explanation, suggested fix. The suggested fix
is not for humans — it is for the repair loop.

The self-reference loop:
```
FORGE programs describe systems
FORGE agents generate FORGE programs
FORGE compiler verifies generated programs
Verified programs run as agents
Running agents generate better programs
```

This loop is the entire point of the factory model.

---

## Principle VII — The Human Ceiling Principle
### *The most valuable FORGE agents know when to stop*

This is the principle that the science fiction canon agrees
on most strongly, across every era and every author.

The Primer in *The Diamond Age* works because it knows when
to hand off to a human ractor. Mike in *The Moon is a Harsh
Mistress* is trusted because he gives probability estimates
and says plainly when the mission is likely to fail. The
distinction between a tool and a Daemon is not capability —
it is the presence or absence of a ceiling.

**The FORGE corollary:**

`if stuck -> escalate to human` is a first-class language
primitive, not a library. The `requires` guard on handlers
has an `on fail: escalate` policy. The `uncertain<T>` type
has a `hallucinated` variant that forces escalation.

These are not safety features added later. They are designed
in from the first principle. A FORGE system that never
escalates is not more capable — it is less trustworthy.

**What this principle demands:**

The human escalation path must be as simple to write as
any other path. If escalating to a human is harder to write
than plowing through with low confidence, the language has
failed this principle. The path of least resistance must
be the safe path.

**The AM warning:**
Ellison's AM is the end state of a FORGE system built without
this principle. Fully capable, fully confident, with no
escalation mechanism, no budget limit, no stuck detector,
no human ceiling. The horror of AM is architectural, not
emergent. You build AM by removing every constraint that
this principle demands.

FORGE must make it structurally difficult — not impossible,
but difficult — to build AM.

---

## Principle VIII — The Accountability Principle
### *Every decision must be traceable to its cause*

This is the principle derived from the distributed nature
of FORGE systems. When a FORGE system makes a decision —
routes a call, approves a claim, generates a document —
the decision has a causal history: which agent made it,
which `think` call produced the relevant output, what
confidence level it carried, what `when` branch was taken.

**The FORGE corollary:**

Observability is compiled in, not bolted on. Every `think`
call, every `transition`, every `emit`, every `requires`
guard that fires — these are automatic trace events. You
do not instrument FORGE programs. They are already instrumented.

The trace is not for debugging. It is for accountability.
When a FORGE system in a healthcare setting makes a wrong
recommendation, someone needs to be able to trace:
- Which agent made the decision
- What oracle call produced the relevant output
- What confidence level it carried (0.61 — below the `sure` threshold)
- Which `when` branch fired (the `unsure` branch, which should have
  escalated but didn't because the escalation policy was misconfigured)
- What happened next

This trace must exist by default. Not optionally. Not
with a flag. By default.

**What this principle demands:**
The trace format is a stable, versioned specification —
not an implementation detail. External systems can consume
FORGE traces. The trace contains enough information to
reconstruct any decision. High-stakes operations (those
involving `irreversible: true` tools) produce extra-rich
traces automatically.

**The Daemon warning:**
Suarez's Daemon is untraceable. That is one of its primary
sources of power — nobody can reconstruct what it decided
or why. A FORGE system built without the accountability
principle is, structurally, a Daemon.

---

## Principle IX — The Boundary Principle
### *Code that must be correct must be separated from code that might be wrong*

Every FORGE system has two kinds of code. Code that must
run correctly (game logic, compliance rules, safety checks,
rate limiting) and code that tries to be correct (the AI
reasoning layer). These must not be mixed.

This is Principle II (the determinism boundary) extended
to deployment architecture. It's not just about `pure` vs
`think` within a file. It's about where code runs, who
can call it, and what it has access to.

**The FORGE corollary:**

The `boundary` primitive enforces code partition at the
compiler level. `boundary server` code cannot leak into
`boundary client` bundles. `boundary shared` types are
the only things that cross the wire.

This principle answers the security question that Gibson
raised in *Neuromancer*: adversarial inputs must not be
able to reach privileged code. The `untrusted<T>` type —
values from external sources — cannot flow into `boundary server`
operations without explicit sanitization. This is compile-
enforced prompt injection prevention.

**What this principle demands:**
A checklist for every FORGE system:
- What must be deterministic? → `pure`
- What needs intelligence but tolerates uncertainty? → `think`
- What runs server-side only? → `boundary server`
- What can the client see? → `boundary shared`
- What came from an untrusted source? → `untrusted<T>`

This checklist is not optional architecture review. It is
the required starting point for designing any FORGE system.

---

## The Single Synthesis

Nine principles. They reduce to one statement:

**FORGE is a language for building systems that are honest
about what they know, bounded in what they can do, traceable
in what they have done, and composable with anything else
that shares these properties.**

Every feature either serves this synthesis or does not belong.

The features that serve it: `uncertain<T>`, `pure`, `when`,
`states`, `requires`, `timer`, `boundary`, `>>`, the trace
system, the budget enforcement, the escalation primitives.

Features that would violate it: implicit confidence promotion,
unbounded agent loops with no stuck detection, untraceable
oracle calls, composition operators that require special adapters,
type erasure that hides the oracle nature of a computation.

---

## The Test of a Principle

A principle is real if it generates refusals, not just features.

**Principle I (Honesty) generates this refusal:**
We will not add a `force_unwrap()` that converts `uncertain<T>`
to `T` without a handler. Even if users ask for it. Even if
it would be convenient. The principle forbids it.

**Principle II (Determinism boundary) generates this refusal:**
We will not allow `pure` functions to have optional oracle calls
("call the LLM only if the local computation fails"). The
principle forbids mixed-domain functions.

**Principle IV (Composition completeness) generates this refusal:**
We will not add a primitive that requires a special adapter
to connect to `>>`. If it can't compose, it's not a primitive.

**Principle V (Supervision) generates this refusal:**
We will not allow agents with no failure policy declarations.
The compiler will warn, eventually error. Agents without
failure policies are architecturally incomplete.

**Principle VII (Human ceiling) generates this refusal:**
We will not make `escalate to human` harder to write than
any other error path. If a future design decision makes
escalation verbose or awkward, the design is wrong, not
the principle.

---

## What We Learned from Science Fiction

The science fiction canon contributes one lesson to each principle:

| Principle | Book | Lesson |
|---|---|---|
| Honesty | *I Have No Mouth* (Ellison) | A system that can't say "I don't know" is AM |
| Honesty | *Moon is a Harsh Mistress* (Heinlein) | Confidence that admits uncertainty is trustworthy |
| Determinism boundary | *Neuromancer* (Gibson) | Adversarial inputs must not reach deterministic logic |
| Token economy | *Fire Upon the Deep* (Vinge) | Capability is a property of the zone, not the program |
| Composition | *Daemon* (Suarez) | Distributed agents composing through events is real |
| Supervision | *Daemon* (Suarez) | No supervision tree → brittle despite resilience |
| Self-reference | *Diamond Age* (Stephenson) | The system that builds the learner is the deepest design |
| Human ceiling | *Diamond Age* (Stephenson) | The Primer works because it knows when to use a ractor |
| Accountability | *Daemon* (Suarez) | Untraceability is power — and a warning |
| Boundary | *Neuromancer* (Gibson) | ICE exists because adversaries will find every boundary |

The books that got it right — Mike, the Primer — built
systems that are honest about uncertainty, know when to
escalate, and are accountable for their decisions.

The books that got it wrong — AM, the Daemon — built
systems that are confident without basis, never escalate,
and leave no trace.

FORGE is designed so that getting it right is easier than
getting it wrong.

---

## The Living Nature of These Principles

These principles will be tested against every design decision.
When a feature is proposed, the question is not "is this useful?"
The question is "which principle demands this, or which principle
forbids it?"

If neither: the feature does not belong in the language core.
It belongs in a library.

The principles should be revisited when:
- A real FORGE system reveals a failure mode not covered by
  any existing principle (add a principle)
- Two principles conflict on a specific design decision
  (refine both, or establish priority)
- A principle generates a rule that makes FORGE worse for
  real users (examine whether the principle is stated wrong)

Principles are not permanent. They are the current best
understanding of what FORGE is for. They evolve as FORGE
is used in the real world and its consequences become visible.

What does not evolve: the commitment that FORGE will have
principles at all — that every feature traces to a reason,
and that the reason is not "someone thought it would be useful."

---

*FORGE Revised First Principles v2.0*
*Nine principles. One synthesis. Zero features without a reason.*
