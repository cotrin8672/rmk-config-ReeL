# Signed A-edge rotary capture

This branch replaces executor-time A/B sampling with a continuously armed
GPIOTE/PPI circuit. P1.14/P1.15, positive = Clockwise, and the existing 5 ms
press/release transport are unchanged. A quiet time is 1000 us; B never resets it.

## Resources and circuit

| Resource | Use |
| --- | --- |
| GPIOTE channels 0, 1 | A and B Toggle events |
| TIMER1, TIMER2 | 32-bit cumulative positive and negative A counts |
| TIMER3 | 1 MHz quiet timer, COMPARE0 STOP shortcut |
| TIMER4 | 32-bit stable-frame count |
| PPI channels 0..2 / group 0 | EQ state: A counts positive and switches to NE; B switches to NE |
| PPI channels 3..5 / group 1 | NE state: A counts negative and switches to EQ; B switches to EQ |
| PPI channel 6 | A -> TIMER3 CLEAR + START |
| PPI channel 7 | TIMER3 COMPARE0 -> TIMER1/2 CAPTURE0 |
| PPI channel 8 | TIMER3 COMPARE0 -> TIMER4 COUNT |

The application's peripheral tokens are consumed by `Capture`. TIMER0, RTC0,
RTC1, and PPI17..31 remain with MPSL/SDC/Embassy. The pinned nrf-sdc
`abe49d22` vendor headers (`mpsl_hwres.h`, `sdc_soc.h`) reserve MPSL channels
19/30/31 and SDC channels 17..31. The nRF52 integration resource lists below
reserve no PPI groups. No FEM is configured in this application.

- [Nordic PPI specification](https://docs.nordicsemi.com/bundle/ps_nrf52840/page/ppi.html)
- [MPSL integration resources](https://github.com/nrfconnect/sdk-nrfxlib/blob/main/mpsl/doc/mpsl.rst)
- [SDC integration resources](https://github.com/nrfconnect/sdk-nrfxlib/blob/main/softdevice_controller/doc/softdevice_controller.rst)

Counters are not cleared at click/idle boundaries. TIMER3 stops after each
quiet deadline and restarts on an A edge; the direction circuit stays active.
Its interrupt copies captured counts into a 64-frame queue, with the frame
counter sampled before and after the copy. Skipped sequence numbers, a changed
sequence during copying, or a new compare during copying latch a fault.
The decoder consumes even-edge frames without emitting; odd-edge frames emit
exactly once using the sign of positive-minus-negative wrapping deltas.

## Faults

Faults stop rotary acquisition/output until reset; other keyboard tasks continue.
An already pressed encoder event still receives its release. Pending directions
are discarded, never replaced with the last direction or a combined interval.
The executor emits `Rotary capture stopped: fault N (reset required)` via defmt.

| N | Meaning |
| --- | --- |
| 1 | Edge during startup seeding; initial EQ/NE cannot safely be established |
| 2 | Captured frame overwritten or sequence discontinuity |
| 3 | Frame queue full |
| 4 | Decoder received a missing/duplicate frame |
| 5 | Direction output queue full |

## Validation and device acceptance

Host tests use an ideal **sequential edge** model. They cover all 131,068
single-bit paths of length 0..14 from four initial states, all six-click
direction histories with bounce/B chatter, 18-click rotations, cancelled first
A edges, quiet timing, delayed consumption, missing frames, and counter wrap.
They replace tests of the removed sampled-level acquisition API. Historical
sampling-loss demonstrations remain in git at `cb06d32`.

These tests and GHA are not hardware acceptance. Still required on the device:

1. Verify positive/negative counts and the one-active-group invariant with
   generated A/B signals; vary edge separation, including nearly simultaneous
   phases. The group-switch propagation limit is not measured yet.
2. Sweep A edges across the 1 ms compare boundary. CLEAR/START versus STOP,
   COUNT versus CAPTURE, and coincident A/B edges are not resolved by the host
   model. A deadline collision may partition or lose a click.
3. Delay IRQ service across multiple frames and saturate each queue; verify a
   latched fault with no combined/guessed clicks. Normal rotation plus BLE,
   matrix, LCD, and flash activity must produce **no faults**.
4. Check slow/fast CW/CCW, exactly 18 clicks/revolution, and first-click reversal
   at both alternating detent positions. Keep the encoder still during boot.
5. Check battery impact: direction capture stays armed during idle. No idle
   power optimization is included in this experiment.
