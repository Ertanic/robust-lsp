use backend::Backend;
use clap::{arg, command, crate_version};
use std::io;
use tower_lsp::{LspService, Server};
use tracing_subscriber::{filter, layer::SubscriberExt, util::SubscriberInitExt};

mod backend;
mod completion;
mod goto;
mod hint;
mod parse;
mod utils;

#[tokio::main]
async fn main() {
    let matches = command!()
        .disable_version_flag(true)
        .arg(arg!(-v --version "Print version information"))
        .get_matches();

    if matches.get_one::<bool>("version") == Some(&true) {
        print!(crate_version!());
        return;
    }

    let fmt_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(false)
        .without_time()
        .with_line_number(true)
        .with_file(true)
        .with_writer(io::stderr)
        .with_thread_ids(true);

    let targets = filter::Targets::new().with_target("robust_lsp", filter::LevelFilter::TRACE);
    #[cfg(debug_assertions)]
    let targets = targets.with_target("tower_lsp", filter::LevelFilter::TRACE);

    tracing_subscriber::registry()
        .with(targets)
        .with(fmt_layer)
        .init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend::new(client));
    Server::new(stdin, stdout, socket).serve(service).await;
}
