# Confidence, Verification, and Hallucination Control in FORGE

**Date:** April 10, 2026  
**Author:** Codex analysis for NCM Labs  
**Primary audience:** FORGE language/runtime design and Phase 2 implementation work  
**Related FORGE issues:** #7, #165, #189-194, #198

## Abstract

FORGE already has an important semantic advantage over most agent frameworks: it treats oracle-derived outputs as structurally uncertain and forces explicit confidence-aware dispatch before those values can be returned. That is a meaningful language-level safeguard. It is also not enough.

The central claim of this paper is that **raw confidence should not be treated as proof in FORGE**. The current `sure / unsure / unreliable` model is useful for control flow, but it is not a trustworthy basis for factual correctness or side-effect authorization. In the current runtime, `reason` and `classify` confidence are derived from a heuristic over the model's wording rather than a calibrated probability of truth. That makes the existing system directionally useful but operationally weak for high-trust automation.

The recommended evolution path is a layered model:

- **Uncertainty** remains a core language feature.
- **Calibration** becomes a measured property of confidence sources, not an assumption.
- **Verification** becomes a runtime discipline over typed claims and evidence.
- **Actionability** becomes a policy gate for side effects.

This paper argues that FORGE should not pursue a generic “hallucination detector” as its primary defense. Instead, it should become the language/runtime where **unverified intelligence cannot silently masquerade as truth**. That is both the correct design direction and a plausible long-term moat.

## 1. The Problem FORGE Is Actually Solving

FORGE is not just trying to make LLM calls easier. It is trying to make **oracle-augmented computation programmable**. That means the core problem is not only generation quality. It is control:

- How should a program represent knowledge that may be wrong?
- How should uncertain results be allowed to influence state?
- When should a system trust an oracle enough to act?
- How should a system recover when model outputs are contradicted by environment truth?

For a development automation pipeline, the risk is not merely “the model said something false.” The real risk is:

- wrong file references,
- fabricated APIs or symbols,
- false claims of task completion,
- silent regression introduction,
- or unsafe repo mutations driven by unsupported model claims.

This is why hallucination handling in FORGE cannot be treated as a UI or product concern. It is a language and runtime concern.

## 2. What Current FORGE Already Gets Right

The current repository already establishes several important reliability primitives.

### 2.1 Uncertainty is explicit

The FORGE reference defines uncertain value handling as a core language invariant: oracle outputs are tainted, and tainted values cannot be given directly without passing through `when` or `match` ([forge-reference.md](/Users/claudiu/Work/ncmlabs/forge/docs/forge-reference.md)). This is a better default than most Python or TypeScript agent libraries, which usually rely on developer discipline rather than compiler rules.

### 2.2 Deterministic and stochastic logic are separated

`pure` functions remain deterministic and cannot invoke oracle operations. This is the correct boundary. It means FORGE already has a principled separation between:

- code that computes,
- and code that asks.

That boundary is one of the strongest ideas in the language today.

### 2.3 Supervision is already part of the model

FORGE wardens already define failure types including `hallucination`, `stuck`, `timeout`, `budget`, and `crash`, plus escalation ladders and scoped responses ([forge-reference.md](/Users/claudiu/Work/ncmlabs/forge/docs/forge-reference.md)). That is exactly the right place to attach future contradiction and verification failures.

### 2.4 Confidence is first-class

Every runtime value carries confidence metadata via `ConfidentValue` in [`confidence.rs`](/Users/claudiu/Work/ncmlabs/forge/src/runtime/confidence.rs). The predicates are simple and explicit:

- `sure`: confidence `>= 0.8`
- `sure_above(threshold)`: confidence `>= threshold`
- `unsure`: `0.5 <= confidence < 0.8`
- `unreliable`: `< 0.5`
- `conflicted`: currently tied to low-consensus agreement

This gives the language a common epistemic currency. That is good. The remaining question is whether the currency is well-calibrated enough to trust.

## 3. Why Current `sure / unsure` Is Not Reliable Enough

This is the most important design critique in the paper.

### 3.1 What `sure` means today

Today, `when result.sure(above: 0.85)` means only this:

- the `ConfidentValue.confidence` field is at least `0.85`, and
- that field is whatever confidence the runtime attached to the upstream oracle result.

For `reason` and `classify`, the runtime path is in [`executor.rs`](/Users/claudiu/Work/ncmlabs/forge/src/runtime/executor.rs). Both operations call `CompletionResponse::estimate_confidence()` from [`llm/mod.rs`](/Users/claudiu/Work/ncmlabs/forge/src/llm/mod.rs), then wrap the result with `ConfidentValue::from_llm(...)`.

### 3.2 Where the score comes from

The current implementation is explicitly heuristic:

- it starts around `0.85`,
- then subtracts for hedge phrases such as “I think,” “possibly,” “unclear,” and “it depends,”
- and bottoms out around `0.3`.

That is useful as a **linguistic confidence hint**. It is not a calibrated probability of correctness.

This means the current system can easily mark fluent but wrong answers as `sure`, especially when the model states them assertively.

### 3.3 Why this matters

There are three distinct things that are currently too close together:

- **uncertainty acknowledgment**
- **confidence calibration**
- **factual verification**

They are not the same.

`when result.sure` is a reasonable branch primitive. It is not evidence that the underlying claim is true. If FORGE treats it as more than that, the language will have a semantic honesty story at compile time but a weak truth story at runtime.

## 4. Research Synthesis

This section summarizes the most relevant external evidence and the FORGE design implication from each.

### 4.1 Models do carry useful uncertainty signals, but not enough by themselves

Kadavath et al., **Language Models (Mostly) Know What They Know** ([arXiv:2207.05221](https://arxiv.org/abs/2207.05221)), show that models can often evaluate the probability that a proposed answer is correct and can display meaningful calibration under the right setup. That is encouraging for FORGE because it supports keeping uncertainty first-class rather than pretending every output is equally trustworthy.

The limitation is equally important: the paper does not justify treating raw self-confidence as proof. Calibration is task-dependent and format-dependent. That supports a FORGE design where model confidence is useful for:

- branch selection,
- abstention,
- escalation,
- and routing,

but not as a substitute for verification.

### 4.2 Better uncertainty estimation requires semantic, not just lexical, signals

Kuhn, Gal, and Farquhar, **Semantic Uncertainty: Linguistic Invariances for Uncertainty Estimation in Natural Language Generation** ([arXiv:2302.09664](https://arxiv.org/abs/2302.09664)), argue that uncertainty estimation in natural language is hard because different surface forms may encode the same meaning. Their semantic-entropy approach is more predictive than simpler baselines.

FORGE implication:

- the current hedge-phrase heuristic in `estimate_confidence()` is too shallow,
- future confidence sources should include richer signals than wording alone,
- especially for coding and retrieval-backed workflows where contradictions matter more than phrasing.

### 4.3 Self-correction exists, but it should not be the only safety layer

Liu et al., **Large Language Models have Intrinsic Self-Correction Ability** ([arXiv:2406.15673](https://arxiv.org/abs/2406.15673)), argue that intrinsic self-correction can work under certain conditions. That is relevant because FORGE already has wardens and could use retries or nudges as part of recovery.

The design implication is narrow:

- self-correction is a useful repair tactic,
- but not a truth guarantee,
- and not a replacement for external validation.

In FORGE, retries and nudges belong in the warden ladder, not in the trust model itself.

### 4.4 Coding hallucinations are not just factual mistakes

Agarwal et al., **CodeMirage: Hallucinations in Code Generated by Large Language Models** ([arXiv:2408.08333](https://arxiv.org/abs/2408.08333)), show that code hallucinations include syntax errors, logical errors, robustness problems, and security issues. This is a useful corrective against a too-narrow definition of hallucination.

FORGE implication:

- coding workflows need more than “is this factually grounded?”
- they also need execution-backed validation:
  - compile,
  - test,
  - diff inspection,
  - API existence checks,
  - and policy checks.

### 4.5 Retrieval-backed hallucination detection works best when it is evidence-conditioned

Yu et al., **ReEval** ([arXiv:2310.12516](https://arxiv.org/abs/2310.12516)), and Lee and Yu, **REFIND** ([arXiv:2502.13622](https://arxiv.org/abs/2502.13622)), both support the broader idea that hallucination evaluation improves when the model output is assessed against evidence rather than judged in the abstract.

FORGE implication:

- “hallucination detection” should be operationalized as **claim vs evidence vs contradiction**,
- not as a standalone model intuition score.

This aligns directly with the direction of `session`, `AgentResult`, and validator stages in Phase 2.

## 5. Recommended FORGE Reliability Model

FORGE should evolve toward a layered reliability model with four distinct concepts.

### 5.1 Uncertainty

This is the language-level concept FORGE already has.

- Oracle outputs are provisional.
- The programmer must acknowledge uncertainty through control flow.
- `when` remains the correct construct for this.

This layer should stay language-native.

### 5.2 Calibration

Calibration answers:

- how predictive is this confidence signal of actual correctness?

FORGE should stop assuming that a numeric score is meaningful just because it exists. Confidence needs a source and observed calibration behavior:

- provider-native probabilities or logprobs where available,
- consensus agreement,
- retrieval similarity,
- test pass rates,
- verifier agreement,
- or weaker heuristics when nothing better exists.

Every confidence source should be treated as a tagged source, not a universal truth scale.

### 5.3 Verification

Verification answers:

- what evidence supports or contradicts the claim?

FORGE should add a runtime claim-verification model:

- `Claim`
- `Evidence`
- `VerificationResult`
- `Contradiction`

This should be the operational definition of hallucination:

- a claim contradicted by trusted validation,
- or a claim used beyond its allowed trust level without required validation.

### 5.4 Actionability

Actionability answers:

- may this result drive side effects?

This should be policy-based, not confidence-based.

Examples:

- a high-confidence plan can still be non-actionable,
- a low-confidence patch proposal can still be explored,
- a commit, push, PR, or merge should require verified preconditions,
- external side effects should require stronger gates than internal reasoning.

## 6. Applying the Model to the README Example

The README example is still valid:

```forge
task classify_intent
  needs message: Text
  gives Intent

  do
    result = classify message into ["buy", "support", "cancel", "other"]

    when result.sure(above: 0.85)  -> give result
    when result.sure               -> give result with flag("low-confidence")
    when result.unsure             -> give ask_for_clarification(message)
    else                           -> escalate to human
```

This is good language design because it forces the programmer to acknowledge uncertainty. It should stay.

What changes under the proposed model is the meaning of the downstream contract:

- `result.sure(above: 0.85)` is enough to choose a control-flow branch,
- it is not enough to certify that the user truly intends cancellation,
- and it is definitely not enough to authorize an irreversible cancellation.

The missing layers are:

1. **Verification**
   - account state exists,
   - user identity is confirmed,
   - policy allows cancellation,
   - surrounding evidence does not contradict the intent.

2. **Actionability**
   - once those checks pass, a downstream operation becomes allowed.

So the evolved design does not replace `when result.sure`. It clarifies its role:

- confidence controls branching,
- verification controls trust,
- policy controls action.

## 7. Operational Model for Coding Pipelines

The development automation pipeline should be the first serious implementation target.

### 7.1 Typed claims

`AgentResult` should carry structured claims such as:

- issue interpretation,
- target files,
- symbol assumptions,
- patch intent,
- tests claimed to pass,
- completion claim,
- proposed side effects.

### 7.2 Validator stages

Before any high-risk step, the runtime should run:

1. schema validation
2. reference validation
3. environment validation
4. execution validation
5. policy validation

These should produce:

- `verified`
- `insufficient`
- `contradicted`
- `error`

### 7.3 Warden integration

Wardens already know how to respond to `hallucination`, `stuck`, `timeout`, `budget`, and `crash`. The runtime should emit `hallucination` when:

- the agent claims files or symbols that do not exist,
- the agent claims completion that tests or checks refute,
- the agent crosses a trust boundary without required verification,
- repeated high-confidence contradictions appear in one session.

Suggested coding-agent default ladder:

- first contradiction: `nudge, self`
- repeated contradiction: `restart, self`
- persistent contradiction in the same task: `replace, downstream`
- contradiction on merge or external side effect: `escalate`

### 7.4 Worktree isolation

The planned `isolate worktree` direction in #194 is important because it reduces blast radius. Verification becomes much more credible when it runs in a bounded sandbox with known repo state.

## 8. What Should Be Language-Native vs Runtime-Native

FORGE should be careful not to overfit the language syntax to one specific product workflow.

### Language-native

- uncertainty as a first-class concept
- deterministic vs stochastic boundary
- confidence-aware branch constructs
- supervision hooks
- event emission for contradiction or escalation
- trust-boundary rules around oracle-origin values

### Runtime-native

- claim and evidence schemas
- validator orchestration
- calibration measurement
- provider-specific confidence adapters
- session-level contradiction tracking
- policy evaluation for side effects

### App-native

- Slack, TUI, browser, and IDE presentation
- approval UIs
- evidence browsing
- operator dashboards
- human override flows

This keeps the language principled while still allowing the runtime and host product to become much more reliable.

## 9. Implementation Priorities

The shortest path to a meaningful upgrade is:

1. keep current `sure / unsure` semantics, but document them honestly as branch primitives
2. add explicit confidence-source tracking and calibration telemetry
3. extend `AgentResult` with claims, verification, and provenance fields
4. add validator stages to `session` completion
5. emit contradiction-driven `hallucination` events into wardens
6. gate commit, PR, merge, and external side effects on verification and policy rather than confidence alone

This sequencing matches the existing Phase 2 issue direction:

- `session` as the unit of long-running coding work
- `AgentResult` as the typed result contract
- worktree isolation as the safety boundary
- adapters as external model/CLI bridges
- wardens as supervision logic

## 10. Final Opinion

FORGE already has a better language story about uncertainty than most of the market. That is real. The current weakness is that the runtime confidence story is still too heuristic to bear the weight of truth or safety.

The correct design move is not to abandon confidence-aware language semantics. It is to make them more honest about what they do:

- `sure` means “the system has a confidence signal above a threshold”
- not “this claim is true”

The moat appears if FORGE becomes the system where:

- uncertainty is explicit,
- confidence is measured and source-aware,
- claims are validated against evidence,
- contradictions become runtime events,
- and unsafe actions are blocked unless trust has been earned.

That is stronger than “hallucination detection.” It is a programming model for **trust progression**.

## References

- FORGE language reference: [docs/forge-reference.md](/Users/claudiu/Work/ncmlabs/forge/docs/forge-reference.md)
- Current confidence implementation: [src/runtime/confidence.rs](/Users/claudiu/Work/ncmlabs/forge/src/runtime/confidence.rs)
- Current LLM confidence heuristic: [src/llm/mod.rs](/Users/claudiu/Work/ncmlabs/forge/src/llm/mod.rs)
- Current `reason` / `classify` execution path: [src/runtime/executor.rs](/Users/claudiu/Work/ncmlabs/forge/src/runtime/executor.rs)
- Kadavath et al., *Language Models (Mostly) Know What They Know*: https://arxiv.org/abs/2207.05221
- Kuhn, Gal, Farquhar, *Semantic Uncertainty: Linguistic Invariances for Uncertainty Estimation in Natural Language Generation*: https://arxiv.org/abs/2302.09664
- Liu et al., *Large Language Models have Intrinsic Self-Correction Ability*: https://arxiv.org/abs/2406.15673
- Agarwal et al., *CodeMirage: Hallucinations in Code Generated by Large Language Models*: https://arxiv.org/abs/2408.08333
- Yu et al., *ReEval: Automatic Hallucination Evaluation for Retrieval-Augmented Large Language Models via Transferable Adversarial Attacks*: https://arxiv.org/abs/2310.12516
- Lee and Yu, *REFIND at SemEval-2025 Task 3: Retrieval-Augmented Factuality Hallucination Detection in Large Language Models*: https://arxiv.org/abs/2502.13622
