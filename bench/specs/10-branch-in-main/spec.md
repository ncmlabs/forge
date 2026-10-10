Write a FORGE program that branches inside `fn main` without moving the
decision into a helper.

Requirements:
- Declare one task that takes a number, asks the model for a verdict for that
  score, and returns the answer, falling back to the text `unavailable` unless
  the answer is sure.
- `fn main` binds the score 7, then branches directly in the main body: it
  prints `high` when the score is greater than 10 and `low` otherwise. Do not
  extract this decision into a pure function or a task.
- After the branch, `fn main` prints the task's verdict on its own line.

Required stdout, exactly two lines:

low
mock response

Expected FORGE features: task, when, if.
