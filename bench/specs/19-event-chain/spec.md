Write a FORGE program with three agents chained through distinct events.

Requirements:
- Declare three events, each carrying a topic: one for intake, one for routing
  and one for completion.
- Declare a router agent subscribed to the intake event. Its handler says
  `routing <topic>` and emits the routing event for the same topic. It must
  never emit the event it subscribes to.
- Declare a finisher agent subscribed to the routing event. Its handler says
  `finishing <topic>` and emits the completion event.
- Declare a logger agent subscribed to the completion event, whose handler says
  `done <topic>`.
- Declare one task that asks the model for an audit line about a topic, falling
  back to `no audit` unless the answer is sure.
- `fn main` prints the task's audit line for `tides`.

Required stdout, exactly one line:

mock response

Expected FORGE features: event, agent, task, when.
