# Tildagon Badge ESP-RS Development Environment

> [!IMPORTANT]
> This Nix-based development environment and rustc sandboxing logic is derived from the [esp-rust-nix-sandbox](https://github.com/michalrus/esp-rust-nix-sandbox) project by Michal Rus, licensed under the Apache License, Version 2.0.

This project provides a sandboxed Rust development environment for the ESP32-S3 (Xtensa) using Nix and Bubblewrap (`bwrap`).

## Prerequisites

### 1. NixOS System Configuration
To use the security sandbox, you must add the following to your `/etc/nixos/configuration.nix` (or equivalent system config):

```nix
security.wrappers.bwrap = {
  owner = "root";
  group = "root";
  source = "${pkgs.bubblewrap}/bin/bwrap";
  setuid = true;
};
```

After adding this, run `sudo nixos-rebuild switch`.

### 2. Enter the Environment
Run the following command in the project root:
```bash
nix develop
```

## Upstream Sources & Updating Versions

The toolchain is managed by `nix/unsafe-bin.nix`. It pulls pre-compiled binaries from official Espressif and ESP-RS repositories.

### 1. ESP Rust Toolchain (`rustc`, `rustdoc`, `rust-src`)
*   **Source:** [github.com/esp-rs/rust-build](https://github.com/esp-rs/rust-build)
*   **How to Update:** 
    *   The version is automatically derived from `pkgs.rustc.version` in your `nixpkgs` input.
    *   If you switch to a newer `nixpkgs` (e.g., `unstable`), you must update the `hash` values for `x86_64-linux`, `aarch64-linux`, etc., in the `rust` and `rust-src` sections of `nix/unsafe-bin.nix`.

### 2. ESP GCC (Xtensa & RISC-V)
*   **Source:** [github.com/espressif/crosstool-NG](https://github.com/espressif/crosstool-NG)
*   **How to Update:** 
    *   Change the `version` string in the `esp-gcc` block of `nix/unsafe-bin.nix` (e.g., `15.2.0_20250920`).
    *   Update the corresponding `sha256` hashes for each architecture and target.

### 3. ESP GDB (Debugger)
*   **Source:** [github.com/espressif/binutils-gdb](https://github.com/espressif/binutils-gdb)
*   **How to Update:**
    *   Change the `version` string in the `esp-gdb` block of `nix/unsafe-bin.nix` (e.g., `16.3_20250913`).
    *   Update the `sha256` hashes.

## Project Structure

*   `flake.nix`: Entry point for the Nix environment.
*   `nix/unsafe-bin.nix`: Logic for fetching and patching the "unsafe" (pre-compiled) binaries.
*   `nix/safe-bwrap.nix`: Bubblewrap sandbox configuration to isolate the toolchain.
*   `nix/devshell.nix`: Defines the `nix develop` shell, tools (`espflash`, `cargo`), and environment variables.
*   `nix/help.nix`: Provides the MOTD and verification scripts.

## Troubleshooting

### "Failed to run bwrap" error
This warning often appears during `nix develop` startup if the environment variables aren't fully initialized. If `rustc --version` works once you are inside the shell, you can safely ignore it.

### Version Mismatch
If you get a 404 error during `nix develop`, it means the ESP-RS team hasn't yet released a version of Rust matching your current `nixpkgs`. You may need to pin your `nixpkgs` input in `flake.nix` to a slightly older version.
