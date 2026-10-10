Write a FORGE program with an agent that keeps a small session in memory.
- Declare a three-state lifecycle `idle`, `active` and `done`, with edges from
  each state to the next and from `done` back to `idle`.
- Declare an agent using it with two memory fields, one text and one number, and
  three handlers. A handler taking a topic may only run from `idle`: on failure
  it returns `session busy`, otherwise it stores the topic, resets the number to
  zero, moves to `active` and says `recording <topic>`. A handler with no
  parameters may only run from `active`, else returns `no session`; it adds one
  to the number and says `count <number>`. A third, also requiring `active`,
  moves to `done` and says `closed <topic>`.
- Declare one task asking the model for a headline about a topic, falling back
  to `no headline` unless the answer is sure; the main body prints it for `tides`.
Required stdout, exactly one line:
mock response
Expected FORGE features: agent, states, requires, task, when.
