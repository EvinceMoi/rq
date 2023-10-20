mod logger;
mod app;
mod capture;

use log::debug;

use crate::app::run;

fn main() {
    logger::init_logger();
    let _ = run();
}
