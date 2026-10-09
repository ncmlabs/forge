Write a FORGE program with an agent that keeps a small session in memory.

Requirements:
- Declare a three-state lifecycle: `idle`, `active` and `done`, with the edges
  idle to active, active to done and done back to idle.
- Declare an agent that uses it and has two memory fields, one text and one
  number, plus three handlers. A handler taking a topic may only run from
  `idle`: on failure it returns `session busy`; otherwise it stores the topic,
  resets the number to zero, moves to `active` and says `recording <topic>`.
  A handler with no parameters may only run from `active`, otherwise returns
  `no session`; it adds one to the number field and says `count <number>`. A
  third handler with no parameters also requires `active`, otherwise returns
  `no session`; it moves to `done` and says `closed <topic>`.
- Declare one task that asks the model for a headline about a topic, falling
  back to `no headline` unless the answer is sure.
- `fn main` prints the task's headline for `tides`.

Required stdout, exactly one line:

mock response

Expected FORGE features: agent, states, requires, task, when.
