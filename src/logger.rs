use log::LevelFilter;
pub use log::{debug, error, info, trace, warn};
use std::env;

fn do_init(level: Option<LevelFilter>) {
    use chrono::Local;
    use env_logger::{fmt::style::{Style, Reset}, Builder};
    use std::io::Write;

    let mut logger = Builder::from_default_env();
    logger.format(|buf, record| {
        let level_style = buf.default_level_style(record.level());
        let dim = Style::new().dimmed();

        writeln!(
            buf,
            "{}{}{}{}{}{} {}{: <5}{}{}{}{} {}",

            dim.render(),
            "[",
            Reset.render(),

            dim.render(),
            Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            Reset.render(),

            level_style.render(),
            record.level(),
            Reset.render(),

            dim.render(),
            "]",
            Reset.render(),

            record.args()
        )
    });

    if let Some(level) = level {
        logger.filter_level(level);
    }

    logger.init()
}

pub fn init_logger() {
    #[cfg(debug_assertions)]
    let level = LevelFilter::Debug;

    #[cfg(not(debug_assertions))]
    let level = LevelFilter::Info;

    match env::var_os("RUST_LOG") {
        Some(_) => do_init(None),
        None => do_init(Some(level)),
    }
}
