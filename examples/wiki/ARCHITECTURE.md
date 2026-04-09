# FORGE Wiki Architecture

The wiki is a documentation site built entirely in FORGE. It exists to prove that FORGE's primitives compose into a real, working application — not a toy. Every language feature appears at least once, and every component serves a real purpose.

---

## System diagram

```
                          ┌──────────────────────────────────────┐
                          │         warden wiki_supervisor       │
                          │   crash · stuck · hallucination ·    │
                          │   timeout · budget → escalate        │
                          └────────────┬─────────────────────────┘
                                       │ manages
              ┌────────────────────────┼────────────────────────┐
              │                        │                        │
     ┌────────▼────────┐     ┌────────▼────────┐     ┌────────▼────────┐
     │ content_manager  │     │  search_agent   │     │    qa_agent     │
     │  PageLifecycle   │────▶│  subscribe ×3   │────▶│  answer_question│
     │  CRUD + states   │emit │  search + ask   │     │  + tracking     │
     └────────┬─────────┘     └────────┬────────┘     └────────┬────────┘
              │                        │                        │
              │ data.store/get         │ search_docs            │ answer_question
              ▼                        ▼                        ▼
     ┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
     │   ReDB (pages)  │     │   LLM (reason)  │     │   LLM (reason)  │
     └─────────────────┘     └─────────────────┘     └─────────────────┘

     ──────────── event bus: PageCreated · PageUpdated · PagePublished ────────────

                                  ┌──────────┐
     8 endpoints ────────────────▶│ pure HTML │──▶ browser
     (home, docs, search, ask,    │ helpers   │
      api_search, ask_form,       └──────────┘
      admin_generate_docs,
      admin_fact_check)
```

---

## Component inventory

| Component | Primitive | Purpose |
|-----------|-----------|---------|
| `wiki_head`, `nav_bar`, `docs_sidebar`, `hero_section`, `feature_cards`, `page_layout`, `docs_layout`, `not_found_page`, `search_results_html`, `qa_page`, `ask_form_page`, `confidence_tier` | `pure` | Deterministic HTML rendering — no LLM calls allowed |
| `seed_page`, `load_page`, `gather_docs` | `task` | Data operations: store, retrieve, list pages |
| `search_docs` | `task` | LLM search with `classify` + `reason` + `when` confidence gating |
| `answer_question` | `task` | LLM Q&A with `reason` + `when` confidence gating |
| `content_manager` | `agent` | Stateful page CRUD with lifecycle, persistent memory, event emission |
| `search_agent` | `agent` | Reactive search index — subscribes to content events |
| `qa_agent` | `agent` | Q&A wrapper with question tracking |
| `generate_docs` | `flow` | 5-stage DAG: scan → extract (parallel ×3) → generate → fact-check → publish |
| `fact_check_panel` | `pool` | 3 workers, majority strategy, 30s timeout, fallback |
| `fact_check_detail` | `pool` | 3 workers, all strategy (collects every verdict) |
| `wiki_supervisor` | `warden` | Supervises all 3 agents with 5 failure policies |
| `forge_wiki` | `system` | Wires agents: `content >> search >> qa` |
| `PageLifecycle` | `states` | State machine: draft → review → published → archived |
| `PageCreated`, `PageUpdated`, `PagePublished` | `event` | Typed cross-agent notifications |
| `home`, `docs`, `search_page`, `ask_form`, `ask_page`, `api_search`, `admin_generate_docs`, `admin_fact_check` | `endpoint` | HTTP routes returning Html or Text |
| `requires lifecycle == ...` | `requires` | Guard clauses on state transitions |
| `when results.sure / .unsure` | `when` | Confidence-aware branching |
| `#! boundary: server` / `shared` | `boundary` | Server/shared compilation boundary |
| `fn main` | `fn` | Entry point |

---

## Data flow

A typical request follows this path:

```
Browser                  FORGE Runtime                    LLM / Data Store
  │                          │                                │
  │  GET /docs?slug=agent    │                                │
  │─────────────────────────▶│                                │
  │                          │  data.get("page:agent")        │
  │                          │───────────────────────────────▶│
  │                          │◀───────────────────────────────│
  │                          │  markdown.render(content)      │
  │                          │  docs_sidebar(slug)            │
  │                          │  docs_layout(slug, sidebar,    │
  │                          │              rendered)          │
  │◀─────────────────────────│                                │
  │         Html             │                                │
```

LLM-powered requests add a reasoning step:

```
  GET /ask_page?question=... → answer_question(question)
                              → gather_docs() → data.list("page:")
                              → reason "..." with docs context
                              → when answer.sure → answer
                                when answer.unsure → qualified answer
                                else → "I don't have enough information"
                              → confidence_tier(answer)
                              → qa_page(question, rendered, confidence)
```

---

## Event flow

Content changes propagate through typed events to reactive subscribers:

```
content_manager                    event bus                  search_agent
  │                                   │                           │
  │  emit PageCreated(slug, title)    │                           │
  │──────────────────────────────────▶│                           │
  │                                   │  on PageCreated           │
  │                                   │──────────────────────────▶│
  │                                   │     index_version += 1    │
  │                                   │                           │
  │  emit PageUpdated(slug)           │                           │
  │──────────────────────────────────▶│                           │
  │                                   │  on PageUpdated           │
  │                                   │──────────────────────────▶│
  │                                   │     index_version += 1    │
```

Events are defined in `shared.forge` (the `shared` boundary) so both server and future client code can reference them.

---

## Supervision tree

The `wiki_supervisor` warden manages all three agents with escalating recovery policies:

| Failure | First response | Escalation |
|---------|---------------|------------|
| `hallucination` | `restart, self` | After 3: `escalate` |
| `stuck` | `nudge, self` | After 5: `restart` |
| `crash` | `restart, self` | After 3: `escalate` |
| `timeout` | `restart, self` | — |
| `budget` | `downgrade, self` | After 2: `escalate` |

Global rate limit: `max_retries 10 per 1h then escalate`.

The `self` directive means the agent attempts self-recovery before the warden intervenes. On escalation, the agent is removed from the supervision tree — the wiki continues serving with degraded functionality.

---

## Confidence pipeline

Confidence flows from LLM output through to the UI:

```
LLM (reason)                    FORGE runtime              Pure rendering
  │                                 │                          │
  │  uncertain<Text>                │                          │
  │────────────────────────────────▶│                          │
  │                                 │  when .sure → answer     │
  │                                 │  when .unsure → "I'm     │
  │                                 │    not fully confident…"  │
  │                                 │  else → "I don't have    │
  │                                 │    enough information…"   │
  │                                 │                          │
  │                                 │  confidence_tier(answer)  │
  │                                 │─────────────────────────▶│
  │                                 │  "high" / "medium" /     │
  │                                 │  "low confidence"        │
  │                                 │                          │
  │                                 │  qa_page(q, answer,      │
  │                                 │          confidence)      │
  │                                 │─────────────────────────▶│
  │                                 │    badge-success (green)  │
  │                                 │    badge-warning (yellow) │
  │                                 │    badge-error (red)      │
```

The text markers ("don't have enough information", "not fully confident") act as confidence signals that `confidence_tier` maps to UI badges. This is a pure function — no LLM call, no ambiguity.

---

## Doc generation flow

The `generate_docs` flow is a 5-stage DAG with parallel extraction:

```
Wave 0:  scan_files
              │
    ┌─────────┼──────────┐
    ▼         ▼          ▼
Wave 1:  extract_    extract_    extract_
         tasks       agents      flows        ← parallel
    │         │          │
    └─────────┼──────────┘
              ▼
Wave 2:  generate_reference
              │
              ▼
Wave 3:  fact_check  ← uses pool (majority vote)
              │
              ▼
Wave 4:  publish  ← emits PageUpdated
```

Each extraction stage uses `reason` with anti-hallucination guardrails ("Use ONLY what appears in the documentation. Do not invent syntax."). The `generate_reference` stage combines all extractions into a single markdown document. The `fact_check` stage runs the document through the verification pool before publishing.

---

## Fact-check pools

Two pools with different strategies verify generated documentation:

**`fact_check_panel`** — majority vote (2 of 3 must agree):
- 3 `fact_checker` workers run in parallel
- Each gets the same claim and returns YES/NO with explanation
- `strategy: majority` — the consensus answer wins
- 30s timeout with `fact_check_fallback` on expiry

**`fact_check_detail`** — all verdicts collected:
- Same 3 workers, but `strategy: all` preserves every verdict
- Used to show individual worker reasoning in collapsible UI details

The `verify_document` task orchestrates both pools per claim, classifies verdicts (PASS / NEEDS_REVIEW / FAIL), and builds a summary table with per-claim details.

---

## Feature map

Every one of FORGE's 14 primitives is used in this wiki:

| # | Primitive | Where in `server.forge` | Why |
|---|-----------|------------------------|-----|
| 1 | `task` | Lines 133–191, 376–463 | Data ops, LLM search/QA, fact-checking |
| 2 | `pure` | Lines 25–128 | All HTML rendering — determinism boundary enforced |
| 3 | `flow` | Lines 316–368 | Multi-stage doc generation with parallel extraction |
| 4 | `agent` | Lines 198–303, 470–487 | Stateful content, search, and QA agents |
| 5 | `pool` | Lines 400–409 | Parallel fact-checking with majority vote |
| 6 | `system` | Lines 517–523 | Wires agents with `>>` composition |
| 7 | `warden` | Lines 494–511 | Supervises all agents with failure policies |
| 8 | `event` | `shared.forge` lines 19–27 | PageCreated, PageUpdated, PagePublished |
| 9 | `states` | `shared.forge` lines 31–36 | PageLifecycle state machine |
| 10 | `when` | Lines 171–173, 188–190, 327–329, 381–383, 390–392, 415–417 | Confidence-aware branching throughout |
| 11 | `boundary` | Line 1 (`server`), `shared.forge` line 1 (`shared`) | Server/shared compilation boundary |
| 12 | `requires` | Lines 209, 210, 230, 238, 247, 252, 257 | Guard clauses on lifecycle transitions |
| 13 | `endpoint` | Lines 529–574 | 8 HTTP routes serving the web UI |
| 14 | `fn` | Lines 578–581 | Entry point (`fn main`) |

Additional builtins used: `use` (imports), `say` (logging), `emit` (events), `subscribe` (reactive handlers), `transition to` (state changes), `memory persistent` (ACID storage), `classify` (LLM classification), `reason` (LLM reasoning), `data.store`/`data.get`/`data.list` (persistence), `markdown.render` (markdown to HTML).
