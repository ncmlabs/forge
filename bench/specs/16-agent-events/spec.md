Write a FORGE program with two agents that talk through events.
- Declare an event carrying a thread name for an incoming mention, and a
  second event carrying a thread name for the reply.
- Declare a listener agent subscribed to the mention event, filtered to the
  thread `main`. Its handler says `echo <thread>` and then emits the reply
  event for the same thread; it must never emit the event it subscribes to.
- Declare a second agent subscribed to the reply event, whose handler says
  `replied <thread>`.
- Declare one task that asks the model to summarise the thread `main`, falling
  back to `no summary` unless the answer is sure.
- `fn main` prints `listener ready` and then the task's summary.
Required stdout, exactly two lines:
listener ready
mock response
Expected FORGE features: event, agent, task, when.
