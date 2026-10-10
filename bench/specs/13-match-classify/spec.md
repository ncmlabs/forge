Write a FORGE program that routes a message by asking a model to classify it.

Requirements:
- Declare one task that takes a message, classifies it into exactly the three
  labels `Buy`, `Support` and `Other`, and returns routing text from a pattern
  match over the classification: `sales` for Buy, `helpdesk` for Support and
  `triage` for anything else. Include a catch-all arm.
- The benchmark's provider answers every classification with a label that is
  not in the requested list, so the catch-all arm is the one that runs.
- `fn main` routes the message `I need help with my order` and prints the
  routing text.

Required stdout, exactly one line:

triage

Expected FORGE features: task, match.
