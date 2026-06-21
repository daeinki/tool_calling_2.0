//! `ptc-harness` — 러너·채점기·골든 픽스처.
//!
//! 현재 구현 범위: M1-T10(골든 픽스처 + 픽스처 실행 비교),
//! M1-T11(음성 테스트 스위트 + 실패 분류), M1-T12(게이트 러너),
//! M2-T03(코드 추출), M2-T04(RunRecord + JSONL), M2-T05(채점기 + ExactMatch),
//! M2-T06(골든 태스크 + 로더), M2-T07(실패 taxonomy 6분류), M2-T08(E2E 러너),
//! M2-T09(M2 게이트 — Mock 경로), M2-T10(라이브 Anthropic 게이트),
//! M3-T02(TraceMatch L2 채점기), M3-T03(골든 스위트 22 + gen-tasks),
//! M3-T04(디렉터리 로더), M3-T05(배치 러너), M3-T08(M3 게이트),
//! M4-T01(ReAct 프로토콜), M4-T02(baseline ReAct 러너), M4-T06(M4 비교 게이트).

pub mod baseline;
pub mod batch;
pub mod extract;
pub mod fixtures;
pub mod gate;
pub mod grader;
pub mod negative;
pub mod react;
pub mod record;
pub mod runner;
pub mod suite;
pub mod task;
pub mod taxonomy;
