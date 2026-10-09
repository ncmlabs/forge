Write a FORGE program where a flow labels a message and summarises it at the
same time.

Requirements:
- Declare one flow that takes a message and returns text, with three stages.
  One stage classifies the message into the labels `Alert`, `Info` and
  `Noise`. A second, independent stage asks the model to summarise the message
  in one line. Both must print nothing.
- The third stage depends on both earlier stages. It asks the model for a
  digest that mentions the classification label and the summary, and returns
  `digest(<label>): ` followed by the digest, or the text
  `digest unavailable` when the digest is not sure.
- The benchmark's provider never returns one of the requested classification
  labels, so the label in the output is the mock's own answer text.
- `fn main` runs the flow for the message `disk is 92% full` and prints it.

Required stdout, exactly one line:

digest(mock response): mock response

Expected FORGE features: flow, when.
