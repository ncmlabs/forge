Write a FORGE program assembling two agents, two events and a warden into a system.
- Declare a contract with one capability taking a topic and returning text, and two
  events carrying a topic: submitted work and reviewed work.
- Declare an intake agent subscribed to the submission event: its handler says
  `intake <topic>` and emits the review event for the same topic, never its own.
- Declare a reviewer agent subscribed to the review event saying `reviewed <topic>`.
- Declare a warden managing both agents, covering all six failure types (stuck, crash,
  hallucination, contradiction, budget, timeout) with the card's actions.
- Declare a system using both agents as `front` and `back`, composed with `front >> back`,
  plus a task asking the model to summarise a review desk run, else `no summary`.
- `fn main` prints `desk ready` then that summary.
Required stdout, exactly two lines:
desk ready
mock response
Expected FORGE features: contract, system, warden, agent, event, task, when.
