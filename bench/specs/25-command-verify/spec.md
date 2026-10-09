Write a FORGE program that reports two shell checks honestly: one that passes
and one that fails.

Requirements:
- Declare a task that runs `printf '3 passed'` through `sh -c` as an
  argument-array command. On success it returns `tests: ` followed by the
  captured output; on failure it returns `tests failed` without asking the
  model anything.
- Declare a second task that runs `printf 'lint error' >&2; exit 2` the same
  way. It returns exactly `lint gate: failed` on the failure path, regardless
  of what the command wrote, and only uses captured output on the success path.
- In `fn main`, print both results, then ask the model once for a summary of a
  run whose lint gate failed and print it, falling back to `no summary` when
  that answer is not sure.
- The failed command must not be reinterpreted: its stderr is a failure signal,
  never a result.

Required stdout, exactly three lines:

tests: 3 passed
lint gate: failed
mock response

Expected FORGE features: command, if, when, task.
