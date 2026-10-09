Write a FORGE program that runs two shell commands and only labels the second
one when it actually succeeded.

Requirements:
- Declare a preflight task that runs `printf ready` through `sh -c` as an
  argument-array command. On success it returns exactly `preflight ok`; on
  failure it returns `preflight failed` and never asks the model anything.
- Declare a finish task that runs `printf done` through `sh -c` and, only on
  the success path, asks the model for a label for a finished run whose output
  was the captured text. It returns `finish: ` followed by that label, or
  `finish: unlabeled` when the label is not sure. On the failure path it
  returns `finish failed` without asking the model.
- Both tasks must branch on the command's success flag, not on its confidence
  alone, before using any captured text.
- `fn main` prints the preflight result and then the finish result.

Required stdout, exactly two lines:

preflight ok
finish: mock response

Expected FORGE features: command, if, when, task.
