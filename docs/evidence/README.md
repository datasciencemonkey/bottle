# Gate evidence

One file per release-gate run (AT-22). Use the format in `docs/tests/EVIDENCE-TEMPLATE.md`.
`scripts/check_gate_evidence.py` fails if a gate is reported as a pass without evidence, so no
unrun spike or gate can be a pass. This directory is empty until a person runs the gates
(reference devices, live workspace and HID rig: TODO(#270)).
