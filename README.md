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
  the calibration matrix changes. Small motion uses an adaptive low-pass
  filter, larger motion bypasses it, and the independent gain is unity by
  default (`src/motion_gain.rs`).

- Auto Mouse Layer 3 with a five-second timeout
- Mouse buttons 1/2 on the J/K positions while the mouse layer is active
- Left encoder mapped to vertical scrolling on every layer
- BLE/USB Vial support with flash-backed keymap storage

## Vial layout

`vial.json` is the fixed Vial definition for the current 41-key ReeL hardware.
It is intentionally checked in without a PCB-to-Vial generator. If the physical
matrix or key geometry changes, review and update this file explicitly.

## Browser tuning console

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
