# Plan: Adopting `tildagon-rs` Best Practices

This document outlines a plan to refactor the `tildagon` crate to adopt the superior architectural patterns found in the `tildagon-rs` project, specifically focusing on type safety, resource management, and ergonomic async APIs.

The rollout must be **compatibility-first**: the `embassy_blinky` project currently depends on `tildagon::hardware::TildagonHardware::new()`, the exposed `i2c`, `button_int`, `rmt`, and `led_pin` fields, plus `Buttons::wait_for_press()`. Those entry points must continue to work while we improve the internals.

## Attribution Policy
To respect the work of the community, we will **credit the upstream project** [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs) in code comments whenever we reuse an architectural pattern, macro, or specific hardware mapping logic derived from their project.

## Key Improvements to Adopt

### 1. Typed Pin Abstraction
**Current**: We use hardcoded I2C addresses and bitmasks (e.g., `i2c.write(0x59, &[0x06, 0xF0])`).
**Best Practice**: Use a `Pin<const ADDRESS: u8, const PORT: Port, const PIN: u8>` type.
*   **Benefit**: Move hardware mapping errors from runtime to compile-time.
*   **Ergonomics**: Allows code like `pins.button.btn1.into_input(i2c).await`.

### 2. Transparent I2C Mux Handling
**Current**: We manually switch mux channels using `i2c.write(0x77, &[1 << channel])`.
**Best Practice**: Implement a `SharedI2cBus` wrapper that handles channel switching automatically.
*   **Benefit**: Drivers for devices on different branches of the mux (e.g., IMU on Port 7, Display on Port 0) can be used simultaneously without manual coordination.
*   **Safety**: Prevents a task from accidentally reading the wrong device because another task switched the mux.

### 3. Resource Splitting (`split_resources!`)
**Current**: `TildagonHardware` owns everything, making it hard to share safely between tasks.
**Best Practice**: A macro-based resource assignment system.
*   **Benefit**: Allows the `main` function to "split" hardware. For example, the `LED` task gets the LED pins, the `Button` task gets the button pins and the interrupt pin, and the `Display` task gets the SPI pins.
*   **Async-Friendly**: Eliminates the need for global `Arc<Mutex<...>>` wrappers around the entire hardware struct.

### 4. Interrupt-Triggered Event System
**Current**: Polling buttons on a timer (via `embassy_blinky` logic) or manual interrupt management.
**Best Practice**: Combine `tildagon-rs`'s `wait_for_interrupt()` with our existing silencing logic.
*   **Benefit**: Create an async stream of events that only wakes up when a button is actually pressed, saving power and CPU cycles.
*   **Caveat**: `tildagon-rs` provides a `wait_for_interrupt()` building block, not a complete button event stream. We should treat an async event API as a follow-on enhancement, not as part of the initial compatibility-preserving refactor.

---

## Implementation Plan

### Phase 0: Compatibility Baseline ✅
*   [x] Document the `embassy_blinky` integration points that must remain working during the refactor:
    * `TildagonHardware::new()`
    * `TildagonHardware` fields: `i2c`, `button_int`, `rmt`, `led_pin`
    * `Buttons::wait_for_press()`
    * `Leds::new(...)`
*   [x] Treat `embassy_blinky` as the regression test for the refactor and keep it building throughout.

### Phase 1: Core Foundation
*   [ ] Port the `Port` and `Pin` types from `tildagon-rs` (with attribution).
*   [ ] Implement the `TCA9548A` mux-aware I2C driver / `SharedI2cBus` wrapper.
*   [ ] Create the `Resources` and `Pins` structs to map the Tildagon hardware layout.
*   [ ] Implement the `split_resources!` macro.

### Phase 2: Refactor Initialization Internals Without Breaking the Public API
*   [ ] Update `TildagonHardware::new` to use the new `Resources` system internally while preserving its current return shape and behavior.
*   [ ] **CRITICAL**: Preserve our "Silence Pulsing Interrupts" logic for FUSB302B and BQ25895, including the required mux channel ordering and delays.
*   [ ] **CRITICAL**: Preserve "Secure USB Serial" logic (0x5a pin 4 LOW) early in initialization.
*   [ ] Preserve existing initialization side effects such as LED power enable and button-expander setup.

### Phase 3: Add New APIs Alongside Existing Ones
*   [ ] **Buttons**: Add typed-pin- and mux-aware internals first, but keep `Buttons::wait_for_press()` working unchanged for existing callers.
*   [ ] **Buttons**: Add a new async `wait_for_event()` / interrupt-driven API only after compatibility is preserved.
*   [ ] **LEDs**: Keep the current RMT-based LED driver API intact; only migrate any related expander-controlled setup (such as LED power control) to typed pins where it improves clarity.

### Phase 4: Opt-In Migration of Downstream Code
*   [ ] Update `embassy_blinky` only after the compatibility layer is proven.
*   [ ] Migrate downstream code incrementally to new resource-splitting and typed-pin APIs instead of forcing a flag day.

---

## What We Are NOT Adopting
*   We will **NOT** remove our FUSB302B/BQ25895 silencing logic. `tildagon-rs` appears to miss this, which can lead to unstable I2C behavior due to constant interrupt pulsing.
*   We will **NOT** switch to `defmt` yet if the user prefers standard `log` or `println`, though we should ensure compatibility with both.
*   We will **NOT** do a flag-day rewrite that breaks `embassy_blinky` in order to land architectural improvements.
