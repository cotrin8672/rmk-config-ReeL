# rmk-config-ReeL

RMK firmware and configuration for the two Seeed XIAO nRF52840 controllers in
[ReeL](https://github.com/cotrin8672/ReeL).

This repository was split from ReeL hardware commit
`b9bacc7681b5a1461f23b08cf7a2b1b867c73b0e`. Hardware sources, mechanical
models, and fabrication files remain in the ReeL repository.

The right half is the BLE/USB central and contains the PMW3610 trackball. The
left half is the BLE peripheral and contains the rotary encoder. The Sharp
memory LCDs on both halves are driven by this firmware.

## Implemented setup

- 4x11 unified matrix with the same 41 physical positions and four layers as
  the previous firmware
- BLE split with the right half at columns 6..10 and the left half at columns
  0..5
- PMW3610 at 250 Hz on the right half, with 1600 CPI as the per-profile default
- Calibrated fixed-point direction transform with input-length normalization and
  retained per-axis remainder:

  ```text
  transformed = M · raw
  output = transformed · |raw| / |transformed|
  ```

  The default direction matrix is:

  ```text
  M = [-0.265,  1.142]
      [-0.831,  0.562]
  ```

  The central firmware uses `TransformMode::Automatic`: an orthogonal
  calibration matrix uses precomputed one-angle rotation coefficients, while
  the existing non-orthogonal calibration uses a normalized direction map
  cached in 8 octants with 64 ratio buckets. Either cache is rebuilt only when
  the calibration matrix changes. Motion is not low-pass filtered. After the
  direction transform, the existing directional/user sensitivity and baseline
  half gain are applied with retained fractional remainders
  (`src/motion_gain.rs`).

- Auto Mouse Layer 3 with a five-second timeout
- Mouse buttons 1/2 on the J/K positions while the mouse layer is active
- Left encoder mapped to vertical scrolling on every layer
- BLE/USB Vial support with flash-backed keymap storage

## Vial layout

`vial.json` is the fixed Vial definition for the current 41-key ReeL hardware.
It is intentionally checked in without a PCB-to-Vial generator. If the physical
matrix or key geometry changes, review and update this file explicitly.

## Browser tuning console

### Sensor diagnostics and condition comparison

Open `tools/trackball-diagnostics.html` through a local HTTP server in desktop
Chrome/Edge, or use the hosted ReeL Trackball Lab. This complements the existing
calibration console: stabilize optical tracking and compare hardware positions
before fitting angle and sensitivity corrections.

The page offers live raw displacement, SQUAL and shutter plots; hands-off rest
measurement; three strokes in each of four directions at slow or fast speeds;
condition histories and A/B comparisons; raw CSV/result JSON exports; and a
22x22 sensor-image viewer with single/continuous capture, reference image A,
raw brightness/contrast/neighbor-difference statistics, PNG and image JSON.
Record the ball, case position, CPI and a repeatable surface travel. Input CPI
and travel are notes; BLE telemetry reads back actual CPI and records it for tests. Browser pointer
input and explicitly labeled synthetic demo data are available as separate
sources; neither substitutes for sensor diagnostics.

For direct sensor data, flash **only the right half** with
`reel_right_diagnostics.uf2`. Pair ReeL with the host in the OS Bluetooth
settings, open the page directly in Chrome/Edge and select **BLEで接続**.
Disconnect USB after flashing: both raw motion/quality and image capture use BLE.
If the OS cached the older GATT service table, remove and re-pair ReeL.
**USBで接続** remains available for motion/quality through its serial port. The `Build RMK firmware` artifact contains that image alongside
the normal `reel_right.uf2` and `reel_left.uf2`. Restore the normal right image
after testing. Do not flash the left image just to use trackball diagnostics.

Build the diagnostic variant with:

```powershell
cargo build --release --locked --bin reel_right --no-default-features --features trackball-diagnostics
```

The diagnostic feature selects RMK's existing USB CDC logger instead of its
default defmt logging, observes the existing seven-byte PMW3610 smart burst,
and polls at a nominal four-millisecond interval even when there is no motion.
Normal motion sampling adds no register writes. Explicit frame capture uses
PixArt's frame-burst register sequence and always resets/reconfigures the sensor
afterward, restoring active CPI and smart mode. Camera and movement tests are
mutually exclusive in the page; capture pauses navigation. There are no pin
changes, calibration writes or new flash records. Normal builds retain the original interrupt-driven polling and
RMK default features. The dependency uses the existing `cotrin8672/rmk` fork,
based on the previous pinned RMK revision, with an opt-in `ble_diagnostics`
service and a PMW3610 capture method. The compatible hardware is the ReeL right PCB's PMW3610
breakout on the existing P1_12/P1_13/P1_14/P1_15 connections, as documented in
the hardware repository split from `b9bacc7681b5a1461f23b08cf7a2b1b867c73b0e`.

USB lines are `RTB1,sequence,timestamp_us,dx,dy,squal,shutter,motion`, optionally
preceded by RMK's log prefix. Deltas are signed 12-bit sensor values before
rotation/gain and zero when the motion-valid bit is clear. SQUAL is burst byte
4 and shutter is big-endian bytes 5/6. Sequence gaps and malformed records
invalidate a capture rather than masquerading as optical tracking loss.

The BLE service is `ef558a00-6fa0-4c67-aa6e-06b1cdf63192`, alongside the
existing keyboard HID services. Its `ef558a01-…` characteristic accepts only
`[0x52,0x54,1,mode]` (0 stop, 1 motion stream, 2 one frame); encrypted GATT
writes are required. `ef558a02-…` notifies fixed 20-byte packets, fitting the
minimum ATT MTU. It exposes sensor data only, without key events or settings.
Disconnect disables streaming and drops pending packets.

- `T,1`: sequence u32, timestamp_us u32, dx i16, dy i16, SQUAL u8, shutter u16,
  motion u8, active CPI u16; little-endian. The page unwraps the 32-bit clock.
- `S,1`: frame ID u16 and phase u8 (1 start, 2 restored, 3 capture/reset error).
- `F,1`: frame ID u16, offset u16 and up to 14 seven-bit pixels; zero padding.
  A frame has 484 pixels in 35 chunks. The page rejects missing, duplicate,
  out-of-range or mismatched chunks and waits for the restored marker.

Pixel order follows PixArt Figure 17: start at top right, proceed down each
column, then move left. PNG is the original 22x22 grayscale image; contrast
stretch applies to the display only. Image brightness, pixel standard deviation
and neighbor RMS are relative optical measures, not tracking pass/fail scores.
The measured image interval includes capture, reset and BLE transfer; it is not
the sensor's internal imaging frame rate. The page stops continuous capture
when hidden. Image references remain in memory and are included in image JSON.

Metrics have deliberately limited interpretations:

- Directional output is the median net displacement of three equal strokes;
  repeatability is sample standard deviation / mean (CV). Four-direction
  balance is the smallest / largest direction median, undefined for zero output.
- Sideways RMS is cumulative deviation from the stroke's start/end line;
  reversal is backward displacement / total absolute displacement projected
  onto that line. Both are independent of sensor mounting rotation.
- SQUAL P10 is calculated per stroke, then summarized by its median. There is
  no universal SQUAL pass threshold. Shutter is reported in register units.
- Maximum no-input time includes intended stops, startup and the end of a
  gesture. It is not a sensor-drop percentage. A fast/slow ratio requires the
  same physical rotation, CPI and input source; hand motion remains a confound.
- Sampling is observed register readout/receipt, not sensor imaging frame rate.
  The diagnostic build changes logging load and idle polling/power. Repeat
  final feel checks with normal firmware, and test USB/BLE on the actual device.
- History stores summaries locally; raw samples for the current measurement
  can be exported before starting another. Data is not uploaded by the page.

Research informing this implementation:

| Reference | Applied finding |
| --- | --- |
| [Keyball Link](https://keyball-link.shiroganelab.com/) and [Vial Web](https://vial.rocks/) | Custom keyboard tools connect to firmware-defined interfaces; retain ReeL's existing Vial calibration instead of building another settings protocol. |
| [QMK Raw HID](https://docs.qmk.fm/features/rawhid) and [HID Explorer](https://nondebug.github.io/webhid-explorer/) | Raw HID exposes defined reports, not arbitrary hidden sensor registers; live sensor diagnostics need firmware support. |
| [Pinned RMK PMW3610 driver](https://github.com/rmk-rs/rmk/blob/936a2a828fddc195d20e42bd952119cd3afa8172/rmk/src/input_device/pmw3610.rs) and [Zephyr's PMW3610 driver](https://github.com/zephyrproject-rtos/zephyr/blob/main/drivers/input/input_pmw3610.c) | Reuse the smart burst; confirm SQUAL/shutter offsets and the motion-valid gate. |
| [Mouse Sensor Lab](https://keymap.io/mouse-sensor-test/) | Repeated slow/fast strokes, hands-off noise and condition comparison are useful; browser mouse output still includes the OS/firmware path. |
| [PixArt full PMW3610 datasheet](https://trackballs.eu/media/Nakabayashi/Digio2/PMW3610DM-SUDU.pdf), pp17/33 | Frame-burst register sequence, mandatory reset before navigation, 22x22 pixels and column order. |
| [Web Bluetooth](https://developer.chrome.com/docs/capabilities/bluetooth) and [GATT blocklist](https://github.com/WebBluetoothCG/registries/blob/master/gatt_blocklist.txt) | Standard HID GATT is blocked in browsers; provide an encrypted, sensor-only custom service while retaining RMK keyboard BLE. |
| [Web Serial](https://developer.chrome.com/docs/capabilities/serial) | Retain the existing USB logger for motion/quality without a native bridge. |

Run the browser math/parser checks with `node --test tools/test-trackball-diagnostics.mjs`.
CI also tests the sensor burst decoder and builds both normal and diagnostic UF2s.

### Calibration console

Open `tools/trackball-profiler.html` in Chrome or Edge and select
**ワイヤレス接続**. The profiler uses RMK's existing vendor-defined Vial HID
collection, so the same page works through either USB or the bonded BLE
keyboard connection.

The console exposes one PMW3610 CPI value for each of RMK's five Bluetooth
profiles through a profile selector and a 200-step slider. Changes are saved
automatically after a short debounce and update the active profile within 25 ms;
switching Bluetooth profiles applies that profile's stored CPI automatically.

The 24 bytes immediately before the calibration record in RMK's macro buffer
are reserved for a versioned `RCP1` profile-CPI record. It contains five
little-endian CPI values and an FNV-1a checksum. The central mirrors valid
records into a wear-levelled flash journal at `0xA7000`.

The last 28 bytes of RMK's macro buffer are reserved for a versioned `RLC1`
calibration record. The record contains the four fixed-point matrix
coefficients and an FNV-1a checksum. A valid write is persisted by RMK's normal
flash-backed Vial path and is applied to the live pointing pipeline within
25 ms; reflashing or rebooting is not required.

The profiler reads the current matrix on connection. With automatic apply
enabled, it writes the fitted replacement matrix after measurement settles.
The firmware rejects corrupt, near-singular, and excessively large matrices
and keeps the last valid value.

## Build outputs

The `Build RMK firmware` GitHub Actions workflow produces:

- `reel_right.uf2` for the right central
- `reel_left.uf2` for the left peripheral

For a local build, install stable Rust with the `thumbv7em-none-eabihf` target,
`llvm-tools`, `cargo-binutils`, and `cargo-hex-to-uf2`, then run:

```powershell
cargo build --release --locked --bins
cargo objcopy --release --locked --bin reel_right -- -O ihex reel_right.hex
cargo hex-to-uf2 --input-path reel_right.hex --output-path reel_right.uf2 --family nrf52840
cargo objcopy --release --locked --bin reel_left -- -O ihex reel_left.hex
cargo hex-to-uf2 --input-path reel_left.hex --output-path reel_left.uf2 --family nrf52840
```

## First flash

1. Put each XIAO into its UF2 bootloader by double-tapping reset.
2. Flash `reel_left.uf2` to the left controller.
3. Flash `reel_right.uf2` to the right controller.
4. Power both halves on and pair the host with `ReeL`.

RMK uses its own BLE split bonding and storage format. Flash both halves from
the same build when the split protocol changes. The first on-device validation
should cover all 41 switches, the encoder direction, trackball direction and
gain, five-second AML release, BLE split reconnection, and Vial persistence.
