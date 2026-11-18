# Ask a question with the talk key

Bottle never sends a spoken question to Genie by itself.
You always have a moment to stop it, or you click to send it.

## The 1.5 second preview

When Bottle is sure that you asked a question, the card shows your question and "Sending in 1.5 s".
The card also shows three buttons:

- **Send now** sends the question at once.
- **Edit** stops the countdown. The question waits until you click **Send now** or **Cancel**.
- **Cancel** stops the question. The card shows "Not sent."

After 1.5 seconds with no cancel, the main agent gets the question and asks Genie.

These actions also stop the question, and nothing is sent:

- Press Esc in the card.
- Press the talk key again.
- Close the MOG.

Not yet: closing only the card does not stop the question (TODO(#374)).

Hold Shift with the talk key to ask on purpose. You still get the 1.5 second preview.

## The Ask Genie chip

When Bottle is not sure that you asked a question, the card shows your text and an **Ask Genie** chip.
Nothing goes to Genie until you click the chip.
The chip goes away after 2 minutes, or when 3 newer chips show.

In a meeting session, every question gets the chip. A meeting question never sends by itself.

## A warning instead of a chip

If your words sounded like an instruction to Bottle, the card shows this warning:
"Warning: this sounded like an instruction to Bottle. Nothing was sent."
Bottle sends nothing for that question.

Not yet: Edit does not open a text box (TODO(#526)).
