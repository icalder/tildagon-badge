//! Tildagon-specific AW9523B expander pin assignments.
//!
//! Each field is a zero-sized [`Pin`] token whose address, port, and bit number
//! are encoded as const generic parameters, making wrong-device bugs impossible
//! at compile time.
//!
//! # Attribution
//! Hardware mapping derived from
//! [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).

use crate::pins::{Pin, aw9523b::Port};

/// All I2C expander GPIO pins on the Tildagon badge.
pub struct Pins {
    pub button:             ButtonPins,
    pub led:                LedPins,
    pub other:              OtherPins,
    pub hexpansion_detect:  HexpansionDetectPins,
    pub top_board:          TopBoardPins,
}

/// The six user-facing buttons (A–F).
pub struct ButtonPins {
    /// Button A — AW9523B 0x5A port0 pin6
    pub btn_a: Pin<0x5A, { Port::Port0 }, 6>,
    /// Button B — AW9523B 0x5A port0 pin7
    pub btn_b: Pin<0x5A, { Port::Port0 }, 7>,
    /// Button C — AW9523B 0x59 port0 pin0
    pub btn_c: Pin<0x59, { Port::Port0 }, 0>,
    /// Button D — AW9523B 0x59 port0 pin1
    pub btn_d: Pin<0x59, { Port::Port0 }, 1>,
    /// Button E — AW9523B 0x59 port0 pin2
    pub btn_e: Pin<0x59, { Port::Port0 }, 2>,
    /// Button F — AW9523B 0x59 port0 pin3
    pub btn_f: Pin<0x59, { Port::Port0 }, 3>,
}

/// LED subsystem control pins.
pub struct LedPins {
    /// WS2812B power enable — AW9523B 0x5A port0 pin2 (HIGH = on)
    pub power_enable: Pin<0x5A, { Port::Port0 }, 2>,
}

/// Miscellaneous control and status pins.
pub struct OtherPins {
    /// VBUS switch — AW9523B 0x5A port0 pin4
    pub vbus_sw:    Pin<0x5A, { Port::Port0 }, 4>,
    /// USB mux select — AW9523B 0x5A port0 pin5
    pub usb_select: Pin<0x5A, { Port::Port0 }, 5>,
    /// Accelerometer interrupt — AW9523B 0x58 port0 pin1
    pub accel_int:  Pin<0x58, { Port::Port0 }, 1>,
}

/// Hexpansion slot detection pins (HIGH when a board is inserted).
pub struct HexpansionDetectPins {
    pub a: Pin<0x5A, { Port::Port1 }, 4>,
    pub b: Pin<0x5A, { Port::Port1 }, 5>,
    pub c: Pin<0x59, { Port::Port1 }, 0>,
    pub d: Pin<0x59, { Port::Port1 }, 1>,
    pub e: Pin<0x59, { Port::Port1 }, 2>,
    pub f: Pin<0x59, { Port::Port1 }, 3>,
}

/// Top-board level-shift control pins.
pub struct TopBoardPins {
    pub ls_1: Pin<0x5A, { Port::Port1 }, 7>,
    pub ls_2: Pin<0x5A, { Port::Port1 }, 6>,
}

impl Pins {
    pub fn new() -> Self {
        Self {
            button: ButtonPins {
                btn_a: Pin::new(),
                btn_b: Pin::new(),
                btn_c: Pin::new(),
                btn_d: Pin::new(),
                btn_e: Pin::new(),
                btn_f: Pin::new(),
            },
            led: LedPins {
                power_enable: Pin::new(),
            },
            other: OtherPins {
                vbus_sw:    Pin::new(),
                usb_select: Pin::new(),
                accel_int:  Pin::new(),
            },
            hexpansion_detect: HexpansionDetectPins {
                a: Pin::new(),
                b: Pin::new(),
                c: Pin::new(),
                d: Pin::new(),
                e: Pin::new(),
                f: Pin::new(),
            },
            top_board: TopBoardPins {
                ls_1: Pin::new(),
                ls_2: Pin::new(),
            },
        }
    }
}
