use tower_lsp::{
    jsonrpc::*,
    lsp_types::{InitializeParams, InitializeResult, InitializedParams, MessageType},
    Client, LanguageServer, LspService, Server,
};

struct Backend {
    client: Client,
}

impl Backend {
    fn check_project_compliance(params: &InitializeParams) -> bool {
        if let Some(root_uri) = params.root_uri.as_ref() {
            let root_path = root_uri.to_file_path().unwrap();

            return root_path.join("SpaceStation14.sln").exists() || root_path.join("RobustToolbox/RobustToolbox.sln").exists();
        }

        false
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if !self::Backend::check_project_compliance(&params) {
            return Err(Error::new(ErrorCode::ServerError(32900)));
        }

        Ok(InitializeResult::default())
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend { client });
    Server::new(stdin, stdout, socket).serve(service).await;
}
