Write a FORGE program where an agent refuses work until something was approved.
- Declare an agent with one number memory field counting approvals.
- Give it two handlers with no event subscription. The first takes no
  parameters, adds one to the approval count and says `approvals <count>`. The
  second takes a topic and requires that the approval count is greater than
  zero, returning `nothing approved` when that requirement fails; otherwise it
  says `publishing <topic>`.
- Declare one task that asks the model to draft one line about a topic, falling
  back to `no draft` unless the answer is sure.
- `fn main` prints `gate ready` and then the task's draft for `tides`.
Required stdout, exactly two lines:
gate ready
mock response
Expected FORGE features: agent, requires, task, when.
