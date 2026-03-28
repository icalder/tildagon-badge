# WiFi and BLE Support Plan for Tildagon (Revised)

This plan outlines the steps and architecture for adding WiFi and Bluetooth Low Energy (BLE) support to the Tildagon badge using the Rust `esp-hal` 1.0 ecosystem and Embassy.

## 1. Core Dependencies

To support WiFi and BLE with Embassy, the following crates should be added to the `tildagon` crate's `Cargo.toml`:

### Radio Stack
- **`esp-radio`**: The low-level driver for WiFi/BLE.
  - Features needed: `esp32s3`, `wifi`, `ble`, `coex`, `log-04`, `unstable`, `esp-alloc`.
- **`esp-rtos`**:
  - Features needed: `embassy`, `esp-alloc`, `esp-radio`, `log-04`.

### WiFi Stack
- **`embassy-net`**: The Embassy network stack for TCP/UDP and DHCP.
  - Features needed: `tcp`, `udp`, `dhcpv4`, `medium-ethernet`, `proto-ipv4`.
- **`smoltcp`**: Used by `esp-radio` for WiFi interfaces.

### BLE Stack
- **`trouble-host`**: The modern async BLE host stack for Embassy.
- **`bt-hci`**: Used by `esp-radio` and `trouble-host` for HCI communication.

### Support
- **`esp-alloc`**: `esp-radio` requires a heap for internal driver allocations.
- **`static_cell`**: For managing static storage of radio controllers.

## 2. Hardware Resource Refactor

The `tildagon` crate now keeps radio setup separate from base hardware setup.

### Changes to `tildagon/src/resources.rs`
The `RadioResources` group should include the peripherals required by `esp-radio`'s `wifi::new` and `ble::new` (or `BleConnector::new`).

```rust
radio: RadioResources<'d> {
    wifi: WIFI,
    bt:   BT,
}
```

### Changes to `tildagon/src/hardware.rs`
`TildagonHardware` now owns non-radio peripherals and retains radio peripherals privately until `init_radio()` is called.

```rust
pub struct TildagonHardware {
    // ...
    radio_res: Option<crate::resources::RadioResources<'static>>,
}
```

The dedicated radio path is represented by `tildagon::radio::TildagonRadio`, which owns the shared `esp_radio::Controller` plus the WiFi/BLE peripherals.

## 3. Initialization Workflow

The radio is initialized only when a client opts in to it.

1.  **Base Hardware Initialization:** Call `TildagonHardware::new(...)`.
2.  **Radio Initialization:** Call `tildagon.init_radio()` when WiFi/BLE is needed.

The allocator is now hidden behind the `tildagon` radio initialization path, using a crate-owned default heap size.

```rust
let mut tildagon = TildagonHardware::new(peripherals).await?;
let mut radio = tildagon.init_radio()?;
```

Internally:

```rust
static RADIO_CELL: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
let controller = init_radio_controller()?;
let radio = TildagonRadio::new(controller, radio_resources);
```

## 4. WiFi Implementation Strategy

Initialize WiFi through `TildagonRadio`.

```rust
let (wifi_controller, wifi_interfaces) = radio.init_wifi(Default::default())?;
```

## 5. BLE Implementation Strategy

Client functionality is sufficient - the ability to scan for BLE devices and receive data from them.

`trouble-host` provides an async-native host stack; the badge example currently uses central scanning, not BLE peripheral mode.

```rust
let connector = radio.init_ble_connector(Default::default())?;
let controller: ExternalController<_, 1> = ExternalController::new(connector);
```

## 6. Memory Considerations

- **Heap:** `tildagon::radio` owns the default heap setup required before `esp_radio::init()`.
- **Internal RAM:** WiFi buffers must reside in internal RAM.
- **Randomness:** Runtime network seed and BLE random address should come from hardware RNG, not fixed demo values.

## 7. Migration Status

- **Phase 1: Basic Radio Init (DONE).** Radio initialization now lives behind `TildagonHardware::init_radio()`.
- **Phase 2: Fix 'static Requirement (DONE).** The shared controller remains `StaticCell`-backed inside `tildagon::radio`.
- **Phase 3: WiFi Station / Scan Example (DONE FOR SCAN PATH).** `embassy_wifi_ble` initializes WiFi through `TildagonRadio`; current demo still runs scan-only WiFi logic.
- **Phase 4: BLE Peripheral (IN PROGRESS).** Planning GATT service for LED control and advertising logic.

## 8. BLE Peripheral Mode

To support the remote LED control use case, the badge must operate as a BLE peripheral.

### GATT Service Definition
A custom GATT service should be defined using `trouble-host`'s `#[gatt_server]` / `#[gatt_service]` macros so the service layout stays explicit in the application crate while still reusing `trouble-host`'s generated GAP/GATT plumbing.

- **Service UUID:** A unique 128-bit UUID for the Tildagon LED Service.
- **LED Control Characteristic:**
  - **Properties:** Read, Write.
  - **Value:** A fixed-width command payload so reads always return the last accepted command and writes can be validated deterministically without heap allocation.
  - **Initial payload shape (8 bytes):**
    - `0x00`: Clear all LEDs. Remaining bytes must be zero.
    - `0x01`: Set one LED: `[0x01, led_index, r, g, b, 0, 0, 0]`.
    - `0x02`: Fill all LEDs: `[0x02, r, g, b, 0, 0, 0, 0]`.
    - `0x03`: Blink all LEDs: `[0x03, r, g, b, repeats, on_100ms, off_100ms, 0]`.
    - `0x04`: Chase pattern: `[0x04, r, g, b, rounds, step_10ms, 0, 0]`.
  - **Validation:** Reject malformed payload lengths, out-of-range LED indices, and unsupported opcodes with ATT errors instead of silently ignoring them.

### Advertising Strategy
The badge will advertise using `AdData`:
- **Flags:** General Discoverable, BR/EDR Not Supported.
- **Complete Local Name:** "Tildagon-XXXX" (where XXXX is a unique suffix from the MAC address).
- **Service UUIDs:** Include the custom LED Service UUID.

The application should keep advertising payloads pre-encoded in fixed buffers so the peripheral loop can restart advertising immediately after a disconnect without reallocating temporary state.

### Connection Handling
The `peripheral` task will:
1.  **Advertise:** Call `peripheral.advertise(...)` with configured `AdData`.
2.  **Connect:** Await `peripheral.accept_connection(...)`.
3.  **Process GATT:** Convert the connection into a `GattConnection` with the attribute server attached, then listen for read/write events on the LED characteristic.
4.  **Reconnect:** Return to the advertising state when the central disconnects or the GATT task exits.

Write handling should be split into two stages:
- **Protocol stage:** Parse and validate the incoming command bytes in the BLE task.
- **Hardware stage:** Forward validated commands to a dedicated LED task that owns `TypedLeds`.

This avoids sharing the LED driver between button logic and BLE logic and keeps all RMT/I2C LED access serialized in one place.

### Memory and Tasks
- **GATT Table:** Statically allocated or built into the stack.
- **Task Management:** The BLE runner, peripheral/advertising loop, button handler, and LED driver task should run as separate Embassy tasks.
- **LED Ownership:** A channel/pubsub queue should connect producers (button task, BLE task) to the single LED task.
- **Command Boundedness:** Pattern commands should have bounded durations/iteration counts so a bad client write cannot lock the LED task into an excessively long animation.
- **Characteristic Storage:** Keep the readback value inside the GATT server, not in a parallel ad-hoc buffer.

### Error Handling and Observability
- Log advertising start, connection establishment, disconnect reasons, read requests, and rejected writes.
- Reject invalid writes with precise ATT errors where possible (`INVALID_ATTRIBUTE_VALUE_LENGTH`, `INVALID_PDU`, or `UNLIKELY_ERROR` as appropriate).
- If applying a validated LED command fails at the hardware layer, log the failure and preserve the BLE connection rather than panicking.
- Clear the LEDs on startup and after disconnect if the last command represented a transient pattern.

### Crate Refactoring
To keep `main.rs` clean and promote reuse across multiple BLE-enabled apps:
- Common, generic BLE stack initialization helpers (with sensible defaults and random address generation) will be added to the `tildagon` crate.
- App-specific GATT definitions (like the test LED Service) will remain in the application code.

For this phase, the reusable extraction target is:
- random BLE address generation / badge-name derivation,
- stack + runner bring-up helpers,
- shared advertise helper for a connectable/scannable peripheral with a custom name and service UUID.

### Connection parameter quirk observed with Web Bluetooth

During testing with Web Bluetooth clients, the badge initially proved unstable when connected with one set of BLE connection parameters and then asked by the central to switch to another. A practical workaround was to request the client's preferred values immediately on connect. The firmware currently defaults to the more phone-friendly values we observed first (`10 ms` interval, `latency 0`, `5 s` supervision timeout), but testing showed that this is not a stable universal answer: one phone/browser flow later renegotiated from `10 ms` to `30 ms`, and another test starting from `30 ms` was renegotiated back down to `10 ms`.

This should be treated as a compatibility workaround rather than the ideal final design. Different phones or browsers may request different values, and `trouble-host 0.4.x` on this stack does not currently give us a clean, proven way to handle arbitrary remote connection parameter requests on the peripheral side without running into lower-level `bt-hci` / controller issues. One practical blocker to simply bumping the host stack is that the current published `esp-radio 0.17.x` line is tied to `bt-hci 0.6.x`, while newer `trouble-host` releases want newer `bt-hci`, so a proper upgrade likely means moving more of the esp-rs stack together rather than changing a single crate in isolation. Future BLE work should revisit this area with either a newer `trouble-host` version or a more robust peripheral-side connection-parameter handling strategy.

## Quick note: possible BLE WiFi setup service

If we later want phone-driven WiFi provisioning, the cleanest approach is probably a separate BLE service rather than extending the fixed-width LED command characteristic. A simple first pass would use distinct characteristics for SSID, password, and connection status, plus a write-only "apply/connect" trigger characteristic. For Web Bluetooth this is much easier than designing a chunked packet protocol, but it should only be used with BLE pairing/bonding plus encrypted GATT access, or equivalent app-layer protection, because it would otherwise expose the WiFi password over an unauthenticated BLE link. Persisting credentials will also require some non-volatile storage strategy on the badge.
