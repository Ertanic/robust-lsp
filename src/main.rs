use std::io;

use backend::Backend;
use tower_lsp::{LspService, Server};
use tracing_subscriber::{filter, layer::SubscriberExt, util::SubscriberInitExt};

mod backend;
mod parse;
mod utils;
mod completion;
mod goto;

#[tokio::main]
async fn main() {
    let fmt_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(false)
        .without_time()
        .with_line_number(true)
        .with_file(true)
        .with_writer(io::stderr)
        .with_thread_ids(true);

    tracing_subscriber::registry()
        .with(filter::Targets::new().with_target("robust_lsp", filter::LevelFilter::TRACE))
        .with(fmt_layer)
        .init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend::new(client));
    Server::new(stdin, stdout, socket).serve(service).await;
}
