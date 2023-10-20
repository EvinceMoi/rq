mod logger;
mod app;

use log::debug;

use crate::app::run;

fn main() {
    logger::init_logger();
    let _ = run();
}
