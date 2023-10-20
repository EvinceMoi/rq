mod logger;
mod app;
mod capture;



use crate::app::run;

fn main() {
    logger::init_logger();
    let _ = run();
}
