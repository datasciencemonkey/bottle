# Which MOG the talk key talks to

The talk key talks to one MOG: the target.
The target is the last MOG that you used.

- When you open a MOG, it becomes the target.
- When you open the card of a MOG, or move to its card window, that MOG becomes the target.
- When you close the target, the MOG that you used before it becomes the target.

Not yet: a click, poke or drag on a MOG does not change the target (TODO(#439)).
To talk to another open MOG now, open its card or move to its card window.
When #439 is done, a click, poke or drag on a MOG makes it the target.
A drag on the macOS panel also needs #83.

The target MOG shows a small triangle under the mascot.
A screen reader reads its name with ", target" at the end.

When you hold the talk key, the target MOG shows "Talking to <name>" for 1.2 seconds.

The MOG that heard your question shows the answer.
This stays true when you use another MOG before the answer comes.
