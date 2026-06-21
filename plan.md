# 구현 계획 — oneai-ptc-harness

> `harness.md`(엔지니어링 설계)를 실제 구현 작업 단위로 분해하고, `clean-code.md`의 원칙을 각 단계에 적용한 실행 계획서.
> 설계 문서가 **무엇을/왜**를 정의한다면, 이 문서는 **어떤 순서로 어떻게 짓는가**를 정의한다.

| 항목 | 내용 |
|------|------|
| 대상 | Programmatic Tool Calling(PTC)을 Rust로 구현하고 LLM으로 검증하는 하네스 |
| 저장소 | `oneai-ptc-harness` (신규 Cargo 워크스페이스) |
| 진행 방식 | Milestone-Gated (M0→M4), 게이트 미통과 시 다음 단계 진입 금지 |
| 코딩 규범 | `clean-code.md` 전 항목을 PR 머지 기준에 포함 |

---

## 0. 계획의 대원칙

설계 문서의 핵심 통찰은 **"측정 도구가 먼저 신뢰 가능해야 측정 결과가 의미 있다"**는 것이다.
따라서 구현 순서는 기능 중요도 순이 아니라 **신뢰 기준선을 쌓는 순서**를 따른다:
인프라(M0) → 인터프리터 정확성(M1) → 단일 E2E(M2) → 규모(M3) → 비교(M4).

각 마일스톤은 다음 3요소를 반드시 갖춘다.

1. **산출물(Deliverable)** — 구현할 크레이트/모듈/파일
2. **게이트(Gate)** — 객관적으로 판정 가능한 통과 조건
3. **클린코드 체크** — 해당 단계에서 특히 중요한 `clean-code.md` 항목

> 게이트는 "사람이 보기에 괜찮다"가 아니라 **자동 판정 가능한 조건**이어야 한다.
> 통과 전까지 다음 마일스톤의 코드를 작성하지 않는다.

---

## 1. 공통 코딩 규범 (전 마일스톤 적용)

`clean-code.md`를 Rust 맥락으로 구체화한 머지 기준이다. 모든 PR이 이 체크리스트를 통과해야 한다.

### 1.1 이름과 함수
- **의도를 드러내는 이름.** `d` 대신 `elapsed_days`, 매직 넘버 대신 `const MAX_NESTING_DEPTH`.
- **개념 하나에 단어 하나.** 코드 텍스트 추출은 `extract_*`, 도구 호출은 `call_*`로 통일(`get/fetch/retrieve` 혼용 금지).
- **함수는 한 가지만.** 한 함수가 한 추상화 수준만 다룬다. 20줄을 넘으면 분리를 검토한다.
- **플래그 인수 금지.** `render(true)` 같은 boolean 분기는 `render_for_suite()` / `render_for_single()`로 나눈다. PTC/baseline 모드 분기도 enum + 별도 함수로.
- **명령/조회 분리.** 상태를 바꾸는 함수와 값을 반환하는 함수를 섞지 않는다.

### 1.2 오류 처리 (Rust 관용구로 번역)
- 오류 코드 대신 `Result<T, E>`와 thiserror 기반 도메인 에러 enum을 사용한다.
- **null/None을 인수로 넘기지 않는다.** 빈 컬렉션이나 특수 케이스 값을 반환한다.
- 에러에 **맥락**을 담는다 — span(줄·열), 도구 이름, 입력값 등을 에러에 포함해 실패 분류(taxonomy)가 가능하게 한다.

### 1.3 경계 (Boundaries)
- 외부 LLM API·HTTP 클라이언트는 `LlmProvider` trait 뒤로 **래핑**한다. 외부 타입이 도메인 코드에 새어 나오지 않게 한다.
- 외부 라이브러리는 **학습 테스트**로 사용법을 고정하고, 버전 업 시 호환성을 검증한다.

### 1.4 테스트 (F.I.R.S.T)
- 인터프리터·파서·채점기는 **TDD**로 작성한다(실패 테스트 → 최소 구현).
- 테스트는 Fast·Independent·Repeatable·Self-Validating·Timely.
- **테스트당 개념 하나.** 한 케이스가 한 동작만 검증한다.

### 1.5 클래스/모듈 (SRP)
- 크레이트 경계 = 책임 경계. `ptc-dsl`은 언어만, `ptc-llm`은 provider만, `ptc-harness`는 오케스트레이션만 안다.
- 생성과 사용을 분리한다(DI). 의존성 조립은 `main`/러너 진입점에서만 한다.

---

## 2. 저장소 부트스트랩 (M0 진입 전)

```
oneai-ptc-harness/
├── Cargo.toml                 # [workspace] members = crates/*
├── rust-toolchain.toml        # 버전 고정 (재현성)
├── .github/workflows/ci.yml   # fmt + clippy + test + mock 파이프라인
├── crates/
│   ├── ptc-dsl/               # 언어: lexer·parser·ast·validator·interp
│   ├── ptc-llm/               # LlmProvider trait + 3 구현체
│   ├── ptc-tools/             # MockToolServer + 도구 카탈로그
│   ├── ptc-harness/           # runner·grader·record (오케스트레이션)
│   └── ptc-analyze/           # McNemar·bootstrap (Rust 또는 Python)
├── tasks/                     # 골든 태스크 (TOML, 선언적)
├── prompts/                   # 시스템 프롬프트 (버전 관리)
└── results/                   # RunRecord JSONL 출력
```

**부트스트랩 작업:**
- [ ] Cargo 워크스페이스 + 5개 빈 크레이트 생성
- [ ] `rustfmt.toml`, `clippy.toml` 합의 후 고정 (형식 = 의사소통, 팀 단일 규칙)
- [ ] CI에 `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` 게이트 추가

---

## 3. 마일스톤별 구현 계획

### M0 — 스캐폴딩 (측정 인프라)

**목표.** 구현 대상(DSL)은 아직 없다. 측정 골격이 먼저 동작함을 보장한다.

**산출물 / 작업:**
- [ ] `ptc-llm`: `LlmProvider` trait 정의 (`complete(CompletionReq) -> CompletionResp`)
  - `CompletionReq`: system / user / temperature / seed / max_tokens
  - `CompletionResp`: text / input_tokens / output_tokens / stop_reason / latency_ms
  - **토큰 회계를 응답에 반드시 포함** (성능 비교의 1차 지표)
- [ ] 3 구현체: `anthropic.rs`, `ollama.rs`, `openai.rs` — 외부 HTTP는 trait 뒤로 래핑
- [ ] `MockProvider` 추가 — CI에서 네트워크 없이 전체 파이프라인 검증용
- [ ] `ptc-harness/record.rs`: `RunRecord` 스키마 + JSONL 라이터, 시드·프롬프트 버전 기록
- [ ] 재현성 기록: 모델 ID·API 버전·시드·temperature를 RunRecord에 고정

**게이트:**
- 3개 provider 모두 "2+2=?" 프롬프트에 응답하고 토큰 수·지연시간이 RunRecord에 적재됨
- CI에서 `MockProvider`로 전체 파이프라인이 그린

**이 단계의 클린코드 초점:**
- **경계 관리**: HTTP/JSON 직렬화가 `LlmProvider` 뒤에 갇혀 도메인으로 새지 않는가
- **학습 테스트**: 각 provider SDK 사용법을 테스트로 고정

---

### M1 — DSL 코어 ⭐ (가장 중요) — ✅ 완료 (게이트 통과)

**목표.** LLM 없이, 손으로 쓴 DSL 스크립트가 파서→검증기→인터프리터를 거쳐 mock 도구를 정확히 호출함을 보장한다. **이 게이트가 "인터프리터는 옳다"는 신뢰 기준선을 만든다.**

**산출물 / 작업 (TDD 권장 순서):**
- [x] `ast.rs`: `Stmt`/`Expr`/`Arg` enum 정의 (`Box`로 재귀, `Span` 부착)
- [x] `lexer.rs`: 토큰화 (들여쓰기 기반 INDENT/DEDENT 처리)
- [x] `parser.rs`: EBNF v0 문법 → AST (assign/for/if/emit/expr만)
- [x] `validator.rs`: 실행 전 정적 검증
  - 미등록 도구 호출 거부
  - 중첩 깊이 초과 거부 (`MAX_NESTING_DEPTH` 상수)
  - 허용되지 않은 노드 종류 거부
- [x] `interp.rs`: tree-walking 인터프리터
  - `Value` enum (Num/Str/Bool/Null/List/Map) — JSON과 1:1
  - `ToolSink` trait — 인터프리터는 MCP/HTTP를 **전혀 모른다**
  - `eval_args` — 4가지 케이스(리터럴/변수/필드접근/중첩호출) 처리
- [x] `ptc-tools/mock.rs`: `MockToolServer` + 최소 도구 4종
  - `list_team`, `get_expenses`, `get_budget`, `send_email`
  - 모든 호출을 `trace`에 순서대로 적재 (채점 근거)
  - 결정론적: 같은 인자 → 항상 같은 결과
- [x] 수기 DSL 스크립트 10개 + 각 기대 트레이스/출력

**게이트 (전부 충족):**
- [x] 수기 스크립트 10개가 100% 기대 트레이스·출력과 일치 (10/10)
- [x] 검증기가 미등록 도구·깊이 초과·금지 노드를 모두 거부 (**음성 10/10**)
- [x] 인터프리터 단위테스트 커버리지 ≥ 80% (**86.27% line**)

> ✅ **M1 완료 (`cargo run -p ptc-harness --bin m1-gate` 통과).** 테스트 101개 전부 green, `clippy -D warnings` 무경고.
> 봉합 정리: T05에서 T04의 dead-code 허용 제거, T08에서 임시 `Unsupported`·`sink` 허용 제거(`ToolFailed`로 대체) 및 `called_name`→`ast::callee_name` DRY 승격.

**이 단계의 클린코드 초점:**
- **SRP**: lexer/parser/validator/interp가 각각 한 책임만. 파서가 검증을 하지 않는다.
- **추상화 수준 분리**: `eval_expr`는 표현식 평가만, 도구 호출 위임은 `ToolSink`로.
- **오류에 맥락**: 모든 검증/런타임 에러에 span 포함 → 줄·열이 찍힌 메시지
- **DRY**: 4가지 인자 케이스를 중복 없이 재귀 평가 하나로 수렴

> ⚠️ M1을 대충 넘기면 이후 모든 측정의 신뢰가 무너진다. 음성 테스트(거부되어야 할 입력)를 반드시 포함한다.

#### M1 작업 티켓

의존성 흐름: **공통타입 → AST → Lexer → Parser → Validator → Interpreter → Mock → 골든 픽스처 → 게이트**

| 티켓 | 제목 | 의존 | 상태 | 완료 기준 (요약) |
|------|------|------|------|------------------|
| M1-T01 | 공통 타입 (Span, 에러 골격) | — | ✅ | 에러가 `Display`로 줄·열 출력, 포맷 단위 테스트 |
| M1-T02 | AST 정의 (`ast.rs`) | T01 | ✅ | Stmt/Expr/Arg/BinOp enum, `Debug`/`Clone` 라운드트립 |
| M1-T03 | Lexer — 토큰화 + INDENT/DEDENT | T01 | ✅ | 정상 토큰화 + 들여쓰기 불일치 음성 테스트 |
| M1-T04 | Parser — 표현식 | T02, T03 | ✅ | 연산자 우선순위·중첩 호출 파싱 |
| M1-T05 | Parser — 문장·블록 | T04 | ✅ | EBNF v0 전체, 중첩 for/else 없는 if |
| M1-T06 | Validator — 정적 검증 | T02 | ✅ | 미등록 도구·깊이 초과·금지 노드 거부 (음성) |
| M1-T07 | 값 모델 + ToolSink + 제어흐름 | T02 | ✅ | for/if/emit 실행, emit 최종값 반환 |
| M1-T08 | 표현식 평가 + eval_args 4케이스 | T07 | ✅ | 4케이스, 중첩 호출 순서 정확 |
| M1-T09 | MockToolServer + 도구 4종 | T07, T08 | ✅ | 결정론적 결과 + trace 적재 |
| M1-T10 | 골든 픽스처 — 수기 스크립트 10개 | T05, T08, T09 | ✅ | 기대 트레이스·출력 선언적 픽스처 |
| M1-T11 | 음성 테스트 스위트 | T06, T08 | ✅ | 거부 케이스 전부 정확한 에러 분류 |
| M1-T12 | 게이트 러너 + 커버리지 | T10, T11 | ✅ | **M1 게이트**: 10개 일치·음성 거부·커버리지 ≥80% |

**임계 경로**: T01 → T02 → T03 → T04 → T05 → T08 → T10 → T12
**병렬 가능**: T06(검증기)은 T02 이후 독립 진행, T09(mock)는 T07 직후 진행 가능

각 티켓의 클린코드 초점:
- T01: 오류에 맥락 담기 — taxonomy 분류의 기반
- T02: 자료구조는 데이터만 노출(동작 없음)
- T03/T05: SRP·함수는 한 가지만 (`parse_for`/`parse_if`/`parse_block` 분리)
- T04/T08: 추상화 수준 분리, DRY (4케이스 단일 재귀로 수렴)
- T06: DI (도구 카탈로그 주입), 매직넘버 대신 상수
- T07: 경계 — 인터프리터는 `ToolSink` trait 뒤만 본다
- T09: 명령/조회 — `send_email`도 부수효과 없이 기록만
- T11: 경계 조건·모든 분기 테스트
- T12: 자동 판정 — boolean으로 게이트 통과 결정

---

### M2 — 단일 태스크 E2E — ✅ 완료 (Mock 게이트 통과 · 라이브 구현 완료)

**목표.** LLM이 실제로 우리 DSL을 생성하고, 그 코드가 1개 골든 태스크에서 정답에 도달하는 전체 경로를 처음으로 연결한다.

**산출물 / 작업:**
- [x] `prompts/sys-v1.md`: 시스템 프롬프트 v1 (DSL 문법 + 도구 스텁 + 예시 1개) — 버전 관리, `include_str!`로 임베드
- [x] 코드 추출 (`ptc-harness/extract.rs`, 3.2절 규칙)
  1. 코드 펜스 있으면 첫 블록만
  2. 없으면 전체를 코드로 간주, 파서 실패 시 `extraction_fail` 기록
  3. 언어 태그 무시 (우리 파서가 진짜 검증)
- [x] `ptc-harness/grader.rs`: `Grader` trait + `ExactMatch`(L1) 구현
- [x] `tasks/easy_first_member.toml`: 골든 태스크 1개 (tier=easy, L1 채점) + TOML 로더
- [x] 실패 분류(taxonomy) 기록: EXTRACTION_FAIL / PARSE_ERROR / VALIDATION_REJECT / RUNTIME_ERROR / WRONG_ANSWER / HARNESS_BUG (`taxonomy.rs`)
- [x] `ptc-harness/runner.rs`: E2E 러너 + `RunRecord`/JSONL(`record.rs`)
- [x] `ptc-llm`: `LlmProvider` trait + `MockProvider` + `AnthropicProvider`(라이브)

**게이트:**
- [x] Mock 경로: 골든 태스크 R=5 통과율 1.00 (`m2-gate` 바이너리·테스트)
- [x] 실패 사례가 taxonomy로 분류되어 기록됨 (러너 6분류 매핑)
- [x] **HARNESS_BUG 0건**
- [~] 라이브 provider(Anthropic) R=5 통과율 ≥0.8 — 구현 완료, 실행은 `ANTHROPIC_API_KEY` 필요 (`m2-live` 바이너리, `#[ignore]` 라이브 테스트)

> ✅ **M2 Mock 게이트 통과** (`cargo run -p ptc-harness --bin m2-gate`). 테스트 153개 통과 + 라이브 1개 `#[ignore]`.
> 설계 판단: `LlmProvider`는 동기 trait(동시성 불필요, async-trait 회피), 태스크는 TOML, 직렬화는 harness 경계에서만(코어 `ptc-dsl`은 serde-free 유지).

**이 단계의 클린코드 초점:**
- **명령/조회 분리**: 추출(조회) / 채점(조회) / 로그 기록(명령)을 섞지 않는다
- **플래그 인수 금지**: 채점 레벨은 trait 다형성으로, boolean 분기로 만들지 않는다

#### M2 작업 티켓

의존성 흐름: **LlmProvider → MockProvider → (코드추출 · RunRecord · Grader · Task) → 러너 → 게이트**

> 검증 환경에서 실제 LLM은 비결정적이고 네트워크·키가 필요하므로, **대부분의 티켓은 `MockProvider`로 결정론적으로 테스트**한다. 실제 provider(Anthropic 등) 통과율 측정은 라이브 단계(T10)로 분리한다.

| 티켓 | 제목 | 의존 | 상태 | 완료 기준 (요약) |
|------|------|------|------|------------------|
| M2-T01 | `ptc-llm` 크레이트 — `LlmProvider` trait | — | ✅ | `CompletionReq`/`Resp`(토큰 회계 포함) 정의, 컴파일 |
| M2-T02 | `MockProvider` — 결정론적 응답 | T01 | ✅ | 프롬프트→고정 코드 텍스트, 토큰·지연 채움 |
| M2-T03 | 코드 추출 (`extract_code`) | — | ✅ | 펜스/언어태그 규칙, 펜스 없음·실패 케이스 분기 |
| M2-T04 | `RunRecord` 스키마 + JSONL 라이터 | — | ✅ | 재현 정보(시드·프롬프트버전·모델ID) 직렬화 |
| M2-T05 | `Grader` trait + `ExactMatch`(L1) | — | ✅ | emit 최종값 == 기대값 판정 (trait 다형성) |
| M2-T06 | 골든 태스크 1개 + 로더 | — | ✅ | tier=easy 태스크 선언 + 기대 출력/호출 |
| M2-T07 | 실패 taxonomy 6분류로 확장 | T11(M1) | ✅ | EXTRACTION_FAIL/WRONG_ANSWER/HARNESS_BUG 추가 |
| M2-T08 | E2E 러너 — LLM→추출→검증→실행→채점 | T01–T07 | ✅ | 1태스크 R회 반복, RunRecord 적재, 실패 분류 |
| M2-T09 | M2 게이트 (Mock 경로) | T08 | ✅ | MockProvider로 통과율 1.0, HARNESS_BUG 0건 |
| M2-T10 | 라이브 provider 실행 (Anthropic) | T08 | ☑ 구현 | 실제 LLM R=5 통과율 ≥0.8 (CI 시크릿/수동) |

**임계 경로**: T01 → T02 → T08 → T09
**병렬 가능**: T03(추출)·T04(record)·T05(grader)·T06(task)는 서로 독립

각 티켓의 클린코드 초점:
- T01: 경계 — HTTP/JSON 직렬화를 `LlmProvider` 뒤에 가둔다
- T02: 학습 테스트 — 결정론적 더블로 러너를 네트워크 없이 검증
- T03: 명령/조회 분리 — 추출(조회)은 부수효과 없이
- T05: 플래그 인수 금지 — 채점 레벨은 trait 다형성으로
- T06: 선언적 데이터 — 태스크는 코드가 아닌 데이터(TOML 또는 Rust 데이터)
- T07: 오류에 맥락 — M1의 `FailureCategory`를 6분류로 확장(중복 없이)
- T08: SRP — 러너는 오케스트레이션만, 각 단계는 기존 컴포넌트에 위임
- T09: 자동 판정 + **HARNESS_BUG 게이트** — 측정 도구 고장 시 통과 불인정

---

### M3 — 태스크 스위트 — ✅ 완료 (Mock 게이트 통과 · 라이브 구현 완료)

**목표.** 규모를 키워 정확성을 통계적으로 의미 있게 측정하고, 3개 provider를 비교한다.

**산출물 / 작업:**
- [x] 골든 태스크 22개 — tier(easy 7/medium 8/hard 7) · domain(hr/finance/schedule) 계층화
  - 각 태스크는 **결정론적 정답**(L1/L2 채점 가능), **다중 도구 호출 유발**(중첩 루프 포함)
- [x] `grader.rs`에 `TraceMatch`(L2) 추가 — 도구별 호출 횟수(count/count_min) 검증
- [~] `Semantic`(L3) — **불필요로 판단해 미구현** (모든 태스크가 L1/L2로 결정론적 채점 가능)
- [x] 배치 러너(`batch.rs`): providers × 전 태스크 × R회 실행
- [x] `ptc-analyze`: 통과율 표(provider×tier) + 실패 6분류 분포 자동 생성
- [~] DSL 점진 확장: 트리거(PARSE_ERROR <10%)는 게이트에 구현, Mock 경로는 0%라 확장 불요

**게이트:**
- [x] 동일 커밋·시드로 두 번 실행 시 통과율 표가 재현됨 (`m3-gate`가 두 배치 표 동일성 판정)
- [x] 문법 부족으로 인한 PARSE_ERROR가 전체의 10% 미만 (Mock 경로 0%)
- [x] (추가) HARNESS_BUG 0건 + 레퍼런스 해법 전체 통과율 1.00

> ✅ **M3 Mock 게이트 통과** (`cargo run -p ptc-harness --bin m3-gate`). 22 태스크 × 3 provider × R5 = 330 실행 전부 통과, 표 재현, PARSE_ERROR 0%. 테스트 197개 + 라이브 3개 `#[ignore]`.
> 설계 판단: **기대값은 손계산하지 않고 레퍼런스 해법(`suite.rs`)을 mock에서 실행해 도출**(`gen-tasks` 바이너리가 `tasks/*.toml` 생성) → 산수 실수로 정답이 어긋날 여지를 제거. 순환 의존을 피해 `ptc-analyze`는 JSONL을 외부 인터페이스로 보고 자체 뷰로 역직렬화(harness 비의존).

**이 단계의 클린코드 초점:**
- **선언적 태스크**: 태스크는 코드가 아니라 TOML 데이터. 로직과 데이터 분리
- **OCP**: 새 도메인/도구 추가가 기존 인터프리터 수정 없이 확장으로 이뤄지는가

#### M3 작업 티켓

의존성 흐름: **도구·도메인 확장 → (TraceMatch · 태스크 스위트) → 배치 러너 → 분석 → 게이트**

> 자동 게이트는 결정론적 `MockProvider` 3종(이름만 다른)으로 "통과율 표 재현"을 판정한다.
> 실제 3-provider(Anthropic·OpenAI·Ollama) 비교는 라이브 단계로 분리한다(M2-T10 방식).

| 티켓 | 제목 | 의존 | 상태 | 완료 기준 (요약) |
|------|------|------|------|------------------|
| M3-T01 | Mock 도구·도메인 확장 | — | ✅ | schedule 도메인 추가, `domain.tool` 라우팅(`base_tool`), 결정론 유지 |
| M3-T02 | `TraceMatch`(L2) 채점기 | — | ✅ | `expected_tool_calls`의 count/count_min 검증, trait 다형성 |
| M3-T03 | 골든 태스크 스위트 20+ | T01 | ✅ | 22개 tier×domain TOML, 기대값은 레퍼런스 해법 실행으로 도출(gen-tasks) |
| M3-T04 | 태스크 디렉터리 로더 | T03 | ✅ | `load_tasks_dir` — 정렬 로드, id 유일성·채점레벨 정합 검증 |
| M3-T05 | 배치 러너 | T02,T04 | ✅ | providers × tasks × R 실행, RunRecord 모음 (M2 러너 위임) |
| M3-T06 | `ptc-analyze` — 표·분포 | T05 | ✅ | JSONL→통과율 표(provider×tier) + 실패 6분류 분포 |
| M3-T07 | 추가 provider (OpenAI·Ollama) | M2-T01 | ✅ | `LlmProvider` 구현, 라이브 분리(`#[ignore]` + `m3-live` 바이너리) |
| M3-T08 | M3 게이트 + DSL 확장 트리거 | T05,T06 | ✅ | Mock 3종 통과율 표 재현 + PARSE_ERROR <10% |

**임계 경로**: T01 → T03 → T04 → T05 → T06 → T08
**병렬 가능**: T02(TraceMatch)·T07(provider)은 독립 진행

각 티켓의 클린코드 초점:
- T01: OCP — 새 도구가 인터프리터 수정 없이 카탈로그·mock 확장만으로 추가되는가
- T02: 플래그 인수 금지 — L2도 trait 다형성, `Execution.trace` 재사용(OCP 확인)
- T03: 선언적 데이터 — 태스크는 TOML, 기대값은 사전 계산
- T04: 명령/조회 분리 — 디렉터리 스캔(IO)과 파싱 분리
- T05: SRP — 배치 러너는 순회·집계만, 실행은 M2 러너에 위임(DRY)
- T06: 측정의 정직성 — 점추정만 내지 않고 분포·재현 정보 함께 (5.3절)
- T08: 자동 판정 + 재현성 — 동일 시드 두 실행이 같은 표; PARSE_ERROR 임계는 문법 확장의 트리거

---

### M4 — 1.0 vs 2.0 비교 (최종 목표) — ✅ 완료 (Mock 게이트 통과)

**목표.** PTC의 정확성·성능 이점을 baseline 1.0(ReAct) 대비 통계적으로 입증한다.

**산출물 / 작업:**
- [x] Baseline 1.0(ReAct) 모드 구현(`baseline.rs`) — 도구 결과마다 LLM 재호출(호출=도구수+1)
  - **공정성**: 두 모드가 **동일한 MockToolServer·Grader** 사용, 차이는 LLM 호출 방식뿐(DRY)
  - ReAct 액션 프로토콜(`react.rs`): `CALL <tool> <json>` / `FINAL <json>`, 버전 관리 프롬프트
- [x] `ptc-analyze::stats`: McNemar 검정 (불일치<25면 정확 이항검정, 그 외 연속성보정 카이제곱)
- [x] `ptc-analyze::stats`: 부트스트랩 CI (B=10,000, 결정론적 xorshift 시드)
- [x] 최종 리포트(`m4-gate`): 통과율·절감 비율·95% CI·재현 정보(시드·프롬프트 버전·모델 ID)

**게이트 (전부 충족):**
- [x] Q1: PTC 통과율이 baseline 대비 비열등 (McNemar `degraded(0.05)` 거짓)
- [x] Q2: LLM 호출·토큰 절감의 95% CI 상한이 모두 1.0 미만
- [x] R≥10, HARNESS_BUG 0건, PTC 전 태스크 통과(1.0)

> ✅ **M4 Mock 게이트 통과** (`cargo run -p ptc-harness --bin m4-gate`). 22 태스크 × 2 모드 × R10.
> **결과: PTC 비열등(p=1.0) · LLM 호출 76.3% 절감(CI [0.193, 0.301]) · 토큰 77.4% 절감(CI [0.183, 0.293]).**
> 설계 판단: baseline ReAct 스크립트를 **PTC 레퍼런스 트레이스에서 도출**(같은 도구 호출 순서 재현, 호출마다 LLM 재호출) → 공정한 비교. 통계는 외부 크레이트 없이 결정론적 구현(재현성). 테스트 223개 + 라이브 3개 `#[ignore]`.

**이 단계의 클린코드 초점:**
- **점추정만 보고 금지**: 항상 신뢰구간과 함께. 측정의 정직성
- **DRY**: PTC/baseline이 동일한 MockToolServer·채점기를 공유 (차이는 LLM 호출 방식뿐)

#### M4 작업 티켓

| 티켓 | 제목 | 의존 | 상태 | 완료 기준 (요약) |
|------|------|------|------|------------------|
| M4-T01 | ReAct 액션 프로토콜 (`react.rs`) | — | ✅ | `CALL`/`FINAL` 렌더·파싱, Value↔JSON 라운드트립 |
| M4-T02 | Baseline ReAct 러너 (`baseline.rs`) | T01 | ✅ | 도구마다 LLM 재호출, MockToolServer·Grader 공유 |
| M4-T03 | 스크립트 baseline mock | T02 | ✅ | 레퍼런스 트레이스→ReAct 스크립트(`baseline_provider`) |
| M4-T04 | McNemar 검정 | — | ✅ | 정확 이항검정(<25) + 카이제곱, `degraded` 판정 |
| M4-T05 | 부트스트랩 CI | — | ✅ | B=10,000 결정론 xorshift, 비율 95% CI |
| M4-T06 | M4 게이트 + 최종 리포트 | T02–T05 | ✅ | Q1 비열등 + Q2 절감 CI 상한<1.0, R≥10 |

---

## 4. 마일스톤 의존성 요약

| 단계 | 핵심 산출물 | 게이트 한 줄 | 선행 |
|------|------------|-------------|------|
| M0 | trait·로깅 골격 | 3 provider ping + RunRecord 적재 | — |
| M1 | DSL 인터프리터 | 수기 스크립트 10개 트레이스 일치, 커버리지 ≥80% | M0 |
| M2 | 단일 E2E | 1 태스크 통과율 ≥0.8, HARNESS_BUG 0건 | M1 |
| M3 | 20+ 태스크 스위트 | 재현 가능한 통과율 표 (3 provider) | M2 |
| M4 | 1.0 vs 2.0 | McNemar+CI로 절감 입증 | M3 |

---

## 5. 리스크와 완화 (설계 7장 연동)

| 리스크 | 완화책 | 관련 마일스톤 |
|--------|--------|--------------|
| LLM 비결정성으로 측정 불안정 | 시드 고정 + 통과율(R회) 측정, CI로 불확실성 표현 | M2~M4 |
| DSL 문법 부족으로 PARSE_ERROR 빈발 | taxonomy로 분리 집계, 임계 초과 시 문법 점진 확장 | M3 |
| Baseline 1.0 불공정 약화 | 두 모드 프롬프트·스텁 동등화, 합리적 ReAct 제공 | M4 |
| L3 채점관 편향·비결정성 | L1/L2 우선 설계, 불가피 시 인간 스팟체크 보정 | M3 |
| Mock과 실제 MCP 괴리 | `ToolSink` trait 공유로 인터프리터 동일, 별도 통합 단계서 실서버 검증 | M1 이후 |
| provider API 변경·비용 | 모델 ID·API 버전 RunRecord 고정, Ollama 로컬 비용 0 경로 | 전 단계 |

---

## 6. 정의 of Done (PR 머지 체크리스트)

모든 PR은 다음을 만족해야 머지한다 (`clean-code.md` 13장 휴리스틱 기반):

- [ ] `cargo fmt --check` / `cargo clippy -D warnings` 통과
- [ ] 새/변경 로직에 단위 테스트 존재, 경계 조건·모든 분기 커버
- [ ] 함수가 한 가지 일만 하는가, 인수 3개 이하, 플래그 인수 없는가
- [ ] 이름이 의도를 드러내고 추상화 수준에 맞는가
- [ ] 죽은 코드·주석 처리된 코드·중복(DRY 위반) 없는가
- [ ] 외부 경계가 trait/래퍼로 격리되어 있는가
- [ ] 해당 마일스톤 게이트에 영향을 주는 변경이면 게이트 재실행 통과
