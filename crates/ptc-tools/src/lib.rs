//! `ptc-tools` — 하네스 검증용 결정론적 mock 도구.
//!
//! 현재 구현 범위: M1-T09([`MockToolServer`]와 도구 4종),
//! M3-T01(schedule 도메인 + `domain.tool` 라우팅 + [`tool_names`]).

pub mod mock;

pub use mock::{base_tool, tool_names, MockToolServer, ToolCall};
