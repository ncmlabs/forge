Write a FORGE program that reports two shell checks honestly, one passing and
one failing.
- Declare a task that runs `printf '3 passed'` through `sh -c` as an
  argument-array command. On success it returns `tests: ` plus the captured
  output; on failure it returns `tests failed` without asking the model.
- Declare a second task running `printf 'lint error' >&2; exit 2` the same way.
  On the failure path it returns exactly `lint gate: failed`; it only uses
  captured output on success and never reinterprets stderr as a result.
- In `fn main`, print both results, then a task's summary of the failed lint
  gate, falling back to `no summary` when the model's answer is not sure.
Required stdout, exactly three lines:
tests: 3 passed
lint gate: failed
mock response
Expected FORGE features: command, if, when, task.
