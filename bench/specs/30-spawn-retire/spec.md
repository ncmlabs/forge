Write a FORGE program where a foreman agent spawns and later retires a specialist.
- Declare a specialist agent that says `specialist up` when it starts and declares
  an explicit failure policy: after being stuck for 3 turns it escalates to a
  human. Without that policy `forge check` warns.
- Declare a foreman agent with one text memory field. One handler takes a topic,
  spawns the specialist under the name `spec_<topic>`, stores the returned name
  and says `spawned <name>`. A second handler looks up the existing agent named
  `spec_<topic>` and retires it, saying `retired`.
- Declare one task asking the model to plan work for a topic, falling back to
  `no plan` unless the answer is sure; the main body prints `foreman ready` then it.
Required stdout, exactly two lines:
foreman ready
mock response
Expected FORGE features: spawn, agent, task, when.
