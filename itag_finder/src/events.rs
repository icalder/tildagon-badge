use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use trouble_host::prelude::Address;

#[derive(Debug, Clone)]
pub enum BleEvent {
    DeviceSeen(Address, i8, Option<heapless::String<32>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonEvent {
    Up,
    Down,
    Select,
    Back,
    PowerOff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemEvent {
    PowerOff,
}

pub static BLE_EVENTS: Channel<CriticalSectionRawMutex, BleEvent, 16> = Channel::new();
pub static BUTTON_EVENTS: Channel<CriticalSectionRawMutex, ButtonEvent, 8> = Channel::new();
pub static SYSTEM_EVENTS: Channel<CriticalSectionRawMutex, SystemEvent, 4> = Channel::new();
pub static DISPLAY_SIGNAL: embassy_sync::signal::Signal<CriticalSectionRawMutex, ()> = embassy_sync::signal::Signal::new();
