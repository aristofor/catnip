// FILE: catnip_lsp/src/main.rs
mod diagnostics;
mod server;
mod symbols;

use server::CatnipLsp;
use tower_lsp::{LspService, Server};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(CatnipLsp::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
