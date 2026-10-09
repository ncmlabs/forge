Write a FORGE program that counts files with a command run in an explicit
working directory and a timeout, then asks a model about the outcome.
- Declare a task that runs `sh -c "printf 'no such directory' >&2; exit 4"` as
  an argument-array command, in the working directory `.`, with a 30 second
  timeout. On success it returns `count=` plus the captured output; on failure
  exactly `count failed`, without asking the model anything.
- Declare a second task taking the count text that asks the model to analyse a
  repository described by it, returning the answer or `no analysis` unless sure.
- `fn main` prints the count line then the analysis. The recorded run's command
  exits non-zero; the count text may only reach the model after the success flag
  is checked.
Required stdout, exactly two lines:
count failed
mock response
Expected FORGE features: command, if, when, task.
