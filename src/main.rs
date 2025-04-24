use backend::Backend;
use clap::{arg, command, crate_version};
use log::init_logger;
use tower_lsp::{LspService, Server};

mod backend;
mod completion;
mod goto;
mod hint;
mod log;
mod parse;
mod utils;
mod references;

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

    init_logger();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
