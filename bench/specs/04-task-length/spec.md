Write a FORGE program that summarises a topic and reports how long the summary
is.

Requirements:
- Declare one task that takes one typed text parameter, asks the model to
  summarise that topic in one sentence, and dispatches the answer on
  confidence: return the answer when it is sure, the text `unclear summary`
  when it is unsure, and the text `no summary` otherwise.
- Declare one deterministic pure function that takes a number and returns
  `long summary` when it is greater than 20, and `short summary` otherwise.
  Keep the decision out of `fn main`.
- In `fn main`, call the task once for the topic `tides`, measure the length of
  the returned text in characters, and print the label, a space, the length in
  parentheses, and the word `chars`.

Required stdout, exactly one line:

short summary (13 chars)

Expected FORGE features: task, pure, when, if.
