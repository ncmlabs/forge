Write a FORGE program with a flow whose stages form a serial chain.

Requirements:
- Declare one flow that takes a seed and returns text, with three stages, each
  depending on the stage before it: the first asks the model to draft one line
  from the seed, the second asks the model to improve that draft, and the third
  asks the model to shorten the second stage's refinement and returns `final: `
  followed by it, or the text `final unavailable` when that answer is not sure.
- Each stage must interpolate the previous stage's text into its own prompt.
- No stage prints anything; only `fn main` does.
- `fn main` runs the flow once with the seed `tides` and prints the result.

Required stdout, exactly one line:

final: mock response

Expected FORGE features: flow, when.
