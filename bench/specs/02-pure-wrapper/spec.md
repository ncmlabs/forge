Write a FORGE program that turns a model-written title into a bracketed label.

Requirements:
- Declare one deterministic pure function named `bracket` that takes one text
  parameter and returns that text wrapped in square brackets, with no spaces
  added.
- Declare one task named `title_for` that takes one typed text parameter,
  `topic`, asks the model for a short title for that topic, and returns the
  title passed through the pure function.
- Every model answer must be dispatched on confidence: when the answer is sure,
  use it; otherwise use the fallback text `untitled` (also bracketed).
- `fn main` calls the task once with the topic `the sea` and prints the result.

Required stdout, exactly one line:

[mock response]

Expected FORGE features: pure, task, when.
