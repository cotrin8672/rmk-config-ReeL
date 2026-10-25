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

Start the local page from the repository root (Python 3 must be on PATH):

```powershell
powershell -NoProfile -File .\tools\start-trackball-lab.ps1
```

Open <http://127.0.0.1:4321/trackball-diagnostics.html> in desktop Chrome/Edge.
The script reuses an existing server or starts one in the background, independent
of the chat terminal. Run it again after a PC restart. No hosting or login is
required. This complements the existing calibration console: stabilize optical
tracking and compare hardware positions before fitting angle and sensitivity
corrections.

The page offers live raw displacement, SQUAL and shutter plots; hands-off rest
measurement; three strokes in each of four directions at slow or fast speeds;
condition histories and A/B comparisons; raw CSV/result JSON exports; and a
22x22 sensor-image viewer with single/continuous capture, reference image A,
raw brightness/contrast/neighbor-difference statistics, PNG and image JSON.
Record the ball, case position, CPI and a repeatable surface travel. Input CPI
and travel are notes; USB telemetry reads back actual CPI and records it for tests. Browser pointer
input and explicitly labeled synthetic demo data are available as separate
sources; neither substitutes for sensor diagnostics.

For direct sensor data, flash **only the right half** with
`reel_right_usb_diagnostics.uf2`. Unplug/replug USB, open the local page directly
in Chrome/Edge and select **USBで接続**, then ReeL. This uses WebHID, as the
existing calibration console does. The older `reel_right_diagnostics.uf2` used
a serial logger and must be replaced once. The local download link serves
`tools/firmware/reel_right_usb_diagnostics.uf2`, copied from successful CI.
This generated directory is not committed. Restore normal `reel_right.uf2`
after testing; the left half does not need to be flashed for diagnostics.

Build with `cargo build --release --locked --bin reel_right --no-default-features --features trackball-diagnostics`.

The diagnostic feature selects the RMK fork's opt-in `usb_diagnostics` HID
interface. It does not route measurements through the 1 KiB debug logger.
Sampling retains the existing seven-byte PMW3610 smart burst and a nominal
four-millisecond polling interval. A bounded 128-record queue batches three
complete records into each 64-byte USB response. Real overflow is signaled
and sequence gaps invalidate affected measurements. The live warning covers
the last four seconds while the readout retains cumulative counts.

The dedicated collection uses vendor page `0xff62`, usage `0x61`, report ID 0.
Keyboard and Vial reports remain on their original collections.

- Request: 64 bytes, prefix `[0x52,0x54,2,command,tag]` and zero padding.
  Commands: 0 start, 1 poll, 2 capture, 3 stop. Tags match each response.
- Reply: `[0xd2,tag,flags,count]` plus up to three 20-byte sensor packets;
  unused bytes are zero. Flags `3` identify protocol/camera support;
  `0x80` additionally indicates real queue overflow.
- `T,1` packet: sequence u32, timestamp_us u32, dx/dy i16, SQUAL u8,
  shutter u16, motion u8, CPI u16 (little-endian). The page unwraps the clock.
- `S,1`: frame ID u16 and phase (1 start, 2 restored, 3 capture/reset error).
- `F,1`: frame ID u16, offset u16, up to 14 seven-bit pixels; zero padding.
  A full frame has 484 pixels in 35 chunks, followed by the restored marker.
- Polling during capture does not cancel it. After two seconds without host
  requests streaming stops; the next poll resumes it. This endpoint exposes
  no calibration/flash writes or arbitrary register commands.

Capture pauses navigation and always resets/reconfigures PMW3610 afterward,
restoring CPI and smart mode. The page requires every valid chunk and a
restoration marker before accepting the image. Pixel order follows PixArt
Figure 17: top right, down each column, then left. Display contrast stretch
does not change PNG/raw statistics. Camera and movement tests are mutually
exclusive. Hardware pins, split roles, keyboard Bluetooth and calibration
flash formats are unchanged. Normal builds retain interrupt-driven polling.

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
  final feel checks with normal firmware, and test USB on the actual device.
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
| [WebHID](https://developer.chrome.com/docs/capabilities/hid) | Use the same OS HID access method as the calibration console, with a sensor-only USB collection. |
| RMK USB logger source | Replace the 1 KiB debug-log ring with batched binary HID requests for sensor measurements. |

Run the browser math/parser checks with `node --test tools/test-trackball-diagnostics.mjs`.
CI also runs the pinned RMK USB queue/protocol tests with `node tools/test-usb-diagnostic-protocol.mjs`, tests the sensor burst decoder and builds both normal and diagnostic UF2s.

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
