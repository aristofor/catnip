// FILE: catnip_mcp/src/main.rs
use rmcp::ServiceExt;
use rmcp::transport::io::stdio;

mod debug_session;
mod resources;
mod server;
mod tools;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let server = server::CatnipMcpServer::new().map_err(|e| anyhow::anyhow!(e))?;
    let service = server.serve(stdio()).await.inspect_err(|e| {
        eprintln!("serving error: {e}");
    })?;
    service.waiting().await?;
    Ok(())
}
