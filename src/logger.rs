use log::LevelFilter;
pub use log::{debug, error, info, trace, warn};
use std::env;

fn do_init(level: Option<LevelFilter>) {
	use chrono::Local;
	use env_logger::{fmt::Color, Builder};
	use log::Level;
	use std::io::Write;

	let mut logger = Builder::from_default_env();
	logger.format(|buf, record| {
		let mut style = buf.style();
		let level_color = match record.level() {
			Level::Error => Color::Red,
			Level::Warn => Color::Yellow,
			Level::Info => Color::Green,
			Level::Debug => Color::Blue,
			Level::Trace => Color::Cyan,
		};
		style.set_color(level_color);

		let mut dim = buf.style();
		dim.set_dimmed(true);

		writeln!(
			buf,
			"{}{} {: <5} {}{} {}",
			dim.value("["),
			dim.value(Local::now().format("%Y-%m-%d %H:%M:%S%.3f")),
			style.value(record.level()),
			dim.value(record.target()),
			dim.value("]"),
			record.args()
		)
	});

	if let Some(level) = level {
		logger.filter_level(level);
	}

	logger.init()
}

pub fn init_logger() {
	match env::var_os("RUST_LOG") {
		Some(_) => do_init(None),
		None => do_init(Some(LevelFilter::Debug)),
	}
}
