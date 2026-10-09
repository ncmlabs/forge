Write a FORGE program that runs two shell commands and gates the second's label.
- Declare a preflight task that runs `printf ready` through `sh -c` as an
  argument-array command. On success it returns exactly `preflight ok`; on
  failure `preflight failed`, never asking the model anything.
- Declare a finish task that runs `printf done` the same way and, only on the
  success path, asks the model for a label for a finished run that printed the
  captured text. It returns `finish: ` plus that label, or `finish: unlabeled`
  when the label is not sure; on failure it returns `finish failed` without
  asking the model.
- Both tasks branch on the command's success flag, never on confidence alone,
  before using captured text. `fn main` prints the preflight then the finish.
Required stdout, exactly two lines:
preflight ok
finish: mock response
Expected FORGE features: command, if, when, task.
