mod capture;
mod logger;
mod selection;

use anyhow::{anyhow, Result};
use image::{RgbaImage, DynamicImage};

use crate::selection::wait_for_selection;

fn main() -> Result<()> {
    logger::init_logger();

    // select area from screen
    let area = wait_for_selection()?;

    // capture area
    let captured = futures::executor::block_on(async {
        capture::area(area.x(), area.y(), area.width(), area.height()).await
    })?;

    // read image
    let image = RgbaImage::from_vec(captured.width, captured.height, captured.buf)
        .ok_or(anyhow!("failed to read image"))?;
    let luma = DynamicImage::from(image).to_luma8();

    let mut img = rqrr::PreparedImage::prepare(luma);
    for grid in img.detect_grids() {
        let (_meta, content) = grid.decode()?;
        println!("{content}");
    }

    Ok(())
}
