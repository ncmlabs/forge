Write a FORGE program that gathers notes from two sources in parallel.

Requirements:
- One task takes a source name, asks the model for notes from that source, and
  returns the answer, falling back to `no notes` unless it is sure.
- One flow takes a topic and returns text. It has three stages: a web-gathering
  stage and a papers-gathering stage that do not depend on each other, then a
  synthesis stage that depends on both, asks the model to combine their notes,
  and returns `brief: ` followed by the combined answer, or the text
  `brief unavailable` when that answer is not sure.
- No stage prints anything; only `fn main` does.
- `fn main` runs the flow once for the topic `tides` and prints the result.

Required stdout, exactly one line:

brief: mock response

Expected FORGE features: flow, task, when.
