Write a FORGE program that finds the largest number in a list and asks a model
for a headline about it.

Requirements:
- Declare one deterministic pure function that takes a dynamic array of numbers
  and returns the largest one, walking the array with a loop and keeping a
  running best value. Do not call a model from it.
- Declare one task that takes the peak as its only parameter, asks the model
  for a one-line headline about a peak of that value, and returns the answer,
  falling back to the text `no headline` unless the answer is sure.
- `fn main` finds the largest of 4, 9 and 2, then prints the peak and the
  headline on one line, separated by a space.

Required stdout, exactly one line:

peak=9 headline=mock response

Expected FORGE features: pure, for, if, task, when.
