# Tildagon Badge Hardware

This document records the hardware details of the Tildagon badge, focusing on what's needed for Rust firmware development with Embassy.

## LED Subsystem

The Tildagon badge has 19 individually addressable RGB LEDs (Neopixels/WS2812) in a single serial chain.

### Hardware Mapping

| Chain Index | Physical Location | Notes |
|---|---|---|
| **0** | **Base Board (Center)** | Single LED in the middle of the badge. |
| **1 – 12** | **Top Board Ring** | 12 LEDs forming the hexagonal ring. |
| **13 – 18** | **Hexpansion Slots** | 6 LEDs (one per slot) for expansion boards. |

### Technical Details

- **Data Pin:** `GPIO21`.
- **Power Control:** Controlled by pin 2 of the AW9523B at `0x5a`. This pin must be driven HIGH to enable the 5V supply for the LED chain.
- **Initialization:** Perform an explicit `clear()` (sending all zeros) immediately after enabling power. This ensures the RMT peripheral and the WS2812B chain are synchronized.

### ⚠️ Driver Stability (ESP32-S3)

For reliable operation of the full 19-LED chain on the ESP32-S3, the following implementation details are **mandatory**:

1. **Use Blocking Mode:** The RMT driver MUST use blocking mode (`esp_hal::Blocking`). In async mode, the Embassy executor can yield during the RMT RAM refill interrupt to run high-priority tasks (like WiFi/BLE). If this takes >50µs, the RMT hardware wraps around its limited RAM (384 pulses), causing corrupted addressing and repeating colors.
2. **Contiguous Buffer:** Send the entire 19-LED frame in a single RMT transaction. 
3. **Explicit Reset Pulse:** Encode a ~300µs "Low" period (e.g., using multiple `PulseCode::new(Level::Low, ...)` entries) at the end of every RMT frame to ensure the WS2812B chips correctly latch the data.

## I2C Bus

The badge has a primary I2C bus used for controlling various peripherals.

- **Pins:**
    - `SDA`: `GPIO45`
    - `SCL`: `GPIO46`
- **Reset Pin:** `GPIO9` is used to enable/reset I2C peripherals. It should be set to `High`.
- **I2C Mux:**
    - **Address:** `0x77`
    - **System Bus Channel:** Channel 7. To communicate with the onboard peripherals, you must first write `1 << 7` to the mux.

### How I2C, the mux, devices, and "pins" fit together

`I2C` is a **serial communication bus**, not a bundle of per-signal GPIO wires. The ESP32-S3 talks over the shared `SDA`/`SCL` lines to multiple chips, each identified by an I2C address such as `0x5a` or `0x6a`.

Those chips are not all the same kind of thing:

- Some are normal peripheral ICs, like the `BQ25895` charger at `0x6a`.
- Some are **GPIO expanders**, like the `AW9523B` chips at `0x58`, `0x59`, and `0x5a`.

A GPIO expander is an I2C device that provides extra digital input/output lines. In other words, the chip itself sits on the serial I2C bus, but the chip exposes multiple logical GPIO "pins" that firmware can read or drive by accessing the chip's registers.

The TCA9548A mux at `0x77` adds one more level: it selects which downstream branch of the bus is active. In Rust terms, this is why it is useful to distinguish:

- a **mux port** such as `Port0` or `Port7`
- an **I2C device address** such as `0x5a`
- a **pin on that device** such as pin `4`

So a typed pin like `Pin<0x5A, Port::Port0, 4>` means:

- talk through mux channel `Port0`
- to the I2C device at address `0x5A`
- and use GPIO pin `4` on that device

ASCII view:

```text
ESP32-S3
  |
  |  I2C bus (serial: SDA=GPIO45, SCL=GPIO46)
  |
  +--> TCA9548A mux @ 0x77
         |
         +--> Port 0
         |     |
         |     +--> FUSB302B @ 0x22
         |
         +--> Port 7
               |
               +--> AW9523B @ 0x58  --> expander pins 0..15
               +--> AW9523B @ 0x59  --> expander pins 0..15
               +--> AW9523B @ 0x5A  --> expander pins 0..15
               +--> BQ25895  @ 0x6A
               +--> FUSB302B @ 0x22
```

Concrete examples:

- `0x5a` pin `2` controls LED power.
- `0x5a` pin `4` controls the USB mux select line.
- `0x5a` pins `6` and `7` are buttons A and B.
- `0x59` pins `0` through `3` are buttons C through F.

This is why the code often needs three pieces of information at once: **which mux channel**, **which I2C device**, and **which pin on that device**.

## I/O Expanders (AW9523B)

There are three `AW9523B` I/O expander chips on the system I2C bus. These are used for driving LEDs, reading buttons, and controlling hexpansion ports.

- **Addresses:** `0x58`, `0x59`, `0x5a`.
- **LED Power Pin:** The power for the 19 Neopixel LEDs is controlled by **pin 2** of the `AW9523B` at address **`0x5a`**. This pin must be configured as an output and driven `high` to enable the LEDs.

### ⚠️ CRITICAL: USB Serial Communication Issue with 0x5a

**The Issue:** Initializing the AW9523B at address `0x5a` can break USB serial communication during firmware startup.

**Root Cause:** The AW9523B soft reset command (register `0x7F`) returns all pins to high-impedance (input) state momentarily. Pin 4 of `0x5a` controls the **USB mux** — while it is floating/high-Z, the USB mux routes USB away from the ESP32, causing the host to drop the serial connection permanently.

**Pin map for `0x5a` (from official firmware source `tildagon_pin.c`):**
- **Pin 2** = 5V supply switch / LED power (`EPIN_LED_POWER = (2, 2)` in Python)
- **Pin 4** = USB mux select (LOW = USB to ESP32, HIGH = USB to hexpansion)
- **Pin 5** = secondary LED switch

**The Fix:** Do **not** soft-reset `0x5a`. Instead, write the output register **before** the direction register so that when pin 4 becomes an output, it immediately drives LOW. This must happen as early as possible after I2C is up — before the USB host times out.

```rust
// Enable mux channel 7, then immediately configure 0x5a pin 4 LOW:
let _ = i2c.write(0x77u8, &[1 << 7]);
// Write output register first (pin 4 will be LOW when direction changes)
let _ = i2c.write(0x5au8, &[0x02, 0x00]);
// Set pins 2, 4, 5 as outputs, rest as inputs (~(1<<2|1<<4|1<<5) = 0xCB)
let _ = i2c.write(0x5au8, &[0x04, 0xCB, 0xFF]);
// Pin 4 is now output LOW — USB routes to ESP32.
```

After this, init `0x58` and `0x59` normally (soft reset is safe on those). Then configure `0x5a` further **without** the soft reset step, and enable LED power (pin 2 HIGH).

**What NOT to do:**
- ❌ Do not write `[0x7F, 0x00]` to `0x5a` — this floats pin 4 and kills USB
- ❌ Do not add long delays (100ms+) after GPIO9 goes HIGH before writing to `0x5a`

### AW9523B Initialization Sequence

For proper operation, each AW9523B chip must be initialized with the following register writes (in order):

1. **Reset:** Write `0x00` to register `0x7F` (chip reset)
2. **Interrupt Disable (Port 0 & 1):** Write `0xFF, 0xFF` to register `0x06` (disables all interrupts; bit = 1 means disabled)
3. **Direction (Port 0 & 1):** Write `0xFF, 0xFF` to register `0x04` (sets all pins as inputs - counter-intuitive but required for proper GPIO operation)
4. **Control Register (GCR):** Write `0x10` to register `0x11` (configures global control settings)
5. **Interrupt Mask (All 16 pins):** Write `0x00` × 16 to register `0x20` (clears interrupt masks for all pins)

After this initialization, individual pins can be:
- Read from register `0x00` (Port 0) or `0x01` (Port 1) for input state
- Written to register `0x02` (Port 0) or `0x03` (Port 1) for output state
- Set direction with register `0x04` (Port 0) or `0x05` (Port 1) where `0x00` = output, `0xFF` = input

## Power & USB-C Management

The badge features two USB-C PD controllers and a dedicated battery management IC. All three of these devices share the `GPIO10` interrupt line.

### USB-C PD Controllers (FUSB302B)

- **Address:** `0x22`
- **Location 1 (usb_in):** Mux Channel 7. Handles power input and negotiation with the host/charger.
- **Location 2 (usb_out):** Mux Channel 0. Handles power delivery to hexpansions.
- **Interrupts:** Without initialization, these chips often assert `GPIO10` continuously due to CC pin state changes or internal events. To silence them in "Hello World" style firmware:
    - Perform a soft reset (Write `0x01` to reg `0x0C`).
    - Power off the oscillator (Write `0x00` to reg `0x0B`).
    - Mask all interrupts (Registers `0x0A`, `0x0E`, `0x0F`).

### Battery Management IC (BQ25895)

- **Address:** `0x6A`
- **Location:** Mux Channel 7.
- **Functions:** Handles battery charging, ADC measurements (`VBAT`, `VSYS`, `VBUS`, charging current), and 5V boost for hexpansions.
- **Not a fuel gauge:** This chip does **not** provide a true state-of-charge counter. Any battery "percentage" shown in firmware is an estimate derived from voltage/current heuristics.
- **Watchdog / interrupts:** The chip has a hardware watchdog that asserts the shared interrupt line if not disabled. It also raises interrupts for charger and power-path events, but these are **event** notifications, not continuous battery-level updates.
- **Silence / startup procedure:** 
    - Reset (Write `0x80` to reg `0x14`).
    - Apply the same 4-register setup block used by the original firmware (Write `0x60, 0x10, 0x18, 0x00` starting at reg `0x02`).
    - Then write `0x8C` to reg `0x07` to disable the watchdog and configure ADC/charger behaviour.
    - Read a status register afterward to clear any pending interrupt state if needed.

#### Reading battery and power measurements

The original badge firmware reads an 8-byte block starting at register `0x0B`:

- `0x0B` = charger status
- `0x0C` = fault status
- `0x0E` = `VBAT`
- `0x0F` = `VSYS`
- `0x11` = `VBUS`
- `0x12` = charging current

In other words, a single `write_read(0x6A, &[0x0B], &mut buf)` can fetch the key status and ADC values needed for UI or logging.

The useful conversions found in the original firmware are:

```rust
let raw_vbat = buf[3] & 0x7F;   // reg 0x0E
let raw_vsys = buf[4] & 0x7F;   // reg 0x0F
let raw_vbus = buf[6] & 0x7F;   // reg 0x11
let raw_ichg = buf[7] & 0x7F;   // reg 0x12

let vbat_volts = if raw_vbat == 0 { 0.0 } else { raw_vbat as f32 * 0.02 + 2.304 };
let vsys_volts = if raw_vsys == 0 { 0.0 } else { raw_vsys as f32 * 0.02 + 2.304 };
let vbus_volts = if raw_vbus == 0 { 0.0 } else { raw_vbus as f32 * 0.10 + 2.600 };
let charge_current_amps = raw_ichg as f32 * 0.05;
```

The charge-status bits live in `reg 0x0B` mask `0x18`:

- `0x00` = not charging
- `0x08` = pre-charging
- `0x10` = fast charging
- `0x18` = charge terminated / charged

#### Estimated battery percentage

The original firmware estimates battery level from `VBAT`, charge state, and charging current. It uses different ranges for charging vs. discharging because the BQ25895 is not a coulomb counter:

- **Discharging / not charging:** map roughly `3.5V .. 4.14V` to `0% .. 100%`
- **Charging, constant-current phase:** map roughly `3.6V .. 4.2V` to the first ~`80%`
- **Charging, constant-voltage phase:** infer the last ~`20%` from the tapering charging current

This is good enough for a UI indicator, but should be treated as an approximation rather than a calibrated battery gauge.

#### Interrupts vs polling

The BQ25895 interrupt output is shared on `GPIO10` with the FUSB302B PD controllers and button-related activity. In practice:

- interrupts are useful for "something changed" events such as plug/unplug, fault, or charge-state transitions
- interrupts do **not** eliminate the need to read the BQ25895 registers to learn the new values
- interrupts are **not** a replacement for periodic battery UI updates, because the chip does not interrupt on every small voltage/percentage change

For display code, the most practical pattern is:

- refresh immediately when the shared interrupt fires and a power event is suspected
- also poll slowly (for example every `2-10s`) for on-screen battery information

## Memory Layout (Internal SRAM)

The ESP32-S3 has a single pool of internal SRAM shared between all static allocations **and** the task stack. There is no separate stack region — the stack occupies whatever SRAM is left after `.bss`/`.data` sections are placed.

### Major static consumers (embassy_wifi_ble)

| Allocation | Size | Defined in |
|---|---|---|
| Radio heap (`DEFAULT_RADIO_HEAP_SIZE`) | 112 KB (tunable) | `tildagon/src/radio.rs` |
| SPI DMA buffers (`dma_buffers!`) | 20,480 B (tunable) | `tildagon/src/display.rs` |
| `StripeBuffer` (StaticCell) | 19,200 B (240×40×2) | `embassy_wifi_ble/src/main.rs` |
| SPI interface buffer (`DISPLAY_BUFFER`) | 4,096 B | `embassy_wifi_ble/src/main.rs` |

Everything above lives in BSS; every byte claimed there shrinks the stack by one byte.

### Measured values (release build, WiFi + BLE scanning active)

| Measurement point | Stack used | Stack free | Heap used | Heap free |
|---|---|---|---|---|
| `main-start` | 5,216 B | 57,268 B | — | — |
| `after-radio-init` | 5,216 B | 57,268 B | 148 B | ~114,500 B |
| `display-init` (before loop) | 20,032 B | 42,452 B | ~82,352 B | ~32,300 B |
| `display-loop` (steady state) | 20,032 B | 42,452 B | ~82,600 B | ~32,000 B |

Total stack size at these settings: **62,484 B** (~61 KB).  
The embassy thread-mode executor is cooperative — all tasks share this one stack.

The jump from `main-start` to `display-init` (≈14,800 B) is the display task frame before its first `.await`, which includes the `TildagonDisplay` parameter on the stack. Heap jumps from 148 B to ~82 KB at `display-init` because BLE/WiFi stacks allocate their runtime structures on first use.

### Tuning

`DEFAULT_RADIO_HEAP_SIZE` (`tildagon/src/radio.rs`) and the `dma_buffers!` size (`tildagon/src/display.rs`) are the two knobs. Reducing either grows the stack by the same amount. The radio heap **cannot** safely drop much below 112 KB with WiFi + BLE coex active — peak observed usage is ~83 KB, leaving ~32 KB headroom for BLE scan results and transient WiFi scan `Vec` allocations.

### Stack / heap diagnostic snippet

```rust
#![feature(asm_experimental_arch)]

unsafe extern "C" {
    static _stack_end_cpu0: u32;
    static _stack_start_cpu0: u32;
}

#[inline(never)]
fn log_stack(label: &str) {
    let sp: u32;
    unsafe { core::arch::asm!("mov {}, a1", out(reg) sp) }
    let top = core::ptr::addr_of!(_stack_start_cpu0) as u32;
    let bot = core::ptr::addr_of!(_stack_end_cpu0) as u32;
    println!("[STACK:{label}] total={} used={} free={}", top - bot, top - sp, sp - bot);
}

#[inline(never)]
fn log_heap(label: &str) {
    println!("[HEAP:{label}] {}", esp_alloc::HEAP.stats());
}
```

Call `log_stack` / `log_heap` at key startup points and periodically inside the display loop to confirm headroom.

### ⚠️ Stack overflow symptom

The `esp-rtos` scheduler enables hardware stack-overflow detection by default. Crossing the stack guard triggers:

```
Detected a write to the stack guard value on ProCpu
```

followed by a hang or panic. If this appears, reduce static allocations (primarily `DEFAULT_RADIO_HEAP_SIZE`) to give the stack more room, and audit large locals or deep call chains in async tasks.



The Tildagon badge has **six buttons arranged around the hexagon shape**, connected via the AW9523B I/O expanders at addresses `0x59` and `0x5a`. These buttons provide user input for applications.

### Button Layout and Hardware Mapping

| Button | Position | Name | GPIO (Chip, Pin) | I2C Expander | Port | Function |
|--------|----------|------|------------------|--------------|------|----------|
| A | Top | UP | (2, 6) | `0x5a` | 0 | Navigate up / Pan up |
| B | Top-Right | RIGHT | (2, 7) | `0x5a` | 0 | Navigate right / Pan right |
| C | Bottom-Right | CONFIRM | (1, 0) | `0x59` | 0 | Confirm selection / Execute action |
| D | Bottom | DOWN | (1, 1) | `0x59` | 0 | Navigate down / Pan down |
| E | Bottom-Left | LEFT | (1, 2) | `0x59` | 0 | Navigate left / Pan left |
| F | Top-Left | CANCEL | (1, 3) | `0x59` | 0 | Go back / Exit application |

**GPIO Notation:** `(chip, pin)` where:
- Chip `0` = `AW9523B` at I2C address `0x58` (hexpansion control)
- Chip `1` = `AW9523B` at I2C address `0x59` (buttons D, E, F and hexpansion ports 4-6)
- Chip `2` = `AW9523B` at I2C address `0x5a` (buttons A, B and hexpansion ports 1-3)

**All button pins are on Port 0** of their respective expanders, with active-low logic (0 = pressed, 1 = released).

### Button Input Detection

Button states are accessed by reading the GPIO input registers of the AW9523B expanders:
- **Port 0 Input Register:** I2C register address `0x00` on each expander
- **Port 1 Input Register:** I2C register address `0x01` on each expander

Reading these registers returns the current state of all pins on that port (1 = released/HIGH, 0 = pressed/LOW).

For button detection, read register `0x00` from both `0x59` and `0x5a`:

```rust
// Read Port 0 from both expanders
let mut port0_59 = [0u8; 1];  // Buttons C, D, E, F
let mut port0_5a = [0u8; 1];  // Buttons A, B
i2c.read(0x59u8, &mut port0_59)?;
i2c.read(0x5au8, &mut port0_5a)?;

let button_a_pressed = (port0_5a[0] & (1 << 6)) == 0;  // Bit 6
let button_b_pressed = (port0_5a[0] & (1 << 7)) == 0;  // Bit 7
let button_c_pressed = (port0_59[0] & (1 << 0)) == 0;  // Bit 0
let button_d_pressed = (port0_59[0] & (1 << 1)) == 0;  // Bit 1
let button_e_pressed = (port0_59[0] & (1 << 2)) == 0;  // Bit 2
let button_f_pressed = (port0_59[0] & (1 << 3)) == 0;  // Bit 3
```

### Button Detection

The hardware provides a shared interrupt line on `GPIO10` that asserts when any button changes state, but in practice this line is also shared with the FUSB302B PD controllers and BQ25895, making it unreliable as a sole trigger — especially during radio activity. The firmware therefore uses **polling**.

**Polling approach (used by this firmware):**
- A background task reads port 0 of both `0x59` and `0x5a` every 20ms
- Compares with the previous read to detect press (HIGH→LOW) or release (LOW→HIGH)
- Publishes `ButtonEvent::Pressed` / `ButtonEvent::Released` to a `PubSubChannel`
- Callers subscribe to the channel; the 20ms poll interval naturally debounces contact bounce

Note: all GPIO expander interrupts are masked during hardware init (`0xFF, 0xFF` to register `0x06` on each chip), so the shared `GPIO10` line is not used for buttons.

### Debouncing

The 20ms poll interval naturally suppresses contact bounce without additional software debounce logic. If tighter response is needed, reduce the interval (minimum ~10ms before I2C bus contention becomes a concern during WiFi/BLE activity).

### Hexagon Expansion Integration

The six buttons also control hexagon expansion insertion/removal detection when **held for 4+ seconds**:

- **Button A (pin 6)** → Hexpansion **Port 1** insert/remove
- **Button B (pin 7)** → Hexpansion **Port 2** insert/remove
- **Button C (pin 0)** → Hexpansion **Port 3** insert/remove
- **Button D (pin 1)** → Hexpansion **Port 4** insert/remove
- **Button E (pin 2)** → Hexpansion **Port 5** insert/remove
- **Button F (pin 3)** → Hexpansion **Port 6** insert/remove

When a button is held for 4+ seconds with the "boop" pin (GPIO 0) pulled LOW:
- After 4 seconds: Triggers a **hexpansion insertion event** on the corresponding port
- On release: Triggers a **hexpansion removal event**
- Normal button press/release (< 4 seconds): Standard button events only

### Button Functionality Conventions

While buttons can be used for any purpose, the standard conventions are:
- **CONFIRM (C):** Primary action / select menu item
- **CANCEL (F):** Go back / exit application
- **UP (A) / DOWN (D):** Navigate menus / adjust values
- **LEFT (E) / RIGHT (B):** Secondary navigation / side panels
