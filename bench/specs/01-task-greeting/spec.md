Write a FORGE program that greets a person by asking a model for the greeting.

Requirements:
- Declare one task named `greet` that takes one typed text parameter, `name`.
- The task asks the model for a one-sentence greeting for that name, using a
  template string that interpolates the parameter into the prompt.
- The task must dispatch the model's answer on its confidence: when the answer
  is sure, return it unchanged; otherwise return the text `no greeting`.
- `fn main` calls the task once with the name `Ada` and prints exactly:

greeting: mock response

For example, a correct program's stdout is the single line above, with no
leading or trailing blank lines.

Expected FORGE features: task, when.
