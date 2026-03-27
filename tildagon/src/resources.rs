//! Hardware resource groups for the Tildagon badge.
//!
//! The [`split_resources!`] macro (generated below) splits an
//! `esp_hal::peripherals::Peripherals` value into typed sub-groups that can be
//! handed to individual tasks without sharing the entire peripheral struct.
//!
//! # Attribution
//! The `assign_resources!` macro pattern is derived from
//! [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs), which
//! in turn adapted it from the esp-hal project.
//!
//! # Example
//! ```ignore
//! let p = esp_hal::init(esp_hal::Config::default());
//! let r = split_resources!(p);
//! // r.i2c  -> I2cResources  (SDA, SCL, I2C0, reset pin)
//! // r.leds -> LedResources  (GPIO21, RMT)
//! // r.system -> SystemResources (button INT)
//! ```

macro_rules! assign_resources {
    {
        $(#[$struct_meta:meta])*
        $vis:vis $struct_name:ident<$struct_lt:lifetime> {
            $(
                $(#[$group_meta:meta])*
                $group_name:ident : $group_struct:ident<$group_lt:lifetime> {
                    $(
                        $(#[$resource_meta:meta])*
                        $resource_name:ident : $resource_field:ident
                    ),*
                    $(,)?
                }
            ),+
            $(,)?
        }
    } => {
        // ── Per-group structs ─────────────────────────────────────────────────
        $(
            $(#[$group_meta])*
            #[allow(missing_docs)]
            $vis struct $group_struct<$group_lt> {
                $(
                    $(#[$resource_meta])*
                    pub $resource_name: esp_hal::peripherals::$resource_field<$group_lt>,
                )+
            }

            impl<$group_lt> $group_struct<$group_lt> {
                /// Unsafely create an instance of this peripheral group out of thin air.
                ///
                /// # Safety
                /// You must ensure that each contained peripheral is only used once.
                pub unsafe fn steal() -> Self {
                    unsafe {
                        Self {
                            $($resource_name: esp_hal::peripherals::$resource_field::steal()),*
                        }
                    }
                }

                /// Reborrow with a shorter lifetime.
                pub fn reborrow(&mut self) -> $group_struct<'_> {
                    $group_struct {
                        $($resource_name: self.$resource_name.reborrow()),*
                    }
                }
            }
        )+

        // ── Outer struct ──────────────────────────────────────────────────────
        $(#[$struct_meta])*
        /// All Tildagon hardware resources, split into typed groups.
        $vis struct $struct_name<$struct_lt> {
            $( pub $group_name: $group_struct<$struct_lt>, )+
        }

        impl<$struct_lt> $struct_name<$struct_lt> {
            /// Unsafely create all resource groups out of thin air.
            ///
            /// # Safety
            /// You must ensure that each peripheral is only used once.
            pub unsafe fn steal() -> Self {
                unsafe {
                    Self {
                        $($group_name: $group_struct::steal()),*
                    }
                }
            }

            /// Reborrow with a shorter lifetime.
            pub fn reborrow(&mut self) -> $struct_name<'_> {
                $struct_name {
                    $($group_name: self.$group_name.reborrow()),*
                }
            }
        }

        // ── split_resources! macro ────────────────────────────────────────────
        /// Split an `esp_hal::peripherals::Peripherals` into typed resource groups.
        ///
        /// # Example
        /// ```ignore
        /// let p = esp_hal::init(esp_hal::Config::default());
        /// let r = split_resources!(p);
        /// ```
        #[macro_export]
        macro_rules! split_resources {
            ($peris:ident) => {
                $crate::resources::$struct_name {
                    $($group_name: $crate::resources::$group_struct {
                        $($resource_name: $peris.$resource_field),*
                    }),*
                }
            }
        }
    };
}

assign_resources! {
    pub Resources<'d> {
        /// I2C bus peripherals (SDA, SCL, controller, reset pin).
        i2c: I2cResources<'d> {
            sda:   GPIO45,
            scl:   GPIO46,
            i2c:   I2C0,
            reset: GPIO9,
        },
        /// System-level peripherals (shared interrupt line).
        system: SystemResources<'d> {
            int: GPIO10,
        },
        /// LED ring peripherals (WS2812B data pin + RMT controller).
        leds: LedResources<'d> {
            data: GPIO21,
            rmt:  RMT,
        },
        /// Top-board high-speed GPIO lines.
        top_board: TopBoardResources<'d> {
            hs_1: GPIO8,
            hs_2: GPIO7,
            hs_3: GPIO2,
            hs_4: GPIO1,
        },
        /// Display SPI + DMA resources.
        display: DisplayResources<'d> {
            spi: SPI2,
            dma: DMA_CH0,
        },
        /// Hexpansion slot A high-speed GPIO lines.
        hexpansion_a: HexpansionAResources<'d> {
            hs_1: GPIO39,
            hs_2: GPIO40,
            hs_3: GPIO41,
            hs_4: GPIO42,
        },
        /// Hexpansion slot B high-speed GPIO lines.
        hexpansion_b: HexpansionBResources<'d> {
            hs_1: GPIO35,
            hs_2: GPIO36,
            hs_3: GPIO37,
            hs_4: GPIO38,
        },
        /// Hexpansion slot C high-speed GPIO lines.
        hexpansion_c: HexpansionCResources<'d> {
            hs_1: GPIO34,
            hs_2: GPIO33,
            hs_3: GPIO47,
            hs_4: GPIO48,
        },
        /// Hexpansion slot D high-speed GPIO lines.
        hexpansion_d: HexpansionDResources<'d> {
            hs_1: GPIO11,
            hs_2: GPIO14,
            hs_3: GPIO13,
            hs_4: GPIO12,
        },
        /// Hexpansion slot E high-speed GPIO lines.
        hexpansion_e: HexpansionEResources<'d> {
            hs_1: GPIO18,
            hs_2: GPIO16,
            hs_3: GPIO15,
            hs_4: GPIO17,
        },
        /// Hexpansion slot F high-speed GPIO lines.
        hexpansion_f: HexpansionFResources<'d> {
            hs_1: GPIO3,
            hs_2: GPIO4,
            hs_3: GPIO5,
            hs_4: GPIO6,
        },
        /// Radio resources (WiFi, Bluetooth, clocks, RNG).
        radio: RadioResources<'d> {
            wifi: WIFI,
            bt:   BT,
            rng:  RNG,
            timer: TIMG1,
        },
    }
}
