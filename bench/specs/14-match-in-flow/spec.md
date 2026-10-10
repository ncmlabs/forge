Write a FORGE program that classifies a message inside a flow and routes it in
a later stage.

Requirements:
- Declare one flow that takes a message and returns text, with two stages. The
  first classifies the message into the labels `Bug`, `Feature` and
  `Question`; the second declares the first stage's classification as a
  dependency and pattern-matches it, returning `routed to engineering` for
  Bug, `routed to product` for Feature and `routed to support` for anything
  else. The first stage must not print anything.
- The benchmark's provider answers every classification with a label that is
  not in the requested list, so the catch-all arm is the one that runs.
- `fn main` runs the flow for the message `the app crashes on launch`.

Required stdout, exactly one line:

routed to support

Expected FORGE features: flow, match.
