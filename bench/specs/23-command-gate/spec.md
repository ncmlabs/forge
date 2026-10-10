Write a FORGE program where a failing shell command blocks a model decision.
- Declare a task that runs `printf 'compile error' >&2; exit 1` through `sh -c`
  as an argument-array command. Its success flag is a deterministic gate: on
  the success path the task may ask the model for a verdict about the captured
  output and return `gate open: ` plus that verdict, or `gate open: no verdict`
  when the verdict is not sure. On the failure path it must not ask the model
  anything and must return exactly `gate blocked: command failed`.
- A failed command must never be reinterpreted as success or as a verdict.
- Declare a second task asking the model for a one-line banner for a build
  report, falling back to `build report` unless the answer is sure.
- `fn main` prints the banner line first and the gate result second.
Required stdout, exactly two lines:
mock response
gate blocked: command failed
Expected FORGE features: command, if, when, task.
