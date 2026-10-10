Write a FORGE program that summarises a topic and reports how long the summary is.
- Declare one task that takes one typed text parameter, asks the model to
  summarise that topic in one sentence, and dispatches the answer on
  confidence: the answer when sure, `unclear summary` when unsure, and
  `no summary` otherwise.
- Declare one deterministic pure function returning `long summary` for a number
  greater than 20 and `short summary` otherwise; keep that decision out of the
  main body.
- In `fn main`, call the task once for `tides`, measure the returned text's
  length in characters, and print the label, the length in parentheses and the
  word `chars`.
Required stdout, exactly one line:
short summary (13 chars)
Expected FORGE features: task, pure, when, if.
