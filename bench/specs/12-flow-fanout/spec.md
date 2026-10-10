Write a FORGE program that reviews a topic from three angles at once.

Requirements:
- One flow takes a topic and returns text, with four stages. The facts, risks
  and uses stages each ask the model a different question about the topic and
  print nothing. The assembly stage depends on all three, asks the model to
  summarise them, and returns `review: ` followed by that summary, or the text
  `review unavailable` when the summary is not sure.
- No stage prints anything; only `fn main` does.
- `fn main` runs the flow once for the topic `tides` and prints the result.

Required stdout, exactly one line:

review: mock response

Expected FORGE features: flow, when.
