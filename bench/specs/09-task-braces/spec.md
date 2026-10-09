Write a FORGE program that renders a model answer inside a JSON-looking line.

Requirements:
- Declare one task that takes one typed text parameter, asks the model to quote
  that topic in five words, and returns the answer, falling back to the text
  `no quote` unless the answer is sure.
- `fn main` calls the task once for the topic `fog`, then prints one line that
  contains a literal opening brace, the double-quoted key `quote`, a colon and
  a space, the answer in double quotes, and a literal closing brace. The braces
  and the quotes must come from escapes in a template string, not from
  concatenation, and the literal text must not be corrupted by interpolation.
- Then print a second line built from a single template string containing an
  escaped newline, reading `line one` and, on the following line, `line two`.

Required stdout, exactly three lines:

{"quote": "mock response"}
line one
line two

Expected FORGE features: task, when.
