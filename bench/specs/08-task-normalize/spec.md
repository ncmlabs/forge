Write a FORGE program that normalises a model-written tag.

Requirements:
- Declare one task that takes one typed text parameter, asks the model for a
  single short tag for that topic, and returns that answer lowercased and with
  surrounding whitespace removed.
- Dispatch on confidence: only a sure answer may be normalised and returned;
  anything else returns the text `unknown`.
- `fn main` normalises the topic `Databases`, then prints two lines: the
  normalised tag after `tag=`, and the result of testing whether that tag
  contains the text `mock`, after `matched=`.

Required stdout, exactly two lines:

tag=mock response
matched=true

Expected FORGE features: task, when.
