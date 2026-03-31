# LED flash stack issue in `embassy_wifi_ble`

## Summary

Adding BLE discovery LED flashing to `embassy_wifi_ble/src/main.rs` exposed a stack-guard crash / hang on the badge.

The issue is not specific to the flash logic itself. After investigation, simply constructing an LED driver in `main` was enough to trigger the problem when the display task was also present.

This strongly suggests the app is already very close to a stack limit, and small changes in code shape or stack layout are enough to tip it over.

## Current stable state

This version was reported working on hardware.

No LED flashing logic is currently present in the stable version.

## Requested feature

Flash all LEDs blue for 1 second whenever the BLE scanner detects a new BLE device in:

- `embassy_wifi_ble/src/main.rs`

LED examples were taken from:

- `embassy_bluetooth_remote/src/main.rs`

## What was tried

### Attempt 1: separate LED flash task

Added:

- `BLE_FLASH_REQUESTS: AtomicU32`
- BLE scanner callback increments the counter for each newly seen device
- a dedicated `led_flash_task`
- LED driver setup based on the patterns in `embassy_bluetooth_remote`

Result:

- hangs after `Display task started`
- sometimes panics with:
  - `Detected a write to the stack guard value on ProCpu`

### Attempt 2: merge LED flashing into `display_task`

Removed the extra LED task and handled pending flash requests from inside `display_task`.

Result:

- still crashed / hung

This suggested the extra Embassy task was not the only issue.

### Attempt 3: lighter LED driver

Switched from `TypedLeds` to `tildagon::leds::Leds` to reduce startup state and avoid extra async LED init.

Result:

- still crashed / hung

### Attempt 4: incremental testing from baseline

Reverted to the committed baseline, then reintroduced changes one at a time.

#### Step 1: construct LED driver only

Only added:

```rust
use tildagon::leds::Leds;
let _leds = Leds::new(tildagon.rmt, tildagon.led_pin);
```

No LED writes, no BLE hook, no flash logic.

Result:

- this alone was enough to trigger the stack guard crash when the display task was present

This was the key finding.

### Attempt 5: reduce display task stack pressure

Moved all `format!` calls out of `display_task` into separate `#[inline(never)]` helper functions:

- `format_wifi_scans`
- `format_wifi_networks`
- `format_ble_seen`
- `format_battery_voltage`
- `format_fps`

Result:

- hardware test reported this version was stable again

### Attempt 6: retry incremental LED step on top of the stack reduction

Re-added only:

```rust
let _leds = Leds::new(tildagon.rmt, tildagon.led_pin);
```

Result:

- stack guard crash returned

## Conclusions

1. The app is already extremely close to a stack limit.
2. The problem is not recursion in the LED flashing code.
3. The problem is not specific to the BLE flash state machine.
4. The display-heavy app shape plus any additional stack/layout pressure appears enough to cross the limit.
5. The retained display formatting refactor reduces stack pressure, but not enough to allow LED driver construction in the same app.

## Likely root cause area

Most likely candidates:

- stack pressure in the display/render path
- stack pressure in the `#[esp_rtos::main]` / generated Embassy task for async `main`
- general sensitivity to task/frame layout on this app

One important observation from code inspection:

- `#[esp_rtos::main]` expands the async `main` body into an `#[embassy_executor::task()]`

So the effective main task stack is part of the problem space, not just explicit spawned tasks.

## Important context for follow-up

- `embassy_bluetooth_remote/src/main.rs` works with BLE + LEDs, but does not use the display
- `embassy_wifi_ble/src/main.rs` uses the display and appears stack-sensitive
- previous display striping work in this repo already required stack-related optimisation

## Recommended next steps

1. Find the actual stack-sizing/configuration knob for this platform/runtime.
   - Investigate `esp-rtos`, `esp-hal`, and any project config that controls main task stack size.
   - The simple Embassy "max task count" idea is not applicable here.

2. Measure or localise the overflow more directly.
   - If possible, resolve the reported PC addresses with an Xtensa `addr2line` toolchain.
   - Add very small progress logs around display task startup / first render if needed.

3. Reduce stack usage in `display_task` further.
   - Audit large locals and closure-heavy rendering paths.
   - Investigate whether `render_with_stripes` or `draw_ui` can be made lighter.

4. Only retry the LED flash feature after stack headroom is understood.
   - Re-introduce changes incrementally again.
   - Start with LED driver construction only.
   - Then LED clear/write.
   - Then event plumbing from BLE scanning.

## Commands used during investigation

```bash
cargo check -p embassy_wifi_ble
cargo fmt --all && cargo check -p embassy_wifi_ble
git --no-pager diff -- embassy_wifi_ble/src/main.rs
git --no-pager show HEAD:embassy_wifi_ble/src/main.rs
```

## Files involved

- `embassy_wifi_ble/src/main.rs`
- `embassy_bluetooth_remote/src/main.rs`
- `tildagon/src/leds.rs`
- `tildagon/src/hardware.rs`

