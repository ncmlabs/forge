Write a FORGE program that answers a question for a named audience.

Requirements:
- Declare one task that takes two typed text parameters: the question and the
  audience.
- The task asks the model to answer the question for that audience in one
  sentence. The prompt must interpolate both parameters.
- Dispatch the model's answer on confidence: return it when it is sure,
  otherwise return the text `no answer available`.
- `fn main` binds the question `What is fog?` and the audience `beginner` to
  names, calls the task once, and then prints the question line first and the
  answer line second.

Required stdout, exactly two lines:

question: What is fog?
answer: mock response

Expected FORGE features: task, when.
