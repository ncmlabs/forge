Write a FORGE program with an agent that learns facts and recalls them.
- Declare an agent with a knowledge store rooted at
  `.forge-knowledge/librarian` holding at most 5000 entries.
- Give it one handler taking a fact that learns it under the category `FACTS`,
  interpolating the fact into the learned text.
- Give it a second handler taking a question that recalls the most relevant
  prior knowledge for it and returns it when sure; otherwise it escalates to a
  human.
- Declare one task that asks the model to answer a question, falling back to
  `no answer` unless the answer is sure, and print `librarian ready` followed
  by that task's answer for the question `what is fog` from the main body.
Required stdout, exactly two lines:
librarian ready
mock response
Expected FORGE features: knowledge, agent, task, when.
