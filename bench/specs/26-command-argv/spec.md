Write a FORGE program that counts files with a command run in an explicit
working directory and a timeout, then asks a model about the count.

Requirements:
- Declare a task that runs `printf '3 files'` through `sh -c` as an
  argument-array command, in the working directory `.`, with a timeout of 30
  seconds. On success it returns `count=` followed by the captured output; on
  failure it returns `count failed` without asking the model anything.
- Declare a second task that takes the count text, asks the model to analyse a
  repository described by that text, and returns the answer, falling back to
  `no analysis` unless the answer is sure.
- `fn main` prints the count line and then the analysis.
- The count text may only reach the model after the command's success flag has
  been checked.

Required stdout, exactly two lines:

count=3 files
mock response

Expected FORGE features: command, if, when, task.
