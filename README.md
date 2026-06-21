# Tool Calling 2.0 — PTC 구현 & LLM 검증 하네스

ONEAI Rust Agent에 **Programmatic Tool Calling(PTC, "Tool Calling 2.0")** 을 도입하기 전에,
프로덕션 코드베이스와 분리된 독립 저장소에서 PTC를 Rust로 구현하고 **LLM으로 자동 검증**하기 위한
하네스(harness)입니다.

> "하네스"란 구현 대상을 반복적으로 **실행 · 측정 · 비교**하기 위한 자동화된 테스트 골격을 말합니다.

---

## 1. 이 프로젝트의 목적

기존 **Tool Calling 1.0(ReAct 방식)** 은 도구를 한 번 호출할 때마다 LLM에게 결과를 돌려주고
다음 행동을 다시 묻습니다. 도구를 N번 호출하려면 LLM을 N+1번 왕복해야 하므로,
호출 횟수와 토큰 소비가 크게 늘어납니다.

**Tool Calling 2.0(PTC)** 은 LLM이 **오케스트레이션 코드 한 덩어리**(루프·조건·중첩 도구 호출이
포함된 작은 프로그램)를 한 번에 생성하고, 그 코드를 우리가 만든 인터프리터가 실행합니다.
LLM 왕복은 원칙적으로 **1번**으로 줄어듭니다.

이 하네스는 두 가지 질문에 **통계적 증거**로 답하는 것을 목표로 합니다.

| 질문 | 측정 대상 | 방법 |
|------|-----------|------|
| **Q1. 정확성** | LLM이 생성한 코드가 올바른 도구 호출 순서로 실행되어 기대한 최종 답에 도달하는가? | 짝지어진 정오(pass/fail) 비교 → **McNemar 검정** |
| **Q2. 성능** | 같은 작업에서 PTC가 1.0 대비 LLM 호출 횟수·토큰을 실제로 줄이는가? | **부트스트랩 신뢰구간(bootstrap CI)** |

LLM은 비결정적이므로, 단일 실행의 우연을 배제하기 위해 **동일 태스크를 시드 고정으로 R회 반복**하여
통과율로 측정합니다.

### 1.1 핵심 설계 원칙 — Milestone-Gated

하네스는 5개 마일스톤(M0–M4)으로 나뉘며, 각 단계는 **자동 판정 가능한 게이트 통과 기준**을
가집니다. 이전 게이트를 통과하지 못하면 다음 단계로 진행하지 않습니다.
이는 "LLM이 코드를 잘 못 만드는 문제", "인터프리터 버그", "채점 로직 오류"가 뒤섞여
원인 분석이 불가능해지는 상황을 막기 위함입니다.

| 단계 | 이름 | 게이트 요약 | 상태 |
|------|------|-------------|------|
| M0 | 스캐폴딩 | LLM provider 골격 + RunRecord 로깅 동작 | ✅ |
| M1 | DSL 코어 | 수기 DSL 스크립트 10개가 인터프리터를 정확히 통과, 커버리지 ≥80% | ✅ |
| M2 | 단일 태스크 E2E | LLM→추출→검증→실행→채점 전 경로 연결, 통과율 1.0 | ✅ |
| M3 | 태스크 스위트 | 22개 태스크 × 3 provider 통과율 표 재현 | ✅ |
| M4 | 1.0 vs 2.0 비교 | McNemar 비열등 + 절감 CI로 입증 | ✅ |

> **M4 측정 결과(Mock 경로):** PTC 비열등(p=1.0) · LLM **호출 76.3% 절감**(CI [0.193, 0.301]) ·
> **토큰 77.4% 절감**(CI [0.183, 0.293]).

---

## 2. 아키텍처 — Cargo 워크스페이스 구성

크레이트 경계 = 책임 경계(SRP). 각 크레이트는 한 가지 관심사만 압니다.

```
tool_calling_2.0/
├── Cargo.toml              # [workspace] — 아래 5개 크레이트
├── crates/
│   ├── ptc-dsl/            # 언어: lexer · parser · ast · validator · interp
│   ├── ptc-llm/            # LlmProvider trait + 구현체(mock/anthropic/openai/ollama)
│   ├── ptc-tools/          # MockToolServer + 도구 카탈로그
│   ├── ptc-harness/        # 오케스트레이션: runner · grader · record · 게이트 바이너리
│   └── ptc-analyze/        # 통계: McNemar 검정 · 부트스트랩 CI
├── tasks/                  # 골든 태스크 22개 (선언적 TOML)
├── prompts/                # 시스템 프롬프트 (버전 관리)
├── harness.md              # 엔지니어링 설계 문서 (무엇을/왜)
├── plan.md                 # 구현 계획서 (어떤 순서로/어떻게)
└── clean-code.md           # 코딩 규범 (머지 기준)
```

- **`ptc-dsl`** — LLM이 생성하는 작은 언어. 들여쓰기 기반 렉서, EBNF 파서, 실행 전 정적
  검증기(validator), tree-walking 인터프리터로 구성. 인터프리터는 `ToolSink` trait 뒤만
  보므로 MCP/HTTP를 전혀 모릅니다.
- **`ptc-llm`** — 외부 LLM API를 `LlmProvider` trait 뒤로 격리. `MockProvider`(결정론적,
  네트워크 불필요)와 실제 provider 3종(Anthropic/OpenAI/Ollama)을 교체 가능.
- **`ptc-tools`** — `MockToolServer`. 모든 도구 호출을 `trace`에 순서대로 기록(채점 근거),
  같은 인자 → 항상 같은 결과(결정론적).
- **`ptc-harness`** — 전체 파이프라인 오케스트레이션과 마일스톤 게이트 실행 진입점.
- **`ptc-analyze`** — JSONL 결과를 읽어 통과율 표·실패 분포·통계 검정을 산출.

---

## 3. 빌드 방법

### 사전 요구사항
- **Rust stable 툴체인** (rustup 권장). 워크스페이스는 `edition = "2021"`, resolver 2를 사용합니다.
- 외부 시스템 의존성 없음. 빌드는 네트워크 없이 완결됩니다.

### 명령

```bash
# 전체 워크스페이스 빌드
cargo build --workspace

# 릴리스 빌드 (게이트를 빠르게 돌리고 싶을 때)
cargo build --workspace --release

# 포맷 검사 + 린트 (CI 머지 기준과 동일)
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
```

> `clippy` 경고 0건이 머지 기준입니다(`-D warnings`로 경고를 에러로 승격).

---

## 4. 테스트 방법

이 프로젝트의 검증은 **두 층**으로 나뉩니다.

1. **단위/통합 테스트** (`cargo test`) — 각 크레이트 내부 로직의 정확성을 결정론적으로 검증.
2. **마일스톤 게이트 바이너리** (`m1-gate` … `m4-gate`) — 마일스톤 전체가 충족됐는지
   객관적 조건으로 판정하는 **종단 검증**. CI가 이 순서대로 실행합니다.

게이트는 모두 **`MockProvider`(결정론적 LLM 대역)** 로 동작하므로 네트워크·API 키 없이
재현 가능합니다. 실제 LLM 호출이 필요한 라이브 측정은 별도 바이너리(`m2-live`, `m3-live`)로
분리되어 있습니다.

### 4.1 단위/통합 테스트 — `cargo test`

```bash
cargo test --workspace
```

**무엇을 검증하나요?** 각 크레이트의 내부 동작을 작은 단위로 쪼개어 검증합니다.

- **`ptc-dsl`**: 렉서가 들여쓰기를 INDENT/DEDENT 토큰으로 올바르게 변환하는지, 파서가 연산자
  우선순위와 중첩 도구 호출을 정확히 AST로 만드는지, 검증기가 **거부해야 할 입력**(미등록 도구
  호출, 중첩 깊이 초과, 허용되지 않은 문법 노드)을 실제로 거부하는지, 인터프리터가
  `for`/`if`/`emit`과 4가지 인자 평가 케이스(리터럴/변수/필드접근/중첩호출)를 정확히 실행하는지.
  거부되어야 할 입력을 검증하는 **음성 테스트(negative test)** 를 반드시 포함합니다.
- **`ptc-tools`**: `MockToolServer`가 같은 인자에 항상 같은 결과를 돌려주고(결정론), 모든 호출을
  trace에 순서대로 적재하는지.
- **`ptc-llm`**: `MockProvider`가 프롬프트에 대해 고정된 코드 텍스트와 토큰 회계를 반환하는지.
- **`ptc-harness`**: 코드 추출 규칙(코드 펜스 처리), 채점기(ExactMatch/TraceMatch), 실패
  taxonomy 6분류 매핑, 러너 오케스트레이션.
- **`ptc-analyze`**: McNemar 검정과 부트스트랩 CI가 알려진 입력에 대해 기대한 통계값을 내는지.

> 실제 LLM을 호출하는 라이브 테스트는 `#[ignore]`로 표시되어 있어 기본 `cargo test`에서는
> 실행되지 않습니다(네트워크·키 필요). 의도적으로 켤 때만 `--ignored`로 실행합니다.

### 4.2 M1 게이트 — DSL 인터프리터 신뢰 기준선

```bash
cargo run -p ptc-harness --bin m1-gate
```

**무엇을 검증하나요?** **LLM을 전혀 쓰지 않고**, 손으로 작성한 DSL 스크립트 10개가
파서→검증기→인터프리터를 거쳐 mock 도구를 **정확한 순서로** 호출하는지 확인합니다.
이 게이트의 의미는 *"인터프리터는 옳다"* 는 신뢰 기준선을 만드는 것입니다. 인터프리터가
틀리면 이후 모든 LLM 측정이 의미를 잃기 때문에, 가장 먼저 통과해야 하는 관문입니다.

통과 조건:
- 수기 스크립트 10개가 **기대 트레이스(도구 호출 순서)와 최종 출력에 100% 일치**.
- 검증기가 미등록 도구·깊이 초과·금지 노드를 **모두 거부**(음성 케이스 전부).
- 인터프리터 단위테스트 라인 커버리지 **≥80%**.

### 4.3 M2 게이트 — 단일 태스크 E2E (Mock)

```bash
cargo run -p ptc-harness --bin m2-gate
```

**무엇을 검증하나요?** LLM이 우리 DSL을 생성하고 → 응답에서 **코드를 추출**하고 →
검증기를 통과시켜 → 인터프리터로 **실행**하고 → 결과를 **채점**하는 전체 파이프라인을
처음으로 끝까지 연결합니다. 여기서는 LLM 자리에 `MockProvider`(정해진 정답 코드를 돌려주는
결정론적 대역)를 끼워, 파이프라인 자체의 배선이 올바른지만 격리해서 봅니다.

통과 조건:
- 골든 태스크 1개를 **R=5회 반복**하여 통과율 **1.0**.
- 실패가 발생하면 **실패 taxonomy 6분류**(EXTRACTION_FAIL / PARSE_ERROR / VALIDATION_REJECT /
  RUNTIME_ERROR / WRONG_ANSWER / HARNESS_BUG) 중 하나로 정확히 분류되어 기록됨.
- **HARNESS_BUG 0건** — 측정 도구 자체의 고장이 있으면 게이트를 통과로 인정하지 않습니다.

### 4.4 M3 게이트 — 태스크 스위트 & 재현성 (Mock)

```bash
cargo run -p ptc-harness --bin m3-gate
```

**무엇을 검증하나요?** 규모를 키워 정확성을 **통계적으로 의미 있게** 측정합니다.
난이도(easy 7 / medium 8 / hard 7)와 도메인(hr / finance / schedule)으로 계층화된
**골든 태스크 22개**를, 이름만 다른 결정론적 Mock provider **3종**으로 **각 R=5회**
실행합니다(22 × 3 × 5 = 330 실행).

통과 조건:
- **동일 커밋·동일 시드로 두 번 실행하면 통과율 표가 완전히 동일**하게 재현됨(재현성).
- 문법 부족으로 인한 **PARSE_ERROR가 전체의 10% 미만**(Mock 경로에서는 0%).
- HARNESS_BUG 0건 + 레퍼런스 해법 전체 통과율 1.0.

> 골든 태스크의 **기대값은 손으로 계산하지 않습니다.** 레퍼런스 해법(`suite.rs`)을 mock에서
> 실행해 정답을 도출하므로(`gen-tasks` 바이너리가 `tasks/*.toml` 생성), 산수 실수로 정답이
> 어긋날 여지를 제거합니다.

### 4.5 M4 게이트 — Tool Calling 1.0 vs 2.0 비교 (최종 목표)

```bash
cargo run -p ptc-harness --bin m4-gate
```

**무엇을 검증하나요?** PTC(2.0)의 정확성·성능 이점을 **baseline 1.0(ReAct)** 대비
통계적으로 입증합니다. ReAct 모드는 도구 결과가 나올 때마다 LLM을 다시 호출하므로
LLM 호출 수 = 도구 수 + 1이 됩니다. **공정성을 위해 두 모드는 동일한 MockToolServer와
동일한 채점기를 공유**하며, 차이는 오직 LLM 호출 방식뿐입니다. 22개 태스크를 두 모드로
**각 R=10회** 실행합니다.

통과 조건:
- **Q1(정확성):** PTC 통과율이 baseline 대비 **비열등**(McNemar 검정으로 유의한 저하 없음).
- **Q2(성능):** LLM 호출 절감과 토큰 절감의 **95% 신뢰구간 상한이 모두 1.0 미만**
  (= 절감이 우연이 아님을 통계적으로 보장).
- R≥10, HARNESS_BUG 0건, PTC 전 태스크 통과(1.0).

이 게이트는 통과율·절감 비율·95% CI·재현 정보(시드·프롬프트 버전·모델 ID)를 담은
**최종 리포트**를 출력합니다.

### 4.6 보조 도구 — 골든 태스크 재생성

```bash
cargo run -p ptc-harness --bin gen-tasks
```

레퍼런스 해법을 mock에서 실행해 `tasks/*.toml`의 기대값을 다시 생성합니다.
태스크를 추가/수정한 뒤 정답을 갱신할 때 사용합니다.

### 4.7 라이브 측정 (선택) — 실제 LLM provider

게이트는 결정론을 위해 Mock으로 동작합니다. **실제 LLM의 통과율**을 측정하려면 별도
라이브 바이너리를 사용하며, 환경 변수로 provider를 선택합니다.

```bash
# M2 라이브 — Anthropic 1종, 단일 태스크 R=5 통과율 ≥0.8 기준
ANTHROPIC_API_KEY=... cargo run -p ptc-harness --bin m2-live

# M3 라이브 — 설정된 provider 전부로 22개 스위트 비교
#   (아래 중 하나 이상 설정 시 해당 provider 활성화)
ANTHROPIC_API_KEY=...                 # Anthropic
OPENAI_API_KEY=...                    # OpenAI 호환
OLLAMA_HOST=http://localhost:11434    # 로컬 Ollama (비용 0 경로)
ANTHROPIC_API_KEY=... cargo run -p ptc-harness --bin m3-live
```

> 라이브 측정은 비결정적이고 네트워크·키·비용이 들기 때문에 CI 기본 파이프라인에서는
> 제외됩니다. 재현성이 필요한 검증은 모두 Mock 게이트로 수행합니다.

### 4.8 CI에서의 전체 순서

`.github/workflows/ci.yml`은 push/PR마다 다음을 순서대로 강제합니다.

```
cargo fmt --all --check        # 형식 = 의사소통, 단일 규칙 강제
cargo clippy -D warnings       # 경고 0건
cargo test --workspace         # 단위/통합 테스트
m1-gate → m2-gate → m3-gate → m4-gate   # 마일스톤 게이트 순차 통과
```

게이트는 마일스톤 순서대로 실행되어, 하위 신뢰 기준선(인터프리터 정확성)부터
최종 비교(1.0 vs 2.0)까지 한 번에 검증됩니다.

---

## 5. 더 읽을거리

| 문서 | 내용 |
|------|------|
| [`harness.md`](./harness.md) | 엔지니어링 설계 문서 — **무엇을/왜** (아키텍처·DSL 명세·통계 방법론) |
| [`plan.md`](./plan.md) | 구현 계획서 — **어떤 순서로/어떻게** (마일스톤별 티켓과 게이트) |
| [`clean-code.md`](./clean-code.md) | 코딩 규범 — PR 머지 기준 |
