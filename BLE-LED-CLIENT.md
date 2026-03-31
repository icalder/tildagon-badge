# BLE LED Client Notes

This document describes the BLE interface exposed by `embassy_bluetooth_remote` for controlling the Tildagon badge LEDs from a client, including a Web Bluetooth client.

## Discovery

The badge advertises as a BLE peripheral.

- Advertised local name pattern: `Tildagon-XXXX`
- The `XXXX` suffix is derived from the badge BLE address and changes per device.
- The badge includes the LED service UUID in advertising data.

For Web Bluetooth, filtering by the service UUID is the most reliable approach.

## GATT Interface

- Service UUID: `12345678-1234-5678-1234-56789abcdef0`
- Characteristic UUID: `12345678-1234-5678-1234-56789abcdef1`
- Characteristic properties: `read`, `write`
- Characteristic value length: exactly `8` bytes

Reading the characteristic returns the last accepted command payload.

Writing the characteristic sends a new LED command.

## LED Model

- Number of LEDs: `19`
- Valid LED indices: `0..18`
- Color format: 8-bit RGB, one byte each for `r`, `g`, and `b`

## Command Encoding

All commands are exactly 8 bytes long.

### `0x00` Clear all LEDs

```text
[0x00, 0, 0, 0, 0, 0, 0, 0]
```

All remaining bytes must be zero.

### `0x01` Set one LED

```text
[0x01, led_index, r, g, b, 0, 0, 0]
```

Example: set LED 5 to green:

```text
[0x01, 0x05, 0x00, 0xff, 0x00, 0x00, 0x00, 0x00]
```

### `0x02` Fill all LEDs

```text
[0x02, r, g, b, 0, 0, 0, 0]
```

Example: fill all LEDs red:

```text
[0x02, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
```

### `0x03` Blink all LEDs

```text
[0x03, r, g, b, repeats, on_100ms, off_100ms, 0]
```

- `repeats`: `1..10`
- `on_100ms`: `1..100`
- `off_100ms`: `1..100`
- Actual timings are `value * 100ms`

Example: blink blue 3 times, on for 500ms, off for 300ms:

```text
[0x03, 0x00, 0x00, 0xff, 0x03, 0x05, 0x03, 0x00]
```

### `0x04` Chase pattern

```text
[0x04, r, g, b, rounds, step_10ms, 0, 0]
```

- `rounds`: `1..8`
- `step_10ms`: `1..100`
- Actual step delay is `value * 10ms`

Example: green chase for 2 rounds with 80ms per step:

```text
[0x04, 0x00, 0xff, 0x00, 0x02, 0x08, 0x00, 0x00]
```

## Validation Rules

The badge rejects writes that do not match the protocol.

Common rejection cases:

- payload length is not exactly 8 bytes
- unsupported opcode
- non-zero reserved bytes
- LED index outside `0..18`
- blink/chase repeat or timing fields outside the allowed ranges

In practice, the client should always construct commands with a fixed `Uint8Array(8)`.

## Web Bluetooth Notes

Web Bluetooth requires the service UUID to be declared up front when requesting the device.

Use:

- `filters: [{ services: ['12345678-1234-5678-1234-56789abcdef0'] }]`

Or, if you want broader discovery:

- `acceptAllDevices: true`
- `optionalServices: ['12345678-1234-5678-1234-56789abcdef0']`

The first approach is preferable.

## Web Bluetooth Example

```js
const LED_SERVICE_UUID = '12345678-1234-5678-1234-56789abcdef0';
const LED_CHARACTERISTIC_UUID = '12345678-1234-5678-1234-56789abcdef1';

async function connectBadge() {
  const device = await navigator.bluetooth.requestDevice({
    filters: [{ services: [LED_SERVICE_UUID] }],
  });

  const server = await device.gatt.connect();
  const service = await server.getPrimaryService(LED_SERVICE_UUID);
  const characteristic = await service.getCharacteristic(LED_CHARACTERISTIC_UUID);

  return { device, characteristic };
}

async function writeCommand(characteristic, bytes) {
  if (!(bytes instanceof Uint8Array) || bytes.length !== 8) {
    throw new Error('BLE LED command must be a Uint8Array(8)');
  }

  await characteristic.writeValue(bytes);
}

function clearAll() {
  return new Uint8Array([0x00, 0, 0, 0, 0, 0, 0, 0]);
}

function setLed(index, r, g, b) {
  if (index < 0 || index > 18) {
    throw new Error('LED index must be in range 0..18');
  }

  return new Uint8Array([0x01, index, r, g, b, 0, 0, 0]);
}

function fill(r, g, b) {
  return new Uint8Array([0x02, r, g, b, 0, 0, 0, 0]);
}

function blink(r, g, b, repeats, on100ms, off100ms) {
  return new Uint8Array([0x03, r, g, b, repeats, on100ms, off100ms, 0]);
}

function chase(r, g, b, rounds, step10ms) {
  return new Uint8Array([0x04, r, g, b, rounds, step10ms, 0, 0]);
}

async function readLastCommand(characteristic) {
  const value = await characteristic.readValue();
  return new Uint8Array(value.buffer.slice(0));
}
```

## Recommended Client Behavior

- Filter by service UUID, not by exact device name.
- Treat the characteristic as a strict binary protocol.
- Validate command lengths and ranges client-side before writing.
- If a write fails, reconnect and retry only after confirming the GATT connection still exists.
- Read the characteristic after connecting if you want to inspect the last accepted command.

## Included Demo Page

There is also a simple standalone browser demo in:

`pwa/ble-led-demo.html`

Open it from a secure origin such as `https://...` or `http://localhost`.

The demo is also set up as a minimal PWA via:

- `pwa/manifest.webmanifest`
- `pwa/sw.js`
- `pwa/icons/`

That makes it suitable for hosting on GitHub Pages and installing onto an Android phone.

## Mobile Browser Support

- Android Chrome / Edge: good Web Bluetooth support
- Desktop Chrome / Edge: good Web Bluetooth support
- iPhone / iPad Safari: no practical Web Bluetooth support for this use case

So the PWA install path is useful on Android, but it will not make iOS gain BLE support.

## Quick Test Commands

Set LED 0 to white:

```js
await writeCommand(characteristic, setLed(0, 255, 255, 255));
```

Fill all LEDs purple:

```js
await writeCommand(characteristic, fill(180, 0, 255));
```

Clear:

```js
await writeCommand(characteristic, clearAll());
```
