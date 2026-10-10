Write a FORGE program with an agent that guards its work with a state machine.
- Declare a two-state lifecycle named `Phase` with the states `idle` and
  `working`, where each state can move to the other.
- Declare an agent using that lifecycle with one text memory field and one
  handler taking a topic. The handler must first require that the lifecycle is
  `idle`, returning the text `busy` on failure; then it stores the topic in
  memory, moves the lifecycle to `working` and says `working on <topic>`. No
  other handler may move the lifecycle.
- Declare one task that asks the model to plan the work for a topic and returns
  the answer, or `no plan` when it is not sure.
- `fn main` prints `worker ready` and then the task's plan for `tides`.
Required stdout, exactly two lines:
worker ready
mock response
Expected FORGE features: agent, states, requires, task, when.
