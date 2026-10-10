Write a FORGE program with an event-driven agent owning a two-state lifecycle.
- Declare two events carrying a topic: work arriving, and work being accepted.
- Declare a two-state lifecycle `ready` and `busy`, each able to move to the
  other.
- Declare a processing agent subscribed to the arrival event, using that
  lifecycle and one text memory field. Its handler may only run from `ready`,
  returning `busy` when that requirement fails; otherwise it stores the topic,
  moves to `busy`, says `processing <topic>` and emits the acceptance event.
- Declare a recorder agent subscribed to the acceptance event whose handler
  says `accepted <topic>`, and one task asking the model for a note about a
  topic, falling back to `no note` unless the answer is sure.
- `fn main` prints that task's note for `tides`.
Required stdout, exactly one line:
mock response
Expected FORGE features: agent, states, requires, event, task, when.
