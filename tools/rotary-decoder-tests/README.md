# Frozen rotary capture

This build changes acquisition scheduling and retains the frozen diagnostics.
Physical improvement has not yet been established. It preserves
cb06d32 decoder decisions (E, then I, then history), GPIO acquisition and
16-sample A confirmation. The failed PPI experiment remains reverted.

## Acquisition change after the measured reversal

The supplied `rotary-20260917-152447-224.log` (firmware 493c176) reproduces
the reported wheel-down / PC-up event: prior AB=00, then 16 samples of 11,
E=I=0 and H=-1. No cancellation/idle clear happened before confirmation.
The 15 measured sample gaps are 9-10 RTC ticks (275-305 us), although the
timer delay is only 2 ticks. The first edge's latency is NOT measured.

Previously the reader was inside RMK's thread-mode run_all with LCD/matrix
work. GPIOTE latches an event flag, then wakes that executor; it does not save
the pin state at the edge. The reader now has its own interrupt executor on
EGU1_SWI1 at P3. It can read pins without waiting for thread-mode rendering
or USB formatting. MPSL keeps EGU0 and all its existing priorities/resources.
Only the direction channel and diagnostic RAM locks become interrupt-safe;
RMK publishing remains in thread mode. No decoder/fallback rule is changed.

This removes one software scheduling delay, not every possible loss: IRQ
masking, higher-priority work and the active sampling interval can still hide
closely spaced edges. The measured fixture intentionally still replays the
old wrong output. CI cannot demonstrate that the new reader sees a missing
intermediate state. Use `-NewCapture -Mode Next` for a one-click comparison
at the same bad position: it exports a capture whether the result is E/I/H/U.

The second measured capture `rotary-20260917-153721-903.log` on f1d47ab
still produced a reported down / PC-up reversal: 37 samples, observed states
00 -> 11 -> 00 -> 11, all Gray deltas/E/I zero, output H=1. The interrupt
executor alone therefore did not resolve the failure. Both actual captures
are retained as replay fixtures, without inventing intermediate states.

The reader now waits for either phase's GPIOTE event OR the two-tick timer
throughout active debounce too, instead of using only the timer in that phase.
After arming it rechecks the port; after waking it reads a single actual port
snapshot and never uses the winning future to infer transition order. Events
can still coalesce before service; this does not guarantee lossless capture.
The 16-sample rule is unchanged, but additional B-event samples can shorten
the elapsed A confirmation time. This timing effect needs device validation.

## Collect one bad-position trace (Windows)

1. Flash `reel_left.uf2` to the left half. Keep the right half connected normally.
2. Connect the left half to the PC with a USB data cable. Leave bootloader mode.
3. Run `./tools/rotary-decoder-tests/collect.ps1` in PowerShell.
4. After the collection prompt, alternate up/down at the known bad position.
   Stop when the LCD says FROZEN and the script reports a saved file.
5. Send the `.log` from `Downloads/ReeL-rotary-logs`, plus the physical direction
   of the last movement and what the PC did. No hand transcription is needed.

The default trigger freezes at the first A confirmation that uses H (history)
or U (unknown), including an unknown confirmation that produces no event.
This matches the reported E=I=0 condition; H alone does not prove the physical
movement was wrong. It does not wait for four movements or test speed.

If automatic port detection fails, specify `-Port COM7` using the actual port.
A completed trace stays frozen despite further rotation or USB reconnection
while the board stays powered. Rerunning the command retrieves that same trace.
Only `-NewCapture` explicitly replaces it. Reset/power loss loses RAM data.
To obtain a comparison later, use `-NewCapture -Mode Next` at the good position;
this freezes at the next A confirmation regardless of source.

## Evidence and limitations

The 512-entry rolling capture retains every actual update() call, including
repeated samples, software timestamp (32768 Hz), A/B, full decoder state before
and after, Gray delta, E/I before clearing, clear reason, selected source and
output. The first retained complete before-state anchors replay even when
older samples were overwritten; BEGIN explicitly reports that overwritten count.
A replay cannot recover physical edges missed between software samples or the
history before this retained prefix. These are not hardware edge timestamps.

`b_`/`a_` columns mean before/after state, not GPIO channels. State fields are
stable/candidate A, A run length, prior AB, idle sample count, tracking flag,
E, I and H. The `clears` bitmask is 1=A confirmation, 2=cancelled A window,
4=idle expiry. Directions are 1=CW, -1=CCW, 0=None; sources E/I/H/U and `-`
(no confirmation). No GPIO reads are added by diagnostics.

Acquisition never waits on USB or formats text. Capture adds CPU/RAM work and
USB adds interrupts, so identical wall-clock sampling is not guaranteed.
USB enumeration, transfer and behavior must still be tested on the device.
The trackball and split event protocol are not changed.

## File integrity and replay

USB CDC commands are INFO, ARM (H/U trigger), NEXT (any confirmation), DUMP.
ASCII LF framing: `BEGIN,1,generation,count,dropped,32768`, CSV header and rows,
then `END,count,fnv1a32`. The checksum covers the header and rows including LF.
The collector checks checksum, count and sequence before saving a completed
file; interrupted data is retained as `.partial`. It installs no drivers.

Run the host replay with the host target appropriate to your computer:

```powershell
cargo run --manifest-path tools/rotary-decoder-tests/Cargo.toml --target x86_64-pc-windows-msvc --locked --bin replay -- path/to/rotary.log
```

Replay checks every state, delta, clear and output against the firmware decoder.
Host tests compare instrumented decisions with the frozen cb06d32 reference,
exercise wrap/freeze/rearm and export/replay, and reject corrupt/truncated logs.
GHA also validates a generated fixture with the PowerShell collector. The two
ignored collapsed-input contracts remain unresolved; host/CI success does not
establish physical improvement or USB functionality.
