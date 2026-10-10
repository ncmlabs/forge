Write a FORGE program that labels a workload from a list of scores and asks a
model for a note about it.

Requirements:
- Declare one deterministic pure function that takes a dynamic array of numbers
  and returns a text label. Sum the numbers with a loop, then return:
  `heavy` when the sum is greater than 10, `medium` when it is greater than 5,
  and `light` otherwise.
- Declare one task that takes the label as its only parameter, asks the model
  for one short note about a workload of that kind, and returns the answer,
  falling back to the text `no note available` unless the answer is sure.
- `fn main` computes the label for the scores 1, 2 and 3, then prints the label,
  a colon and a space, and the task's note, all on one line.

Required stdout, exactly one line:

medium: mock response

Expected FORGE features: pure, task, for, if, when.
