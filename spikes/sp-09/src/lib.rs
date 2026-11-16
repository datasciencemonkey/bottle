//! SP-09 (#29): a thin JSON-RPC-over-HTTP MCP client for Genie One MCP.
//!
//! The client sends a fresh token on each request (G-04d), keeps the
//! `Mcp-Session-Id` (G-04b), names protocol 2025-06-18 (G-04c) and reads both
//! `application/json` and `text/event-stream` replies (MCP Streamable HTTP).
//! Tokens come only from `databricks auth token --profile <p>` and stay in memory.

pub mod contract;
pub mod endpoint;
pub mod envelope;
pub mod probe;
pub mod shape;
pub mod sse;
pub mod token;
pub mod transport;

/// The only MCP protocol version this client negotiates (G-04c).
pub const PROTOCOL_VERSION: &str = "2025-06-18";
