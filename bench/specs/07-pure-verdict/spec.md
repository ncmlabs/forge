Write a FORGE program that decides whether a review is a priority and asks a
model for a note about it.

Requirements:
- Declare one deterministic pure function that takes a dynamic array of text
  tags and one text tag to look for, and returns `priority` when the array
  contains that tag and `normal` otherwise.
- Declare one task that takes the verdict as its only parameter, asks the model
  for one line about a review with that verdict, and returns the answer,
  falling back to the text `no note` unless the answer is sure.
- `fn main` checks the tags `urgent` and `backend` for `urgent`, and prints the
  verdict, a colon and a space, and the task's note.

Required stdout, exactly one line:

priority: mock response

Expected FORGE features: pure, if, task, when.
