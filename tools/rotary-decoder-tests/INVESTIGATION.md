# Rotary state coverage — 2026-09-06

Base: `edbc0eee771dd848bd3b4f3c1d876b8945eb10d8` (`origin/main`).
Branch: `codex/rotary-state-coverage`. Firmware code is unchanged.

The existing 12 waveform tests pass but do not cover a known direction followed
by repeated ambiguous clicks across the same physical boundary. The new tests
check event order and count **after every click**, without assigning private
decoder fields. Two contract tests intentionally fail on the current firmware;
they are neither ignored nor marked `should_panic`.

## Coverage

For each capture mode, enumerate 20,480 six-click scenarios (122,880 clicks):

- Four initial A/B states and every CW/CCW history of length six (64 histories).
- Four inter-edge spacings: 1, 15, 16, 17 samples.
- Five rest durations: 0, 15, 16, 17, 200 samples.
- Contact bounce on/off and B-only rest chatter on/off.

Capture modes: all intermediate states visible, intermediate state hidden on
even physical boundaries, or hidden on odd boundaries. Boundary identity uses
the smaller of departure/arrival knob positions, so reversal crosses the same
boundary. Across those modes: 61,440 scenarios / 368,640 clicks.

Additional tests cover the reported sequence (rock, move one extra click, rock
again) in 16 phase/history/boundary variants; signed A-only clicks with unknown
or either previous direction; aborted A edges; and indistinguishable sampled
inputs for opposite physical movements.

This is exhaustive only within those finite dimensions, not every possible
contact waveform, timing, long history, interrupt schedule, or BLE queue state.
The model's hidden-edge placement is a hypothesis, not measured device data.
In particular, neither MCU wakeup latency nor GPIO capture is executed here.

## Results

Candidates were compiled in temporary standalone host crates with the same
integration tests. `drop-fallback` changes only the zero-movement branch from
`self.last_direction` to `None`. `close-after-b` uses `src/rotary_decoder.rs` from
historical commit `69741fa`. Neither candidate is installed in this branch.

Counts below combine the even/odd hidden-boundary matrices (245,760 clicks).

| Decoder | Visible matrix | Wrong-direction clicks | Missing clicks | Extra events | A-only contract |
|---|---:|---:|---:|---:|---|
| Current main | 20,480/20,480 pass | 23,040 | 40,320 | 0 | Pass |
| Drop fallback | 20,480/20,480 pass | 0 | 121,600 | 0 | Pass |
| Close after B | 20,480/20,480 pass | 0 | 122,880 | 0 | Fail |

All three fail all 16 reported-sequence variants. Current main produces 16
wrong-direction clicks and 40 missing clicks across those 160 clicks. Both
candidates produce 80 missing clicks. These counts describe the synthetic
corpus, not a predicted real-world failure rate.

## Conclusion

Neither candidate fixes the full requirement. Removing stale direction is not
equivalent to preserving one input per physical click. Waiting for B also breaks
the established A-clock behavior when no net B movement is sampled.

Once the two intermediate phase orders have collapsed into identical input,
the decoder cannot infer both opposite directions from identical prior history.
The failing hidden-edge contracts expose an acquisition/API limitation: they
cannot all pass with the existing `update(a, b)` observations alone. A successful
fix needs evidence captured before that loss, and acquisition-level tests using
that evidence, not an expected-direction hint fabricated from the test oracle.
The current tests do not establish where the physical device loses evidence.

## Run

Windows:

```powershell
rtk cargo test --manifest-path tools/rotary-decoder-tests/Cargo.toml --target x86_64-pc-windows-msvc --locked -- --nocapture
```

Linux / existing GitHub Actions: substitute `x86_64-unknown-linux-gnu`.
Expected current result: existing 12 pass; new tests 4 pass, 2 fail.
The existing workflow picks up integration tests automatically. A red test step
must not be bypassed to produce or promote a supposedly fixed firmware artifact.

Passing host tests would still not replace physical tests of both directions,
neighbouring detents, reversal history, slow/fast motion, and one full revolution.
