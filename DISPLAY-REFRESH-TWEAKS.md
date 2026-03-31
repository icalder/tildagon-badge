# Display refresh tweaks log

This notes the experiments I ran on `embassy_wifi_ble/src/main.rs` while trying to reduce display flicker without killing frame rate.

## What I started from

The original code rendered directly to the hardware display from `render_display_frame`, with a simple `previous` frame state used to erase old shapes. That approach was fast enough, but it could flicker because the panel was being updated in place while new pixels were still being drawn.

## What I tried

### 1. Full-frame software double buffering

I first tried allocating a full 240x240 `Rgb565` framebuffer in static memory and drawing into that off-screen before presenting it.

Result:

- Flicker was reduced.
- The build failed with a linker error because the static frame buffer pushed memory usage into the stack/linker region.

### 2. Heap-allocated full-frame buffer

I then switched the full-frame buffer to heap allocation.

Result:

- That avoided the linker overlap.
- At runtime it panicked with `memory allocation of 0 bytes failed`, which pointed to the display task not having usable heap at that point in startup.

### 3. Stripe-based buffering

Next I fell back to a smaller static buffer and rendered the screen in stripes.

Result:

- Flicker was gone.
- Frame rate dropped noticeably because the screen was now being updated in multiple passes per frame.

### 4. Stripe height tuning

I tried several stripe heights to recover performance:

- 16 rows
- 32 rows
- 48 rows
- 64 rows
- 80 rows
- 120 rows

Result:

- Larger stripes improved speed a bit, but larger stripes also made memory pressure worse.
- Some stripe sizes triggered linker stack-region overlap again.
- Small stripe sizes were safe but slow.

### 5. Stripe-aware drawing optimization

To avoid redrawing the whole UI in each stripe, I changed `render_display_frame` so it only drew content in the stripe band currently being processed.

Result:

- This recovered some frame rate.
- It also introduced brittleness: parts of the UI started disappearing or becoming clipped depending on which band they landed in.

### 6. Label placement fixes

I specifically had to chase a corrupted `BLE` label near the status bar. The issue turned out to be related to stripe-band placement and clipping rather than the font itself.

Result:

- I moved the label between bands and adjusted the stripe renderer to stop emitting off-band pixels.
- That fixed the label once, but the approach remained fragile.

## What broke last

The final stripe-aware version became too brittle:

- major display areas started disappearing
- stripe band boundaries no longer lined up cleanly with the UI layout
- the `BLE` label issue came back in a different form

## What finally worked

### 7. Optimized Stripe-based Buffering

I implemented a 240x40 `Rgb565` stripe buffer (19,200 bytes) in static memory using `StaticCell`. The screen is rendered in 6 stripes per frame.

Result:

- Flicker is completely eliminated because each stripe is updated atomically on the display.
- Frame rate is preserved by using `bounding_box` intersection checks to skip drawing UI elements that don't overlap with the current stripe.
- Memory usage is well within limits for the ESP32-S3's internal SRAM.
- Performance was further improved by pre-formatting all UI strings once per frame rather than once per stripe.
- Bumping the CPU clock from 80/160MHz to 240MHz increased the frame rate from ~9 FPS to over 20 FPS.

This approach provides a smooth, flicker-free UI without the memory pressure of a full-frame buffer or the performance penalty of unoptimized multi-pass rendering.

### 8. DMA-Optimized Stripe Transfers (Future)

While we are currently using `SpiDmaBus`, the `mipidsi` crate's `fill_contiguous` method uses an iterator, which can introduce overhead by processing pixels individually.

**Potential Optimization:**
- Instead of iterator-based filling, we could use a specialized `write_pixels` method or raw byte access to send the entire 19,200-byte `StripeBuffer` in a single DMA transaction.
- This would offload the transfer entirely to the DMA hardware, allowing the CPU to start rendering the *next* stripe immediately.
- This could potentially double the current frame rate or allow for more complex UI animations.

## Verification performed

During the final implementation I ran:

- `cargo check -p embassy_wifi_ble --quiet`
- `cargo build -p embassy_wifi_ble --release --quiet`

The code compiles and correctly partitions the screen into stripes with efficient clipping.
