# Bottle

Bottle is a small desktop app for creating and managing **MOGs**: floating, always-on agents that dictate, answer questions through Genie One MCP, and send approved context to Genie on Databricks.

> The project folder was formerly named *Mother Databricks* / *Motherbricks*. Use the name **Bottle**.

## Start here

| File | Purpose |
|---|---|
| [`AGENTS.md`](AGENTS.md) | Context index and binding decisions for agents working in this repo |
| [`requirements.html`](requirements.html) | Final end-to-end requirements (v1.2, ASD-STE100, 9 SVG diagrams) |
| [`research/final-requirements.md`](research/final-requirements.md) | Markdown source for `requirements.html` |
| [`research/message-board.md`](research/message-board.md) | Append-only agent message board |
| [`research/kimi-snapshot/`](research/kimi-snapshot/) | Snapshot of the alternative Kimi requirements run (reference only) |
| [`mog-rive/`](mog-rive/) | Rive RML source for the MOG mascot |

Rebuild the HTML: `python3 tools/build_requirements_html.py`

## License

MIT. See [`LICENSE`](LICENSE).
