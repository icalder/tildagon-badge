# Rust Tildagon Crate

## Summary

What's my dream? To have a Rust crate that makes programming the EMF Camp Tildagon badge in Rust as easy as using the official Python packages.  Performance will be better thanks to Rust and the use of the [embassy](https://github.com/embassy-rs/embassy) async framework.

We should try and follow the same functions as the Python interfaces, so that programmers familiar with prgramming the badge using Python have an easy transition.  But we should also follow Rust best practices if there is any conflict.

## What we have already

We have some [HARDWARE](./HARDWARE.md) docs.
We have [Python device programming](./PYTHON-PROGRAMMING.md) docs.
We have a working "Hello World and blink LEDs" program in [embassy-blinky](./embassy_blinky/).

## Other sources of information

Make full use of the cratesio mcp server to look up information about Rust crates including available versions and the types and functions they contain.

## The Plan

### Step 1 - core hardware initialisation [COMPLETED]

The new crate should contain a module for hardware init.  Any common init or general purpose hardware init functions should go here.  When complete, the aim is to refactor `enbassy_blinky` to depend on it and replace the inline coding.

### Step 2 - LEDs [COMPLETED]

Let's create an LEDs module.  Initially it should offer sufficent utilty to allow the `embassy_blinky` program to start using it instead of the inline coding.

### Step 3 - Buttons [COMPLETED]

Let's create a buttons module.  Initially it should offer sufficent utilty to allow the `embassy_blinky` program to start using it instead of the inline coding.

### Additional Improvements [COMPLETED]
- **Error Handling**: Implemented `crate::Error` and updated all modules (`hardware`, `leds`, `buttons`) to return `Result`.
- **Workspace Setup**: Created a root `Cargo.toml` workspace to unify `tildagon` and `embassy_blinky` for better LSP support and build management.