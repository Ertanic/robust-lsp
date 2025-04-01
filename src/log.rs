use std::io;
use tracing_subscriber::{filter, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_logger() {
    let fmt_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(false)
        .without_time()
        .with_line_number(true)
        .with_file(true)
        .with_writer(io::stderr)
        .with_thread_ids(true);

    let targets = filter::Targets::new().with_target("robust_lsp", filter::LevelFilter::TRACE);
    #[cfg(debug_assertions)] let targets = targets.with_target("tower_lsp", filter::LevelFilter::TRACE);

    tracing_subscriber::registry()
        .with(targets)
        .with(fmt_layer)
        .init();
}