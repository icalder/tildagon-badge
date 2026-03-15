# Tildagon Badge Documentation - Programming

## The Badge

The badge has an ESP32S3 chip and a number of peripheral components including a display.  It also has six so-called hexpansion ports.  The SDK provided is in Python.

(Programming Interface Overview)[https://tildagon.badge.emfcamp.org/tildagon-apps/reference/reference/]

## Hardware Interfaces

### Display

For more information about the display search [badge-2024-software](./badge-2024-software/).  Hopefully it us a device supported by the Rust crates `display_interface_spi` and `embedded_graphics`.

[Python ctx interface](https://tildagon.badge.emfcamp.org/tildagon-apps/reference/ctx/) seems not to directly relate to the display hardware - it is a canvas abstraction.

### LEDs

LED hardware is aleady described in [HARDWARE](./HARDWARE.md).

[Python LEDs interface](https://tildagon.badge.emfcamp.org/tildagon-apps/reference/badge-hardware/#leds).

### Buttons

Button hardware is already described in [HARDWARE](./HARDWARE.md).

[Python Buttons interface](https://tildagon.badge.emfcamp.org/tildagon-apps/reference/badge-hardware/#buttons).

### Pins

Rust will use the `embassy` and `esp-hal`.

[Python Pins interface](https://tildagon.badge.emfcamp.org/tildagon-apps/reference/badge-hardware/#pins).

### IMU

The IMU device is a highly integrated, low power inertial measurement unit (IMU) that combines precise acceleration and angular rate (gyroscopic) measurement. The triple axis device has been configured to measure 2g and 2 degree per second ranges. It also has a step count function intended for wrist mounted applications.

[Python IMU interface](https://tildagon.badge.emfcamp.org/tildagon-apps/reference/badge-hardware/#imu).

### Power

The Python package allows one to perform multiple battery related functions, like powering off the badge or getting the battery level.

[Python Power interface](https://tildagon.badge.emfcamp.org/tildagon-apps/reference/badge-hardware/#power).
