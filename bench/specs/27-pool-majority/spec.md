Write a FORGE program that fact-checks a claim with three workers and a
majority strategy.

Requirements:
- Declare one task that takes a claim, asks the model whether the claim is
  true, and returns the answer, falling back to `unverified` unless the answer
  is sure.
- Declare one pool of exactly three workers running that task, using the
  majority strategy and a 15 second timeout.
- Declare one deterministic pure function that takes the pool's verdict text
  and returns `verdict: ` followed by it.
- `fn main` sends the claim `the sky is blue` to the pool with the label
  `check`, passes the returned verdict through the pure function and prints it.

Required stdout, exactly one line:

verdict: mock response

Expected FORGE features: pool, task, pure, when.
