Write a FORGE program with an event-driven agent that owns a two-state
lifecycle.

Requirements:
- Declare two events carrying a topic: one for work arriving, one for work
  being accepted.
- Declare a two-state lifecycle `ready` and `busy`, each able to move to the
  other.
- Declare a processing agent subscribed to the arrival event, using that
  lifecycle and one text memory field. Its handler may only run from `ready`,
  returning `busy` when that requirement fails; otherwise it stores the topic,
  moves to `busy`, says `processing <topic>` and emits the acceptance event.
- Declare a recorder agent subscribed to the acceptance event whose handler
  says `accepted <topic>`.
- Declare one task that asks the model for a note about a topic, falling back
  to `no note` unless the answer is sure.
- `fn main` prints the task's note for `tides`.

Required stdout, exactly one line:

mock response

Expected FORGE features: agent, states, requires, event, task, when.
