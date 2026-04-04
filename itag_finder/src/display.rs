use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::{DrawTarget, RgbColor},
};
use tildagon::display::TildagonDisplay;

type DisplayDrawError = <TildagonDisplay<'static> as DrawTarget>::Error;

fn clear_display(display: &mut TildagonDisplay<'static>) -> Result<(), DisplayDrawError> {
    display.clear(Rgb565::BLACK)
}

#[embassy_executor::task]
pub async fn display_task(mut display: TildagonDisplay<'static>) {
    // Clear the display on startup
    clear_display(&mut display).expect("Failed to clear display");
}
