# Button Interrupt Debugging — Status & Next Steps

## Hardware Overview

Six buttons connect to two AW9523B I2C GPIO expanders. All button interrupts funnel into a single
shared open-drain interrupt line (GPIO10), which is also shared with the FUSB302B USB PD controller.

| Button | Chip   | Port 0 Bit | I2C Address |
|--------|--------|-----------|-------------|
| A (UP)     | 0x5a | bit 6 | Port 0 reg 0x00 |
| B (RIGHT)  | 0x5a | bit 7 | Port 0 reg 0x00 |
| C (CONFIRM)| 0x59 | bit 0 | Port 0 reg 0x00 |
| D (DOWN)   | 0x59 | bit 1 | Port 0 reg 0x00 |
| E (LEFT)   | 0x59 | bit 2 | Port 0 reg 0x00 |
| F (CANCEL) | 0x59 | bit 3 | Port 0 reg 0x00 |

**GPIO10** — open-drain, active-low, wired-OR across:
- AW9523B at 0x58 (hexpansion control — no buttons, all interrupts masked)
- AW9523B at 0x59 (buttons C/D/E/F, interrupt mask 0xF0 = bits 0-3 enabled)
- AW9523B at 0x5a (buttons A/B, interrupt mask 0x3F = bits 6-7 enabled)
- FUSB302B USB PD controller at 0x22

**AW9523B interrupt semantics:** register 0x06 = interrupt mask for Port 0. Bit = 0 means the pin
WILL assert INT on change; bit = 1 means DISABLED. INT is cleared by reading the input register
(0x00 for Port 0, 0x01 for Port 1). Active-low: button pressed = bit is 0.

## Implementation in `embassy_blinky/src/main.rs`

The main task sets up an `esp_hal::gpio::Input` on GPIO10 with pull-up and calls
`wait_for_falling_edge().await` in a loop. On each falling edge:
1. Immediately checks `is_high()` — if already High, it's a spurious phantom edge (I2C crosstalk) and
   is skipped.
2. 20ms debounce delay.
3. Reads Port 0 + Port 1 from 0x58, 0x59, 0x5a in sequence, checking `is_low()` before each chip
   (mirrors the reference firmware's `while (GPIO10==0)` → `tildagon_pins_generate_isr()` pattern).
4. If GPIO10 still Low after all AW9523B reads, reads FUSB302B interrupt registers (0x3E, 0x42, 0x40)
   to clear any USB PD interrupt.
5. XOR against last state to detect press (HIGH→LOW) and release (LOW→HIGH) transitions.

**AW9523B interrupt mask registers are confirmed correct at boot** — readback prints verify:
```
[INIT] 0x58 int mask: port0=0xFF(exp 0xFF) port1=0xFF(exp 0xFF) OK
[INIT] 0x59 int mask: port0=0xF0(exp 0xF0) port1=0xFF(exp 0xFF) OK
[INIT] 0x5a int mask: port0=0x3F(exp 0x3F) port1=0xFF(exp 0xFF) OK
```

## Current Symptom

With no buttons pressed, GPIO10 is oscillating. It goes Low, stays Low for **less than 20ms**, goes
High, then falls again — continuously, ~10-100x per second. This is confirmed by the output:

```
[BUTTON] GPIO10 initial level: High (expect High)
[BUTTON] Entering main loop, waiting for falling edge...
[BUTTON] GPIO10 Low — debouncing...    ← GPIO10 genuinely Low (not spurious)
[BUTTON] GPIO10 Low — debouncing...    ← no chip reads fire (GPIO10 went High during 20ms wait)
[BUTTON] GPIO10 Low — debouncing...    ← back to Low again, next falling edge caught
... (repeats indefinitely)
```

Key facts:
- Passes the `is_high()` spurious-edge filter → GPIO10 is genuinely Low on entry each time
- Zero chip read lines print → GPIO10 goes High again within 20ms (before reads happen)
- No buttons are pressed during this
- Steady-state I2C register values seen in earlier iterations: 0x59=0x0F, 0x5a=0xC4

## What Has Already Been Tried

1. **Polling every 5s** — worked (detected button C) but not interrupt-driven
2. **`wait_for_falling_edge()`** — catches the edge but loops
3. **`wait_for_low()`** — causes spin loop (level-triggered, races on re-assertion)
4. **Boot dummy reads** — all three AW9523B chips, both ports, plus FUSB302B registers
5. **Single-register I2C writes** for mask setup (matching reference C firmware pattern)
6. **AW9523B interrupt mask readback** — confirmed all correct values
7. **Reading only Port 0 from 0x59 and 0x5a** — still loops (INT re-asserts after reads)
8. **Reading all three chips, both ports, per-chip gating on GPIO10** — still loops

## Hypotheses (Most Likely First)

### 1. FUSB302B is generating continuous USB PD interrupts (MOST LIKELY)

The FUSB302B at 0x22 is the USB PD negotiation controller. It sits on the same I2C mux channel 7
(system bus) and shares GPIO10. It is **not initialised at all** in the Rust firmware — only the
AW9523B chips are set up.

The reference C firmware (`tildagon_power.c`) runs a full FUSB302B init sequence. Without this,
the FUSB302B is likely in a reset or uninitialised state and continuously asserting INT because it
has pending events (VBUS detection, CC pin state changes, etc.) that nobody is reading.

**Evidence:** GPIO10 pulses with a ~10-20ms duty cycle. The boot-clear step reads FUSB302B registers
once, but if the FUSB302B is continuously generating new events (not just clearing a static pending
flag), the boot clear is insufficient.

**To verify:** Disable all three AW9523B interrupts entirely (write 0xFF to all 0x06 registers on
all chips) and check if GPIO10 still pulses. If it does, FUSB302B (or something else) is the culprit.

### 2. AW9523B 0x58 or 0x59/0x5a Port 1 pins are noisy

0x58 controls hexpansion ports. Its input pins may be floating (no pull-up, open-drain, no expansion
connected) and oscillating. Although mask register 0x58:0x06=0xFF (all disabled), some hardware
designs have a chip-level INT status that is separate from the per-pin mask.

The value 0x59 port0=0x0F seen consistently suggests bits 4-7 are always low (pulled down by
unconnected hexpansion pads). These bits have interrupt enabled (mask=0xF0 = bits 0-3 disabled,
bits 4-7 **enabled**).

**Wait — this may be the bug:** `mask 0xF0` means bits 4-7 are ENABLED (0 = enabled, 1 = disabled).
The HARDWARE.md documents `(1, 0)-(1, 3)` as the button pins, so we only want bits 0-3 enabled.
`0xF0` = `1111_0000` = bits 0-3 disabled, bits 4-7 **enabled** — this is backward!

**Actually this might be correct:** re-read the datasheet. For AW9523B, bit=0 in reg 0x06 enables
that pin's interrupt, bit=1 disables it.
- Buttons are on bits 0-3 → we want those ENABLED → those bits should be 0
- Non-button bits 4-7 → we want those DISABLED → those bits should be 1
- So `0xF0` = `1111_0000` = bits 4-7 disabled (1), bits 0-3 enabled (0) ✓ **This is correct.**

### 3. esp-hal GPIO edge detection is misfiring on noise

The ESP32-S3 GPIO interrupt hardware has minimum pulse width requirements. Very short glitches
(< ~100ns) can still be latched by the interrupt controller. If something is generating narrow
pulses on GPIO10 (e.g. I2C bus capacitive coupling), these could trigger `wait_for_falling_edge()`
even though the pin appears High when read immediately after.

However, the `is_high()` check at the top of the loop filters this — and we confirmed it's not
filtering (GPIO10 is genuinely Low). So this hypothesis is less likely.

## Most Promising Next Steps

### Step 1: Isolate FUSB302B as the source

Add this diagnostic block at boot, after all AW9523B reads, to disable ALL AW9523B interrupts
and check if GPIO10 stays High:

```rust
// TEST: disable all AW9523B interrupts and see if GPIO10 stays High
for addr in [0x58u8, 0x59u8, 0x5au8] {
    let _ = i2c.write(addr, &[0x06, 0xFF]); // all Port 0 masked
    let _ = i2c.write(addr, &[0x07, 0xFF]); // all Port 1 masked
}
delay.delay_millis(100);
esp_println::println!("[TEST] All AW9523B ints disabled, GPIO10={}", 
    if gpio10_pin.is_high() { "High (FUSB302B is culprit)" } else { "Low (AW9523B culprit)" });
```

- **GPIO10=High** → FUSB302B (or something else) is the source → initialise FUSB302B
- **GPIO10=Low** → An AW9523B is asserting INT despite the mask → re-enable one chip at a time

### Step 2 (if FUSB302B is culprit): Initialise FUSB302B

The reference firmware initialises the FUSB302B with specific register writes. Key init registers
found in `badge-2024-software/drivers/tildagon_power/fusb302b/fusb302b.c`:

- FUSB302B I2C address: **0x22**
- Init write starting at register 0x01, 13 bytes (resets device)
- Second init write starting at register 0x02, 15 bytes (configures PD operation)

The simplest approach: mask all FUSB302B interrupts. Write `0xFF` to its interrupt mask register
to stop it from asserting INT until a proper PD stack is running. Check the FUSB302B datasheet
for the interrupt mask register address (likely 0x0F or 0x08).

### Step 3 (if AW9523B Port 1 or 0x58 is culprit): Fix interrupt mask

If disabling 0x58's interrupts doesn't help, check Port 1 of 0x59 and 0x5a. The hexpansion
detection uses Port 1 pins — if any are floating and oscillating, and their interrupt is enabled,
they will fire continuously. Ensure Port 1 masks on 0x59 and 0x5a are truly 0xFF.

### Step 4 (fallback): Use polling instead of interrupt-driven

As documented in HARDWARE.md, polling every 20-50ms is simpler and avoids all INT complications.
Change the `wait_for_falling_edge().await` loop to a `Timer::after(Duration::from_millis(50)).await`
loop that simply reads Port 0 from 0x59 and 0x5a every 50ms and XORs against last state.
This was already working (button C detected) in the original polling code.

## Reference Firmware

`badge-2024-software/drivers/tildagon_power/tildagon_power.c` — full GPIO10 ISR and FUSB302B init
`badge-2024-software/drivers/tildagon_power/fusb302b/fusb302b.c` — FUSB302B driver
`badge-2024-software/drivers/tildagon_pin/aw9523b.c` — AW9523B driver (ISR handler reads 0x00, 0x01)
`badge-2024-software/drivers/tildagon_pin/tildagon_pin.c` — chip iterator loop checking GPIO10

## Key esp-hal APIs

```rust
// GPIO interrupt-driven
use esp_hal::gpio::{Input, InputConfig, Pull};
let mut pin = Input::new(peripherals.GPIO10, InputConfig::default().with_pull(Pull::Up));
pin.wait_for_falling_edge().await;   // edge-triggered (requires unstable feature)
pin.wait_for_low().await;            // level-triggered (causes spin loop — avoid)
pin.is_high() / pin.is_low()         // immediate level read

// I2C
i2c.write_read(addr, &[reg], &mut buf)  // write register address, then read
i2c.write(addr, &[reg, val])            // write register + value
```

Cargo.toml requires: `esp-hal = { version = "1.0.0", features = ["unstable"] }`
