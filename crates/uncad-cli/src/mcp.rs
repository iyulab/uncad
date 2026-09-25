//! `uncad mcp`: the verb table as a Model Context Protocol server over
//! stdio.
//!
//! Every tool is a verb of [`crate::verbs`], and a tool call is
//! [`Verb::call`](crate::verbs::Verb::call) -- the same path the command
//! line takes -- so the first content block of a result is byte for byte
//! what `uncad <verb>` prints on stdout, and the blocks after it are the
//! warnings it prints on stderr. The server keeps nothing between calls:
//! each call reads its files again.
//!
//! Calls run one at a time on a single thread. Reading a drawing goes
//! through LibreDWG, which is not written for concurrent use.

use crate::verbs::VERBS;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use std::sync::Arc;

const INSTRUCTIONS: &str = "Read-only questions about DWG and DXF drawings. Every tool \
    reads the files it is given, by path on this machine, and answers with a JSON document; \
    the same files and arguments give the same answer. A value the drawing does not establish \
    is reported as such (absent, ambiguous with every candidate, or not searched with a \
    reason), never guessed.";

struct Server;

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("uncad", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = VERBS
            .iter()
            .map(|verb| {
                Tool::new(verb.name, verb.description, Arc::new(verb.input_schema()))
                    .with_annotations(ToolAnnotations::from_raw(
                        None,
                        Some(true),
                        Some(false),
                        Some(true),
                        Some(false),
                    ))
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let verb = VERBS
            .iter()
            .find(|v| v.name == request.name)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("no tool '{}'", request.name), None)
            })?;
        let args = request.arguments.unwrap_or_default();
        // A failure is the tool's answer, not a protocol error: the caller
        // reads the message and can correct the call.
        let result = match verb.call(&args) {
            Ok(answer) => {
                let mut content = vec![ContentBlock::text(answer.json)];
                content.extend(
                    answer
                        .warnings
                        .into_iter()
                        .map(|w| ContentBlock::text(format!("warning: {w}"))),
                );
                CallToolResult::success(content)
            }
            Err(message) => CallToolResult::error(vec![ContentBlock::text(message)]),
        };
        Ok(result.into())
    }
}

/// Serves until the client closes stdin.
pub fn serve() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the MCP server: {e}"))?;
    runtime.block_on(async {
        let service = Server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| format!("MCP initialization failed: {e}"))?;
        service
            .waiting()
            .await
            .map_err(|e| format!("MCP server stopped: {e}"))?;
        Ok(())
    })
}
