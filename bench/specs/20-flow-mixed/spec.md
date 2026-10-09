Write a FORGE program where a flow calls a model to label and summarise at once.
- Declare one flow taking a message and returning text, with three stages. One
  stage classifies the message into the labels `Alert`, `Info` and `Noise`; a
  second, independent stage asks the model to summarise the message in one
  line. Neither prints anything.
- The third stage depends on both. It asks the model for a digest mentioning
  the classification label and the summary, and returns `digest(<label>): `
  followed by the digest, or `digest unavailable` when the digest is not sure.
- The benchmark's provider never returns one of the requested labels, so the
  label in the output is the mock's own answer text.
- `fn main` runs the flow for the message `disk is 92% full` and prints it.
Required stdout, exactly one line:
digest(mock response): mock response
Expected FORGE features: flow, when.
