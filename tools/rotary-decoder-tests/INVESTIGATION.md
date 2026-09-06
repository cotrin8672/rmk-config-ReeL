# Rotary state coverage — 2026-09-06

Base: `edbc0eee771dd848bd3b4f3c1d876b8945eb10d8` (`origin/main`).
Branch: `codex/rotary-state-coverage`.

The existing 12 waveform tests pass but do not cover a known direction followed
by repeated ambiguous clicks across the same physical boundary. The new tests
check event order and count **after every click**, without assigning private
decoder fields. They exposed the acquisition failure before the firmware fix.

## Coverage

For each capture mode, enumerate 20,480 six-click scenarios (122,880 clicks):

- Four initial A/B states and every CW/CCW history of length six (64 histories).
- Four inter-edge spacings: 1, 15, 16, 17 samples.
- Five rest durations: 0, 15, 16, 17, 200 samples.
- Contact bounce on/off and B-only rest chatter on/off.

Capture modes: all intermediate states visible, or a dedicated edge latch
preserving an intermediate state missed by periodic sampling on even or odd
physical boundaries. Boundary identity uses
the smaller of departure/arrival knob positions, so reversal crosses the same
boundary. Across those modes: 61,440 scenarios / 368,640 clicks.

Additional tests cover the reported sequence (rock, move one extra click, rock
again) in 16 phase/history/boundary variants; signed A-only clicks with unknown
or either previous direction; aborted A edges; and indistinguishable sampled
inputs for opposite physical movements.

This is exhaustive only within those finite dimensions, not every possible
contact waveform, timing, long history, interrupt schedule, or BLE queue state.
The model's edge placement is a hypothesis, not measured device data.
In particular, neither MCU wakeup latency nor GPIO capture is executed here.

## Results

Candidates were compiled in temporary standalone host crates with the same
integration tests. `drop-fallback` changes only the zero-movement branch from
`self.last_direction` to `None`. `close-after-b` uses `src/rotary_decoder.rs` from
historical commit `69741fa`. Neither candidate is installed in this branch.

Counts below are from the pre-fix collapsed-input experiment and explain why a
decoder-only change was rejected.

| Decoder | Visible matrix | Wrong-direction clicks | Missing clicks | Extra events | A-only contract |
|---|---:|---:|---:|---:|---|
| Current main | 20,480/20,480 pass | 23,040 | 40,320 | 0 | Pass |
| Drop fallback | 20,480/20,480 pass | 0 | 121,600 | 0 | Pass |
| Close after B | 20,480/20,480 pass | 0 | 122,880 | 0 | Fail |

All three failed all 16 reported-sequence variants. Current main produced 16
wrong-direction clicks and 40 missing clicks across those 160 clicks. Both
candidates produce 80 missing clicks. These counts describe the synthetic
corpus, not a predicted real-world failure rate.

## Implemented correction

Neither decoder candidate fixes the full requirement. Removing stale direction is not
equivalent to preserving one input per physical click. Waiting for B also breaks
the established A-clock behavior when no net B movement is sampled.

Once the two intermediate phase orders have collapsed into identical input,
the decoder cannot infer both opposite directions from identical prior history.
The collapsed-edge contracts exposed an acquisition/API limitation: they
cannot all pass with the existing `update(a, b)` observations alone. A successful
fix needs evidence captured before that loss, and acquisition-level tests using
that evidence, not an expected-direction hint fabricated from the test oracle.
The firmware now uses dedicated GPIOTE channels for P1.14/P1.15, checks the
coherent P1 level immediately after arming them to close the rearm race, and
uses one P1 `IN` load for every A/B sample. It feeds only observed states into
the existing decoder; it does not infer phase order from timestamps.

## Run

Windows:

```powershell
rtk cargo test --manifest-path tools/rotary-decoder-tests/Cargo.toml --target x86_64-pc-windows-msvc --locked -- --nocapture
```

Linux / existing GitHub Actions: substitute `x86_64-unknown-linux-gnu`.
Expected result: 21 tests pass. The existing workflow picks up integration tests automatically.

Passing host tests would still not replace physical tests of both directions,
neighbouring detents, reversal history, slow/fast motion, and one full revolution.

## Acquisition investigation

`tests/acquisition.rs` contains three passing failure demonstrations that drove
the acquisition fix. They model source-level reads/wakeup ordering; they are
not captured device traces.

1. `peripheral.rs` reads P1.14 then P1.15 via two `Input::is_high()` calls.
   In embassy-nrf 0.11 each call performs a separate GPIO `IN` register load.
   Physical CW `11 -> 01 -> 00`, interrupted between the A and B reads, can
   become observed `11 -> 10 -> 00`: a valid but opposite Gray path. This is
   worse than merely losing direction. A single P1 `IN` load avoids a torn
   pair, but still cannot recover two edges occurring between complete reads.
2. `Flex::wait_for_any_edge()` in embassy-nrf 0.11 chooses `SENSE` opposite to
   the pin level at its first poll, not opposite to the last decoded sample.
   If B changes from 0 to 1 between the last decoded `00` and arming the wait,
   it now waits for B=0. The later A edge wakes the task at `11`, hiding the
   `01` departure of a CCW reversal. Waiting for a level different from the
   last decoded level removes that rearm race, but not arbitrary wake latency.
3. PPI first/last timestamp reconstruction is not full waveform capture.
   `11 -> 10 -> 11 -> 01 -> 00` is a CW click with harmless rest B chatter,
   but its first B edge precedes its first A edge. Conversely, CW followed by
   A bounce (`11 -> 01 -> 00 -> 10 -> 00`) leaves the latest A edge after the
   latest B edge. Both can be wrongly reconstructed as CCW from timestamps
   alone. The actual complete streams decode CW. The `615f82b` first-edge
   implementation also consumes its hint once and rearms only after idle,
   so continuous chatter can leave later ambiguous clicks without a hint.

Source references (pinned versions):

- [GPIO input load](https://docs.rs/embassy-nrf/0.11.0/src/embassy_nrf/gpio.rs.html)
- [GPIO wait and interrupt handling](https://docs.rs/embassy-nrf/0.11.0/src/embassy_nrf/gpiote.rs.html)
- Repository `src/peripheral.rs`, and historical commits `ee901d1`, `615f82b`.

The async `Timer::after_ticks(2)` is a minimum wait, not a guaranteed sampling
period. Executor work, interrupt latency, and a full direction-channel `.send`
can lengthen the interval. RMK's downstream events carry direction explicitly;
the keymap caches encoder layers separately by direction. No downstream
position-parity state was found that would itself explain alternating detents.

### Correction constraints

- Read A/B from one P1 snapshot; retain P1.14/P1.15 assignments and mappings.
- Use dedicated hardware edge latches so idle wakeup does not depend on PORT
  `SENSE` being configured from a newly changed level.
- Keep raw direction evidence separate from A's approximately 1 ms debounce.
  If feeding captured edges at variable intervals, count elapsed stable time,
  not calls to `update`; otherwise replay changes the debounce duration.
- Check the level after arming to close the clear-before-wait race.
- Preserve the established A-clock/Gray-direction semantics on complete input.
  Do not manufacture a direction on an acquisition gap or suppress a valid
  A-only click while waiting indefinitely for B.

The host suite now covers coherent edge wakes, the rearm race, torn reads,
phase/history combinations, bounce and rest chatter. GPIOTE is a latch rather
than a FIFO, so an unmeasured case where both direction-bearing edges complete
before task service remains outside the host proof.

These source-level counterexamples establish concrete vulnerabilities, not
which one dominates this physical unit. Prior logs contain aggregate counts,
not timestamped A/B traces. The connected-device inventory exposed ReeL over
Bluetooth and no named SWD probe; it does not provide raw left-side GPIO data.
Physical validation must still cover both neighbouring detents, the reported
move-one-click sequence, slow/fast reversal, and a full revolution. Hardware
fault is not established.
