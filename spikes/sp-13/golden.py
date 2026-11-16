#!/usr/bin/env python3
"""SP-13 golden set: 500 router utterances from fixed templates (no LLM), each with its expected intent.

Deterministic: the same code gives the same file. Regenerate with
  uv run --no-project spikes/sp-13/golden.py --out spikes/sp-13/golden.json
All text is synthetic. Intents: Appendix D enum. Dictation follows RTR-01b (a PTT or hands-free
utterance that is not a question, a memory question or a command).
"""
import argparse
import itertools
import json
import random

SEED = 1301
COUNTS = {"question": 150, "memory_question": 70, "context": 60, "command": 81, "dictation": 100, "noise": 39}

METRICS = ["revenue", "gross margin", "churn rate", "average order value", "net retention", "bookings", "pipeline"]
REGIONS = ["EMEA", "APAC", "North America", "the west region", "LATAM"]
PERIODS = ["last quarter", "this year", "in March", "last week", "year to date"]
DIMS = ["product line", "channel", "segment", "sales rep"]
ENTITIES = ["new customers", "support tickets", "orders", "trial signups"]
TOPICS = ["pricing", "the renewal plan", "the Q3 forecast", "the onboarding flow", "the partner program"]
PEOPLE = ["Priya", "Marco", "Dana", "Kenji"]
MONTHS = ["February", "April", "July", "October"]
DAYS = ["Monday", "Thursday", "Friday"]
WORKSPACES = ["sales", "finance", "marketing"]

QUESTION_T = [
    "what was {metric} in {region} {period}",
    "show me {metric} by {dim} {period}",
    "which {dim} had the highest {metric} {period}",
    "how many {entity} did we get in {region} {period}",
    "compare {metric} for {region} {period} with the year before",
]
MEMORY_T = [
    "what did we decide about {topic} earlier",
    "what did {person} say about {topic} in the last meeting",
    "remind me what you told me about {metric} before",
    "did we already ask about {topic} today",
    "what was the answer you gave me on {metric} for {region}",
]
CONTEXT_T = [
    "for context we only care about {region} in this discussion",
    "just so you know our fiscal year starts in {month}",
    "keep in mind that {metric} excludes refunds for us",
    "background for later {topic} started in {month}",
    "for context the {dim} numbers we look at are in US dollars",
    "fyi {person} owns {topic} on our side",
    "assume {region} for the next few questions",
]
COMMAND_T = {
    "switch_workspace_once": ["{pre}ask the {ws} workspace for this one question",
                              "{pre}just this once use the {ws} workspace",
                              "{pre}for this question only switch to {ws}"],
    "switch_workspace_session": ["{pre}switch to the {ws} workspace",
                                 "{pre}use the {ws} workspace for the rest of this session",
                                 "{pre}move this whole session over to {ws}"],
    "reset_workspace": ["{pre}go back to my home workspace", "{pre}reset the workspace",
                        "{pre}switch back to the default workspace"],
    "new_topic": ["{pre}new topic", "{pre}let's start a new topic", "{pre}change of subject start fresh"],
    "cancel": ["{pre}cancel that", "{pre}never mind cancel the question", "{pre}cancel the last ask"],
    "stop_recording": ["{pre}stop recording", "{pre}stop the recording now", "{pre}end the recording"],
    "pause": ["{pre}pause", "{pre}pause listening for a moment", "{pre}pause the recording"],
    "resume": ["{pre}resume", "{pre}resume listening", "{pre}okay resume the recording"],
    "forget_session": ["{pre}forget this session", "{pre}forget everything from this session",
                       "{pre}delete what you remember from this session"],
}
PREFIXES = ["", "hey mog ", "please "]
DICTATION_T = [
    "dear {person} thanks for the update on {topic} I will send the numbers by {day}",
    "note to self call {person} on {day} about {topic}",
    "the review of {topic} moves to {day} afternoon",
    "hi team quick reminder that the {topic} sync is on {day}",
    "I think we should ship {topic} next sprint and tell {person}",
]
NOISE_BASE = ["um", "uh huh", "okay okay", "hmm let me think", "sorry I was talking to someone else",
              "right right", "testing one two three", "oh wait", "yeah", "mm hmm", "hold on a second",
              "ah never mind me", "so yeah anyway"]
NOISE_FILL = ["", " uh", " um"]


def fill(templates, slots, rng):
    """All template x slot combinations, shuffled with rng, unique by text."""
    out = []
    for t in templates:
        keys = [k for k in slots if "{" + k + "}" in t]
        for combo in itertools.product(*(slots[k] for k in keys)):
            out.append(t.format(**dict(zip(keys, combo))))
    rng.shuffle(out)
    seen, uniq = set(), []
    for x in out:
        if x not in seen:
            seen.add(x)
            uniq.append(x)
    return uniq


def build():
    rng = random.Random(SEED)
    slots = {"metric": METRICS, "region": REGIONS, "period": PERIODS, "dim": DIMS, "entity": ENTITIES,
             "topic": TOPICS, "person": PEOPLE, "month": MONTHS, "day": DAYS}
    items = []

    def add(intent, texts, command="none", mode=None):
        for i, text in enumerate(texts):
            items.append({"text": text, "expected_intent": intent, "expected_command": command,
                          "capture_mode": mode or ("hands_free" if i % 4 == 3 else "push_to_talk"),
                          "speaker_track": "you"})

    add("question", fill(QUESTION_T, slots, rng)[:COUNTS["question"]])
    add("memory_question", fill(MEMORY_T, slots, rng)[:COUNTS["memory_question"]])
    add("context", fill(CONTEXT_T, slots, rng)[:COUNTS["context"]])
    per = COUNTS["command"] // len(COMMAND_T)
    for cmd, ts in COMMAND_T.items():
        add("command", fill(ts, {"pre": PREFIXES, "ws": WORKSPACES}, rng)[:per], command=cmd)
    add("dictation", fill(DICTATION_T, slots, rng)[:COUNTS["dictation"]])
    noise = [b + f for b in NOISE_BASE for f in NOISE_FILL]
    add("noise", noise[:COUNTS["noise"]])
    for n, it in enumerate(items):
        it["id"] = f"g{n:03d}"
    return items


def main(argv=None):
    p = argparse.ArgumentParser()
    p.add_argument("--out", required=True)
    a = p.parse_args(argv)
    with open(a.out, "w", encoding="utf-8") as f:
        json.dump(build(), f, indent=1, sort_keys=True)
        f.write("\n")


if __name__ == "__main__":
    main()
