Write a FORGE program that runs two shell commands and reports a failed preflight.
- Declare a preflight task that runs `sh -c "exit 3"` as an argument-array
  command. It returns exactly `preflight ok` when the command's success flag is
  true and exactly `preflight failed` when it is false; the failure path must
  not ask the model anything.
- Declare a finish task that runs `printf done` the same way. Only on the
  success path does it ask the model for a label for a finished run that printed
  the captured text, returning `finish: ` plus that label, or `finish:
  unlabeled` when the label is not sure; on failure it returns `finish failed`.
- Both tasks branch on the command's success flag, never on confidence alone,
  before using captured text. The recorded preflight command exits non-zero.
Required stdout, exactly two lines:
preflight failed
finish: mock response
Expected FORGE features: command, if, when, task.
