//! The MCP protocol for wubble's agent sessions, revision 2026-07-28 (docs/research/mcp-server.md
//! sections 0, 3 and 4). Pure: bytes and values in, values out. The host owns the socket, the
//! session table and every effect; this crate only parses a line into [`Incoming`] and builds the
//! reply line.
//!
//! - [`frame`]: newline-delimited JSON over a byte stream.
//! - [`tools`]: the v0 tool table as data (name, schema, annotations).
//! - [`call`]: a validated `tools/call` as one [`AgentCall`] value.
//! - [`server`]: `server/discover`, `tools/list`, `tools/call` and typed errors.

pub mod call;
pub mod frame;
pub mod server;
pub mod tools;

pub use call::{AgentCall, ButtonKind, ElementRef, NavTarget, SessionId, TabId, WaitCondition};
pub use frame::{FrameError, LineFramer, encode_line};
pub use server::{
    Content, Incoming, RequestId, RpcError, ToolResult, handle_line, tool_response, unavailable,
};
