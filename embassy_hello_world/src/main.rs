//! Tildagon Badge LED Control with USB Support
//!
//! This firmware initializes the Tildagon badge LEDs while maintaining
//! USB serial communication. Button presses trigger LED animation sequences.
//!
//! Architecture:
//! - Main loop: Awaits GPIO10 falling-edge interrupt (shared button line),
//!   reads I2C expanders to identify which button changed, sends to tasks
//! - button_monitor task: Awaits button press signal
//! - blinky task: Runs LED animation when triggered
//! - Communication: embassy-sync PubSubChannel (broadcast events)

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{PubSubChannel, Subscriber};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::I2c;
use esp_hal::rmt::Rmt;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async};
use smart_leds::{
    SmartLedsWriteAsync,
    colors::{self, *},
};
use static_cell::StaticCell;

esp_bootloader_esp_idf::esp_app_desc!();

#[embassy_executor::task]
async fn run() {
    loop {
        esp_println::println!("Hello world from embassy!");
        Timer::after(Duration::from_millis(3_000)).await;
    }
}

const NUM_LEDS: usize = 19;

#[embassy_executor::task]
async fn button_monitor(mut sub: Subscriber<'static, CriticalSectionRawMutex, (), 1, 2, 1>) {
    esp_println::println!("[BUTTON_MONITOR] Ready, awaiting button press...");

    // Wait for real button press signal from main loop
    loop {
        sub.next_message_pure().await;
        esp_println::println!("[BUTTON_MONITOR] Button C (CONFIRM) pressed!");
    }
}

#[embassy_executor::task]
async fn blinky(
    mut led: SmartLedsAdapterAsync<'static, { buffer_size_async(NUM_LEDS) }>,
    mut sub: Subscriber<'static, CriticalSectionRawMutex, (), 1, 2, 1>,
) {
    esp_println::println!("[BLINKY] Waiting for button C (CONFIRM) press to start animation...");

    loop {
        // This awaits until button_monitor sends an event - no polling, no busy loop
        sub.next_message_pure().await;

        esp_println::println!("[BLINKY] Starting LED animation...");
        let colors = [RED, GREEN, BLUE, YELLOW, MAGENTA, CYAN, WHITE];

        // Flash each color once at 50% brightness
        for color in colors {
            let dim: smart_leds::RGB8 = smart_leds::RGB8 {
                r: color.r / 2,
                g: color.g / 2,
                b: color.b / 2,
            };
            let data = [dim; NUM_LEDS];
            if let Err(e) = led.write(data.iter().cloned()).await {
                esp_println::println!("LED write error: {:?}", e);
            }
            Timer::after(Duration::from_millis(500)).await;
        }

        // Turn LEDs off
        let data = [colors::BLACK; NUM_LEDS];
        if let Err(e) = led.write(data.iter().cloned()).await {
            esp_println::println!("LED write error: {:?}", e);
        }
        Timer::after(Duration::from_millis(500)).await;

        esp_println::println!("[BLINKY] Animation complete, LEDs off");
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    esp_println::logger::init_logger_from_env();

    let delay = esp_hal::delay::Delay::new();

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(
        timg0.timer0,
        #[cfg(target_arch = "riscv32")]
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT),
    );

    // Tildagon Badge I2C Reset/Enable Pin (GPIO 9) - releases expanders from reset.
    // Use minimal delay: pin 4 of 0x5a (USB mux) is high-Z while the chip is in reset
    // or has not been configured. We must drive it LOW as fast as possible.
    let _i2c_reset = Output::new(peripherals.GPIO9, Level::High, OutputConfig::default());
    delay.delay_millis(5); // Reduced from 100ms - just enough for I2C bus to settle

    // Tildagon Badge I2C initialization (SDA=GPIO45, SCL=GPIO46)
    let mut i2c = I2c::new(peripherals.I2C0, esp_hal::i2c::master::Config::default())
        .expect("I2C init failed")
        .with_sda(peripherals.GPIO45)
        .with_scl(peripherals.GPIO46);

    // Enable I2C Mux Channel 7 (System Bus)
    if let Err(e) = i2c.write(0x77u8, &[1 << 7]) {
        log::warn!("I2C mux (0x77) enable failed: {:?}", e);
    }

    // --- CRITICAL: SECURE USB SERIAL IMMEDIATELY ---
    // The 0x5a AW9523B pin 4 controls the USB mux. We must drive it LOW
    // as fast as possible to prevent the host from dropping the connection.
    // Write output register first (pin 4 = LOW), then direction register.
    if let Err(e) = i2c.write(0x5au8, &[0x02, 0x00]) {
        log::error!("0x5a output reg failed (USB mux unstable!): {:?}", e);
    }
    // Set direction: pins 2, 4, 5 as outputs (bit = 0), rest as inputs (bit = 1)
    if let Err(e) = i2c.write(0x5au8, &[0x04, 0xCB, 0xFF]) {
        log::error!("0x5a direction reg failed (USB mux unstable!): {:?}", e);
    }
    // USB Mux is now locked to ESP32. We can safely proceed with other inits.

    // --- SILENCE PULSING INTERRUPTS ---
    // GPIO10 is shared with two FUSB302B PD controllers (Port 0 and Port 7)
    // and a BQ25895 PMIC (Port 7).
    let mut dummy = [0u8; 2];

    // 1. Silence FUSB302B on Mux Port 0 (usb_out)
    if i2c.write(0x77u8, &[1 << 0]).is_err() {
        log::warn!("Mux port 0 select failed");
    }
    delay.delay_millis(2);
    if i2c.write(0x22u8, &[0x0C, 0x01]).is_err() {
        log::warn!("FUSB 0x22 (p0) reset failed");
    }
    delay.delay_millis(2);
    if i2c.write(0x22u8, &[0x0B, 0x00]).is_err() {
        log::warn!("FUSB 0x22 (p0) osc off failed");
    }
    if i2c.write(0x22u8, &[0x04, 0x02]).is_err() {
        log::warn!("FUSB 0x22 (p0) gmask failed");
    }
    if i2c.write(0x22u8, &[0x0A, 0xFF]).is_err() {
        log::warn!("FUSB 0x22 (p0) mask failed");
    }
    if i2c.write(0x22u8, &[0x0E, 0xFF]).is_err() {
        log::warn!("FUSB 0x22 (p0) maskA failed");
    }
    if i2c.write(0x22u8, &[0x0F, 0xFF]).is_err() {
        log::warn!("FUSB 0x22 (p0) maskB failed");
    }
    if i2c.write_read(0x22u8, &[0x3E], &mut dummy).is_err() {
        log::warn!("FUSB 0x22 (p0) clear INTAB failed");
    }
    if i2c.write_read(0x22u8, &[0x42], &mut dummy[..1]).is_err() {
        log::warn!("FUSB 0x22 (p0) clear INT failed");
    }

    // 2. Silence BQ25895 and FUSB302B on Mux Port 7 (system)
    if i2c.write(0x77u8, &[1 << 7]).is_err() {
        log::warn!("Mux port 7 select failed");
    }
    delay.delay_millis(2);

    // BQ25895: disable watchdog and ADC continuous mode
    if i2c.write(0x6Au8, &[0x14, 0x80]).is_err() {
        log::warn!("BQ25895 reset failed");
    }
    delay.delay_millis(10);
    if i2c.write(0x6Au8, &[0x02, 0x00]).is_err() {
        log::warn!("BQ25895 ADC cfg failed");
    }
    if i2c.write(0x6Au8, &[0x07, 0x8C]).is_err() {
        log::warn!("BQ25895 WDT off failed");
    }
    if i2c.write_read(0x6Au8, &[0x0B], &mut dummy).is_err() {
        log::warn!("BQ25895 clear INT failed");
    }

    // FUSB302B (usb_in): silence interrupts
    if i2c.write(0x22u8, &[0x0C, 0x01]).is_err() {
        log::warn!("FUSB 0x22 (p7) reset failed");
    }
    delay.delay_millis(2);
    if i2c.write(0x22u8, &[0x0B, 0x00]).is_err() {
        log::warn!("FUSB 0x22 (p7) osc off failed");
    }
    if i2c.write(0x22u8, &[0x04, 0x02]).is_err() {
        log::warn!("FUSB 0x22 (p7) gmask failed");
    }
    if i2c.write(0x22u8, &[0x0A, 0xFF]).is_err() {
        log::warn!("FUSB 0x22 (p7) mask failed");
    }
    if i2c.write(0x22u8, &[0x0E, 0xFF]).is_err() {
        log::warn!("FUSB 0x22 (p7) maskA failed");
    }
    if i2c.write(0x22u8, &[0x0F, 0xFF]).is_err() {
        log::warn!("FUSB 0x22 (p7) maskB failed");
    }
    if i2c.write_read(0x22u8, &[0x3E], &mut dummy).is_err() {
        log::warn!("FUSB 0x22 (p7) clear INTAB failed");
    }
    if i2c.write_read(0x22u8, &[0x42], &mut dummy[..1]).is_err() {
        log::warn!("FUSB 0x22 (p7) clear INT failed");
    }

    // Full init for 0x58 and 0x59 (safe: no USB-sensitive pins on these chips)
    for addr in [0x58u8, 0x59u8] {
        if i2c.write(addr, &[0x7F, 0x00]).is_err() {
            log::warn!("0x{:02x} reset failed", addr);
        } // Soft reset
        delay.delay_millis(2);
        if i2c.write(addr, &[0x06, 0xFF, 0xFF]).is_err() {
            log::warn!("0x{:02x} mask failed", addr);
        } // Mask all interrupts (P0 & P1)
        if i2c.write(addr, &[0x04, 0xFF, 0xFF]).is_err() {
            log::warn!("0x{:02x} dir failed", addr);
        } // All pins as inputs
        if i2c.write(addr, &[0x11, 0x10]).is_err() {
            log::warn!("0x{:02x} GCR failed", addr);
        } // GCR: push-pull mode
    }

    // Additional 0x5a config — NO soft reset (would float pin 4 and break USB!)
    // Output and direction registers were already set early above to secure the USB mux.
    if i2c.write(0x5au8, &[0x06, 0xFF, 0xFF]).is_err() {
        log::warn!("0x5a mask failed");
    } // Mask interrupts
    if i2c.write(0x5au8, &[0x11, 0x10]).is_err() {
        log::warn!("0x5a GCR failed");
    } // GCR: push-pull mode
    delay.delay_millis(2);

    // Enable LED power: 0x5a pin 2 HIGH (5V supply for NeoPixels)
    if let Err(e) = i2c.write(0x5au8, &[0x02, 1 << 2]) {
        log::error!("0x5a LED power enable failed: {:?}", e);
    }

    // Clear ALL pending interrupts before entering the loop.
    // GPIO10 is an open-drain, wired-OR line shared by the three AW9523B expanders
    // AND the FUSB302B USB power controller (0x22). Any device with unread interrupt
    // events holds GPIO10 low. Read input registers from every device to fully
    // de-assert INT.
    {
        let mut dummy = [0u8; 1];
        // AW9523B: read Port 0 and Port 1 from all three chips
        for addr in [0x58u8, 0x59u8, 0x5au8] {
            if i2c.write_read(addr, &[0x00], &mut dummy).is_err() {
                log::warn!("0x{:02x} clear P0 failed", addr);
            }
            if i2c.write_read(addr, &[0x01], &mut dummy).is_err() {
                log::warn!("0x{:02x} clear P1 failed", addr);
            }
        }
        // FUSB302B: read interrupt and status registers (registers 0x3E, 0x42, 0x40)
        let mut fus2 = [0u8; 2];
        if i2c.write_read(0x22u8, &[0x3E], &mut fus2).is_err() {
            log::warn!("FUSB 0x22 clear INTAB failed");
        }
        if i2c.write_read(0x22u8, &[0x42], &mut fus2[..1]).is_err() {
            log::warn!("FUSB 0x22 clear INT failed");
        }
        if i2c.write_read(0x22u8, &[0x40], &mut fus2[..1]).is_err() {
            log::warn!("FUSB 0x22 clear STATUS failed");
        }
    }

    // GPIO10 is the shared interrupt line for all six buttons (active-low,
    // falls when any button is pressed).
    let mut button_int = Input::new(
        peripherals.GPIO10,
        InputConfig::default().with_pull(Pull::Up),
    );

    // --- RE-ENABLE BUTTON INTERRUPTS ---
    // 0x59 Port 0 bits 0-3 = buttons C, D, E, F  →  mask 0xF0 (unmask 0-3)
    if let Err(e) = i2c.write(0x59u8, &[0x06, 0xF0]) {
        log::error!("0x59 port0 int enable failed: {:?}", e);
    }
    // 0x5a Port 0 bits 6-7 = buttons A, B  →  mask 0x3F (unmask 6-7)
    if let Err(e) = i2c.write(0x5au8, &[0x06, 0x3F]) {
        log::error!("0x5a port0 int enable failed: {:?}", e);
    }
    // All other ports stay masked (0xFF) as set in the previous block.
    delay.delay_millis(10);

    esp_println::println!(
        "[INIT] AW9523B interrupts re-enabled for buttons, GPIO10={}",
        if button_int.is_high() {
            "High (Expected)"
        } else {
            "Low (Still noisy!)"
        }
    );

    esp_println::println!("Boot: I2C done, 0x5a pin 4 held LOW throughout, USB should be stable");

    spawner.spawn(run()).ok();

    // Create channel for button press detection (broadcast to multiple tasks)
    // PubSubChannel: capacity=1, subscribers=2, publishers=1
    static BUTTON_CHANNEL: StaticCell<PubSubChannel<CriticalSectionRawMutex, (), 1, 2, 1>> =
        StaticCell::new();
    let channel = BUTTON_CHANNEL.init(PubSubChannel::new());

    // Spawn button monitor - awaits real button presses
    spawner
        .spawn(button_monitor(channel.subscriber().unwrap()))
        .ok();

    // NOW set up LED hardware and spawn blinky
    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80))
        .unwrap()
        .into_async();
    let rmt_channel = rmt.channel0;

    static RMT_BUFFER: StaticCell<[esp_hal::rmt::PulseCode; buffer_size_async(NUM_LEDS)]> =
        StaticCell::new();
    let rmt_buffer =
        RMT_BUFFER.init([esp_hal::rmt::PulseCode::default(); buffer_size_async(NUM_LEDS)]);

    let led = SmartLedsAdapterAsync::new(rmt_channel, peripherals.GPIO21, rmt_buffer);

    // Spawn blinky - also awaits button press from the same channel
    spawner
        .spawn(blinky(led, channel.subscriber().unwrap()))
        .ok();

    esp_println::println!(
        "[BUTTON] GPIO10 initial level: {} (expect High)",
        if button_int.is_high() { "High" } else { "Low" }
    );

    // Main loop: suspend until GPIO10 falls (any button event), then read I2C
    // to identify which button(s) changed state.
    let mut last_state_59: u8 = 0xFF;
    let mut last_state_5a: u8 = 0xFF;
    let publisher = channel.publisher().unwrap();

    esp_println::println!("[BUTTON] Entering main loop, waiting for falling edge...");
    loop {
        button_int.wait_for_falling_edge().await;

        if button_int.is_high() {
            // Spurious edge (likely I2C noise).
            continue;
        }

        // Debounce
        Timer::after(Duration::from_millis(20)).await;
        if button_int.is_high() {
            // Pulse was too short to be a button press.
            continue;
        }

        // --- CLEAR INTERRUPTS ---
        // We must iterate through all chips and ports to clear INT.
        // If GPIO10 is still Low after some reads, it means another chip
        // is holding it down or re-asserted.

        let mut port0_58 = [0u8; 1];
        let mut port0_59 = [last_state_59; 1];
        let mut port1_59 = [0u8; 1];
        let mut port0_5a = [last_state_5a; 1];
        let mut port1_5a = [0u8; 1];
        let mut dummy = [0u8; 2];

        // 1. Check chips on Port 7 (System)
        if i2c.write(0x77u8, &[1 << 7]).is_err() {
            log::warn!("Mux port 7 select failed");
        }
        if button_int.is_low() {
            if i2c.write_read(0x58u8, &[0x00], &mut port0_58).is_err() {
                log::warn!("0x58 clear P0 failed");
            }
            if i2c.write_read(0x58u8, &[0x01], &mut port0_58).is_err() {
                log::warn!("0x58 clear P1 failed");
            }
        }
        if button_int.is_low() {
            if i2c.write_read(0x59u8, &[0x00], &mut port0_59).is_err() {
                log::warn!("0x59 clear P0 failed");
            }
            if i2c.write_read(0x59u8, &[0x01], &mut port1_59).is_err() {
                log::warn!("0x59 clear P1 failed");
            }
        }
        if button_int.is_low() {
            if i2c.write_read(0x5au8, &[0x00], &mut port0_5a).is_err() {
                log::warn!("0x5a clear P0 failed");
            }
            if i2c.write_read(0x5au8, &[0x01], &mut port1_5a).is_err() {
                log::warn!("0x5a clear P1 failed");
            }
        }
        if button_int.is_low() {
            if i2c.write_read(0x6Au8, &[0x0B], &mut dummy).is_err() {
                log::warn!("BQ25895 clear INT failed");
            }
            if i2c.write_read(0x22u8, &[0x3E], &mut dummy).is_err() {
                log::warn!("FUSB (p7) clear INTAB failed");
            }
            if i2c.write_read(0x22u8, &[0x42], &mut dummy[..1]).is_err() {
                log::warn!("FUSB (p7) clear INT failed");
            }
        }

        // 2. Check chips on Port 0 (USB Out)
        if button_int.is_low() {
            if i2c.write(0x77u8, &[1 << 0]).is_err() {
                log::warn!("Mux port 0 select failed");
            }
            if i2c.write_read(0x22u8, &[0x3E], &mut dummy).is_err() {
                log::warn!("FUSB (p0) clear INTAB failed");
            }
            if i2c.write_read(0x22u8, &[0x42], &mut dummy[..1]).is_err() {
                log::warn!("FUSB (p0) clear INT failed");
            }
            // Return to Port 7 for next loop
            if i2c.write(0x77u8, &[1 << 7]).is_err() {
                log::warn!("Mux port 7 return failed");
            }
        }

        // XOR against last state to detect transitions.
        let changed_59 = port0_59[0] ^ last_state_59;
        let pressed_59 = !port0_59[0] & changed_59;
        if pressed_59 & (1 << 0) != 0 {
            esp_println::println!("[BUTTON] Button C (CONFIRM) pressed!");
            // Use immediate publish so we don't block the interrupt loop
            publisher.publish_immediate(());
        }
        if pressed_59 & (1 << 1) != 0 {
            esp_println::println!("[BUTTON] Button D (DOWN) pressed!");
        }
        if pressed_59 & (1 << 2) != 0 {
            esp_println::println!("[BUTTON] Button E (LEFT) pressed!");
        }
        if pressed_59 & (1 << 3) != 0 {
            esp_println::println!("[BUTTON] Button F (CANCEL) pressed!");
        }

        let changed_5a = port0_5a[0] ^ last_state_5a;
        let pressed_5a = !port0_5a[0] & changed_5a;
        if pressed_5a & (1 << 6) != 0 {
            esp_println::println!("[BUTTON] Button A (UP) pressed!");
        }
        if pressed_5a & (1 << 7) != 0 {
            esp_println::println!("[BUTTON] Button B (RIGHT) pressed!");
        }

        last_state_59 = port0_59[0];
        last_state_5a = port0_5a[0];

        // If we still see Low, there might be a high-frequency oscillation.
        // Adding a small delay here prevents a tight spin-loop that starves other tasks.
        if button_int.is_low() {
            Timer::after(Duration::from_millis(10)).await;
        }
    }
}
