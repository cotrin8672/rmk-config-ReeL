# Rotary decision diagnostic firmware

The failed signed-A GPIOTE/PPI experiment (`1dc23cf`) is reverted. This build
uses the `cb06d32` acquisition and decoder unchanged in their decision rules:
single P1 snapshot, dedicated GPIOTE waits, 2-tick sampling delay, 16 identical
A samples, and E -> I -> previous-direction fallback. Latest main is retained.
This is a diagnostic build, not a claim that the reversal defect is fixed.

## Use the left LCD

Only the left firmware needs updating for this diagnostic. The left LCD shows
the newest A confirmation at the top and the preceding one below it. Rotate to
reproduce the already known bad reversal and stop; photograph the display and
note the intended physical direction. Do the same at the known good position.
Do not infer the intended direction from the reported CW/CCW output.

Each record shows:

- `#N`: A confirmation number, including confirmations returning no event.
- `E>CW`, `I>CCW`, `H>CW`, or `U>-`: selected evidence and actual output.
  E = current A window, I = whole interval, H = previous direction,
  U = unknown. `-` means None, not a missing record.
- `t...ms`: time at entry to the confirming update() call.
- `A0>1 AB11`: old/new accepted A and the exact A/B passed to update().
- `E:...`, `I:...`, `H:...`: both pre-clear sums and pre-update history.

Both positive and negative signs keep the old direction mapping. E/I/H are
copied before clearing the sums or replacing the previous direction. No extra
GPIO reads are made for diagnostics. LCD formatting/flushing is outside the
acquisition loop, polled every 100 ms so None outputs are visible too. Rapid
rotation may skip displayed records; the LCD is not a complete event log.

## RAM record and limits

`rotary_diagnostics::TRACE` holds the latest 32 confirmations and the latest
1024 update() samples. Each sample keeps a sequential number, an Embassy tick
timestamp (32768 Hz), and the actual A/B values. Repeated identical samples
are retained, not collapsed into edges. Confirmation records reference the
sample number and timestamp. This is a rolling buffer: old slots are overwritten,
never silently presented as a complete trace. Full counters and timestamps are
in RAM; LCD counter/time fields are modulo 100,000,000 for width.

A debugger can inspect the RAM while halted. Samples require debugger access;
the LCD alone is intended for the first E/I/H diagnosis. A retained suffix whose
prefix has been overwritten is not enough to reproduce the decoder's prior
state. Do not replay that suffix as a fresh decoder and call it a device replay.
These are software sample times, not independently captured electrical edges.
The first diagnostic stage does not claim to observe all physical transitions.

The recorder never awaits or formats strings. Recording and display still add
CPU work and can affect scheduling; preserving the original sampling code does
not prove that wall-clock timing is identical. No trace data is written to flash.
No split protocol, direction queue, press/release timing, or trackball setting is
changed by the diagnostic commit.

## Checks

The frozen `cb06d32` decoder in `tests/support/reference_decoder.rs` is compared
against the instrumented decoder for every output and idle state across four
initial A/B states and 20,000 generated sample runs per state, including repeated
samples and skipped states. The tests exercise E/I/H and separately verify U,
pre-clear evidence, one-shot records, and buffer wrap with None outputs.

The existing two ignored collapsed-input contracts remain unresolved and ignored.
Passing the host suite or GHA establishes build/logic checks only. Actual bad
and good detent records are needed before changing the decoder or acquisition.
