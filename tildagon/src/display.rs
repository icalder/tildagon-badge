use crate::resources::{DisplayResources, TopBoardResources};
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use esp_hal::{
    Blocking,
    dma::{DmaRxBuf, DmaTxBuf},
    gpio::{Level, Output},
    spi::{
        Mode,
        master::{Config, Spi, SpiDmaBus},
    },
    time::Rate,
};
use mipidsi::{
    NoResetPin,
    interface::SpiInterface,
    models::GC9A01,
    options::{ColorInversion, ColorOrder, Orientation, Rotation},
};

/// The width of the Tildagon display.
pub const WIDTH: usize = 240;
/// The height of the Tildagon display.
pub const HEIGHT: usize = 240;
/// The height of a single rendering stripe.
pub const STRIPE_HEIGHT: usize = 40;
/// The number of stripes required to cover the display.
pub const NUM_STRIPES: usize = HEIGHT / STRIPE_HEIGHT;

/// Errors returned while bringing up the Tildagon display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayInitError {
    DmaRxBuffer,
    DmaTxBuffer,
    ResourcesUnavailable,
    SpiConfig,
    SpiDevice,
    DisplayInit,
}

/// Type alias for the Tildagon display.
pub type TildagonDisplay<'a> = mipidsi::Display<
    SpiInterface<
        'a,
        ExclusiveDevice<SpiDmaBus<'static, Blocking>, Output<'static>, NoDelay>,
        Output<'static>,
    >,
    GC9A01,
    NoResetPin,
>;

/// Initialize the Tildagon badge's round display.
///
/// # Attribution
/// This implementation is based on [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).
pub fn init<'a>(
    top_board: TopBoardResources<'static>,
    display: DisplayResources<'static>,
    buffer: &'a mut [u8],
) -> Result<TildagonDisplay<'a>, DisplayInitError> {
    // Note: dma_buffers! uses static storage internally in esp-hal to provide 'static lifetimes.
    // This means this function should only be called once to avoid aliasing static mut.
    let (rx_buffer, rx_descriptors, tx_buffer, tx_descriptors) = esp_hal::dma_buffers!(32000);

    let dma_rx_buf =
        DmaRxBuf::new(rx_descriptors, rx_buffer).map_err(|_| DisplayInitError::DmaRxBuffer)?;
    let dma_tx_buf =
        DmaTxBuf::new(tx_descriptors, tx_buffer).map_err(|_| DisplayInitError::DmaTxBuffer)?;

    let spi = Spi::new(
        display.spi,
        Config::default()
            .with_frequency(Rate::from_mhz(80))
            .with_mode(Mode::_0),
    )
    .map_err(|_| DisplayInitError::SpiConfig)?
    .with_sck(top_board.hs_1)
    .with_mosi(top_board.hs_2)
    .with_dma(display.dma)
    .with_buffers(dma_rx_buf, dma_tx_buf);

    let cs = Output::new(top_board.hs_4, Level::High, Default::default());
    let dev = ExclusiveDevice::new_no_delay(spi, cs).map_err(|_| DisplayInitError::SpiDevice)?;

    let dc = Output::new(top_board.hs_3, Level::High, Default::default());
    let di = SpiInterface::new(dev, dc, buffer);

    mipidsi::Builder::new(GC9A01, di)
        .display_size(WIDTH as u16, HEIGHT as u16)
        .color_order(ColorOrder::Bgr)
        .invert_colors(ColorInversion::Inverted)
        .orientation(Orientation::new().rotate(Rotation::Deg180))
        .init(&mut embassy_time::Delay)
        .map_err(|_| DisplayInitError::DisplayInit)
}

/// A buffer for rendering a single horizontal stripe of the display.
///
/// This is used to implement flicker-free rendering on displays that lack a hardware
/// back-buffer by rendering the frame in multiple passes (stripes) into this off-screen
/// buffer before sending each stripe to the display.
pub struct StripeBuffer {
    pub pixels: [Rgb565; WIDTH * STRIPE_HEIGHT],
    pub offset_y: i32,
}

impl StripeBuffer {
    pub const fn new(bg: Rgb565) -> Self {
        Self {
            pixels: [bg; WIDTH * STRIPE_HEIGHT],
            offset_y: 0,
        }
    }
}

impl DrawTarget for StripeBuffer {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            let x = point.x;
            let y = point.y - self.offset_y;
            if x >= 0 && x < WIDTH as i32 && y >= 0 && y < STRIPE_HEIGHT as i32 {
                self.pixels[y as usize * WIDTH + x as usize] = color;
            }
        }
        Ok(())
    }
}

impl OriginDimensions for StripeBuffer {
    fn size(&self) -> Size {
        // We report the full display size so that embedded-graphics doesn't clip
        // drawing operations before they reach our draw_iter, which handles
        // clipping to the current stripe.
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

/// Helper for efficient rendering by only drawing items that intersect the current stripe.
pub fn draw_if_intersects<T, D>(
    target: &mut D,
    item: &T,
    stripe_rect: Rectangle,
) -> Result<(), D::Error>
where
    T: Drawable<Color = Rgb565> + Dimensions,
    D: DrawTarget<Color = Rgb565>,
{
    if item.bounding_box().intersection(&stripe_rect).size != Size::zero() {
        item.draw(target)?;
    }
    Ok(())
}

/// Macro to simplify stripe-based rendering with automatic intersection checks.
///
/// Usage:
/// ```rust
/// draw_stripe!(target, stripe_rect,
///     item1,
///     item2,
/// );
/// ```
#[macro_export]
macro_rules! draw_stripe {
    ($target:expr, $stripe_rect:expr, $($item:expr),* $(,)?) => {
        $(
            $crate::display::draw_if_intersects($target, &$item, $stripe_rect)?;
        )*
    };
}

/// Helper to render a full frame using stripe-based buffering.
pub fn render_with_stripes<'a, F>(
    display: &mut TildagonDisplay<'a>,
    buffer: &mut StripeBuffer,
    bg: Rgb565,
    mut draw: F,
) -> Result<(), <TildagonDisplay<'a> as DrawTarget>::Error>
where
    F: FnMut(&mut StripeBuffer, Rectangle) -> Result<(), core::convert::Infallible>,
{
    for stripe_idx in 0..NUM_STRIPES {
        let offset_y = (stripe_idx * STRIPE_HEIGHT) as i32;
        let stripe_rect = Rectangle::new(
            Point::new(0, offset_y),
            Size::new(WIDTH as u32, STRIPE_HEIGHT as u32),
        );

        buffer.offset_y = offset_y;
        let _ = buffer.clear(bg);
        let _ = draw(buffer, stripe_rect);

        display.fill_contiguous(&stripe_rect, buffer.pixels.iter().copied())?;
    }
    Ok(())
}
