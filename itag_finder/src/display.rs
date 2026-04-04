use crate::events::{DISPLAY_SIGNAL, SYSTEM_EVENTS, SystemEvent};
use crate::itag::APP_STATE;
use embedded_graphics::{
    mono_font::{
        MonoTextStyle,
        ascii::{FONT_9X15, FONT_10X20},
    },
    pixelcolor::Rgb565,
    prelude::*,
    text::{Alignment, Text},
};
use tildagon::display::TildagonDisplay;

fn rssi_color(rssi: i8) -> Rgb565 {
    if rssi >= -55 {
        Rgb565::RED
    } else if rssi >= -65 {
        Rgb565::YELLOW
    } else if rssi >= -75 {
        Rgb565::GREEN
    } else if rssi >= -85 {
        Rgb565::CYAN
    } else if rssi >= -95 {
        Rgb565::BLUE
    } else {
        Rgb565::MAGENTA
    }
}

#[embassy_executor::task]
pub async fn display_task(mut display: TildagonDisplay<'static>) {
    let _ = display.clear(Rgb565::BLACK);

    // Initial draw
    draw_radar(&mut display).await;

    loop {
        use embassy_futures::select::{Either, select};

        match select(DISPLAY_SIGNAL.wait(), SYSTEM_EVENTS.receive()).await {
            Either::First(_) => {
                draw_radar(&mut display).await;
            }
            Either::Second(SystemEvent::PowerOff) => {
                let _ = display.clear(Rgb565::BLACK);
                let style = MonoTextStyle::new(&FONT_10X20, Rgb565::RED);
                let _ = Text::with_alignment(
                    "Shutting down...",
                    Point::new(120, 120),
                    style,
                    Alignment::Center,
                )
                .draw(&mut display);
                // Give it a moment to show the message
                embassy_time::Timer::after(embassy_time::Duration::from_millis(500)).await;
                break;
            }
        }
    }
}

async fn draw_radar(display: &mut TildagonDisplay<'static>) {
    let _ = display.clear(Rgb565::BLACK);

    let state = APP_STATE.lock().await;

    let header_style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let _ = Text::with_alignment(
        "iTag Radar",
        Point::new(120, 35),
        header_style,
        Alignment::Center,
    )
    .draw(display);

    // Starting Y coordinate for the list
    let list_start_y = 65;

    for (i, device) in state.devices.iter().enumerate() {
        let y = list_start_y + (i as i32 * 20);
        if y > 215 {
            break;
        } // Don't draw off the bottom of the circle

        let is_selected = i == state.selected_index;

        let style = MonoTextStyle::new(
            &FONT_9X15,
            if is_selected {
                Rgb565::CYAN
            } else {
                Rgb565::WHITE
            },
        );

        let name = device
            .name
            .as_ref()
            .map(|n| n.as_str())
            .unwrap_or("Unknown");
        let mut buf: heapless::String<64> = heapless::String::new();
        let _ = core::fmt::write(
            &mut buf,
            format_args!("{}{} ", if is_selected { "> " } else { "  " }, name),
        );

        // Circular safe X varies by Y, but 30 is generally safe for the middle 2/3 of the screen
        let x_offset = 30;
        let _ = Text::new(buf.as_str(), Point::new(x_offset, y), style).draw(display);

        // Draw RSSI indicator in its color after the name
        let rssi_style = MonoTextStyle::new(&FONT_9X15, rssi_color(device.rssi));
        let mut rssi_buf: heapless::String<16> = heapless::String::new();
        let _ = core::fmt::write(&mut rssi_buf, format_args!("RSSI:{}", device.rssi));

        let name_width = (buf.len() as i32) * 9;
        let _ = Text::new(
            rssi_buf.as_str(),
            Point::new(x_offset + name_width, y),
            rssi_style,
        )
        .draw(display);
    }
}
