Write a FORGE program that renders a model answer inside a JSON-looking line.
- Declare one task that takes one typed text parameter, asks the model to quote
  that topic in five words, and returns the answer, or the text `no quote`
  unless the answer is sure.
- `fn main` calls the task once for `fog` and prints one line holding a literal
  opening brace, the double-quoted key `quote`, a colon and a space, the answer
  in double quotes, and a literal closing brace. The braces and quotes must
  come from template-string escapes and interpolation must not corrupt them.
- Then print a second line from a single template string with an escaped
  newline, reading `line one` and then `line two`.
Required stdout, exactly three lines:
{"quote": "mock response"}
line one
line two
Expected FORGE features: task, when.
