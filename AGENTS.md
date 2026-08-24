# ReeL RMK firmware notes for agents

## Completion requirements

- For RMK firmware changes, do not report completion from local checks alone.
- Push the working branch, wait for the `Build RMK firmware` GitHub Actions workflow to succeed, and confirm that it produced the `reel-rmk-firmware` artifact before merging or cherry-picking into `main`.
- Treat GitHub Actions as build and artifact evidence only. Physical cursor, encoder, display, battery, BLE, and split behavior require explicit device tests.

## Hardware boundary

- Hardware sources live in https://github.com/cotrin8672/ReeL.
- `vial.json` is a fixed definition for the current 41-key matrix. There is no PCB-derived generator.
- A pin assignment, matrix, sensor, display, battery, or split-role change must be checked against the compatible ReeL hardware revision.
