# Tool Calling 2.0 — Rust Implementation & LLM Validation Harness

**ENGINEERING DESIGN DOCUMENT**

> 별도 검증 저장소에서 Programmatic Tool Calling을 Rust로 구현하고, 정확성과 성능을 LLM으로 측정하기 위한 하네스 설계

| 항목 | 내용 |
|------|------|
| 프로젝트 | ONEAI — C++/Rust MCP 기반 AI 프레임워크 |
| 대상 컴포넌트 | Rust Agent · Tool Calling 2.0 (PTC) |
| 검증 저장소 | `oneai-ptc-harness` (신규, 독립 repo) |
| 코드 실행 엔진 | 커스텀 DSL (Parser + Tree-walking Interpreter) |
| LLM 백엔드 | Anthropic · Ollama · OpenAI 호환 (swappable) |
| 문서 상태 | Draft v0.1 — 리뷰 대기 |
| 작성 | ONEAI Engineering |

---

## 목차

- [0. 문서 요약](#0-문서-요약)
- [1. 아키텍처 개요](#1-아키텍처-개요)
  - [1.1 세 개의 교체 가능한 축](#11-세-개의-교체-가능한-축)
  - [1.2 데이터 흐름](#12-데이터-흐름)
  - [1.3 저장소 레이아웃](#13-저장소-레이아웃)
- [2. 커스텀 DSL 명세](#2-커스텀-dsl-명세)
  - [2.1 설계 원칙](#21-설계-원칙)
  - [2.2 문법 (EBNF)](#22-문법-ebnf)
  - [2.3 AST 정의 (Rust)](#23-ast-정의-rust)
  - [2.4 값 모델과 도구 호출 경계](#24-값-모델과-도구-호출-경계)
  - [2.5 인자 평가의 4가지 케이스](#25-인자-평가의-4가지-케이스)
- [3. 하네스 구조](#3-하네스-구조)
  - [3.1 LlmProvider trait](#31-llmprovider-trait)
  - [3.2 코드 추출](#32-코드-추출)
  - [3.3 MockToolServer와 도구 카탈로그](#33-mocktoolserver와-도구-카탈로그)
  - [3.4 채점기 (Grader)](#34-채점기-grader)
  - [3.5 RunRecord 스키마](#35-runrecord-스키마)
- [4. LLM 검증 프로토콜](#4-llm-검증-프로토콜)
  - [4.1 두 가지 모드 — PTC vs Baseline 1.0](#41-두-가지-모드--ptc-vs-baseline-10)
  - [4.2 골든 태스크 설계](#42-골든-태스크-설계)
  - [4.3 반복과 시드](#43-반복과-시드)
  - [4.4 실패 분류 (Taxonomy)](#44-실패-분류-taxonomy)
- [5. 측정과 통계 분석](#5-측정과-통계-분석)
  - [5.1 Q1 정확성 — McNemar 검정](#51-q1-정확성--mcnemar-검정)
  - [5.2 Q2 성능 — 부트스트랩 신뢰구간](#52-q2-성능--부트스트랩-신뢰구간)
  - [5.3 보고 산출물](#53-보고-산출물)
- [6. 마일스톤 상세 (M0–M4)](#6-마일스톤-상세-m0m4)
- [7. 리스크와 완화](#7-리스크와-완화)
- [8. 참고 문헌](#8-참고-문헌)

---

## 0. 문서 요약

**목적.** 이 문서는 ONEAI의 Rust Agent에 Tool Calling 2.0(Programmatic Tool Calling, 이하 PTC)을 도입하기 전에, 프로덕션 코드베이스와 분리된 독립 저장소에서 PTC를 구현하고, LLM으로 자동 검증하기 위한 **하네스(harness)의 엔지니어링 설계**를 정의한다. 하네스란 구현 대상을 반복적으로 실행·측정·비교하기 위한 자동화된 테스트 골격을 말한다.

**범위.** 하네스는 두 가지를 측정한다. **(1) 정확성** — LLM이 생성한 오케스트레이션 코드가 올바른 도구 호출 시퀀스로 실행되어 기대한 최종 답에 도달하는가. **(2) 성능** — 같은 작업을 Tool Calling 1.0(매 호출마다 LLM 왕복) 방식과 비교했을 때, LLM 호출 횟수와 토큰 소비가 실제로 줄어드는가. 코드 실행 엔진은 ONEAI 정식 방향인 커스텀 DSL(Parser + Tree-walking Interpreter)을 사용한다.

> **✅ 핵심 설계 원칙 — Milestone-Gated**
>
> 하네스는 5개의 마일스톤(M0–M4)으로 나뉘며, 각 마일스톤은 **게이트(gate) 통과 기준**을 가진다. 이전 마일스톤의 게이트를 통과하지 못하면 다음 단계로 진행하지 않는다. 이는 "LLM이 코드를 잘 생성하지 못하는 문제"와 "인터프리터 버그"와 "채점 로직 오류"가 뒤섞여 원인 분석이 불가능해지는 상황을 방지한다.

**측정 방법론.** 정확성 비교는 항목별 정오(pass/fail)가 짝지어지는 구조이므로 **McNemar 검정**으로 두 방식(1.0 vs 2.0)의 차이가 통계적으로 유의한지 판정하고, 성능 지표(토큰·호출 수)는 **부트스트랩 신뢰구간(bootstrap CI)**으로 불확실성을 표현한다. 단일 실행의 우연을 배제하기 위해 동일 태스크를 시드 고정으로 N회 반복한다.

### 0.1 한눈에 보는 마일스톤

| 단계 | 이름 | 게이트 통과 기준 (요약) |
|------|------|------------------------|
| M0 | 스캐폴딩 | 빈 하네스가 LLM provider 3종에 ping 성공, 로그·시드 고정 동작 |
| M1 | DSL 코어 | 수기 작성 DSL 스크립트가 파서·검증기·인터프리터를 통과, mock 도구 호출 정확 |
| M2 | 단일 태스크 E2E | LLM이 생성한 코드가 1개 골든 태스크에서 정답 도달 (provider 1종) |
| M3 | 태스크 스위트 | 20+ 태스크에서 정확성 측정, 3개 provider 비교, 결과 재현 가능 |
| M4 | 1.0 vs 2.0 비교 | McNemar + bootstrap CI로 호출·토큰 절감 통계적으로 입증 |

이후 본문은 **아키텍처(1장) → DSL 명세(2장) → 하네스 구조(3장) → LLM 검증 프로토콜(4장) → 측정·통계(5장) → 마일스톤 상세(6장) → 리스크(7장)** 순으로 전개한다.

---

## 1. 아키텍처 개요

하네스는 세 개의 독립적으로 교체 가능한 축을 가진다. 각 축은 Rust trait로 추상화되어, 구현을 바꿔도 나머지 코드가 영향을 받지 않는다. 이는 ONEAI의 `ISearchBackend` 패턴(swappable trait)을 PTC 하네스 전체로 확장한 것이다.

### 1.1 세 개의 교체 가능한 축

| 축 | Rust trait | 역할 | 초기 구현체 |
|----|-----------|------|------------|
| LLM Provider | `LlmProvider` | 프롬프트→코드 텍스트 생성 | Anthropic / Ollama / OpenAI |
| 실행 엔진 | `ToolExecutor` | 코드 텍스트→도구 호출 실행 | `DslInterpreter` (커스텀) |
| 채점기 | `Grader` | 실행 결과→pass/fail 판정 | ExactMatch / Semantic |

이 세 축이 만나는 지점이 **하네스 러너(harness runner)**다. 러너는 태스크를 받아 LLM에 코드를 요청하고, 실행 엔진으로 코드를 돌리고, 채점기로 결과를 판정한 뒤, 모든 중간 산물(생성된 코드, 도구 호출 트레이스, 토큰 수, 지연시간)을 구조화된 로그로 남긴다.

### 1.2 데이터 흐름

단일 태스크 1회 실행 시 데이터 흐름은 다음과 같다:

```
  ┌──────────┐   prompt    ┌──────────────┐   code text   ┌──────────────┐
  │  Task    │ ──────────▶ │ LlmProvider  │ ────────────▶ │ ToolExecutor │
  │  (golden)│             │ (3종 중 1)   │               │ (DSL interp) │
  └──────────┘             └──────────────┘               └──────┬───────┘
       │                                                          │
       │ expected                              tool calls │ via MockToolServer
       │ answer                                           ▼
       │                                          ┌──────────────┐
       │                                          │  Tool Trace  │
       │                                          │  + final out │
       ▼                                          └──────┬───────┘
  ┌──────────┐                                          │
  │  Grader  │ ◀────────────────────────────────────────┘
  └────┬─────┘
       │ pass/fail + metrics (tokens, llm_calls, tool_calls, latency)
       ▼
  ┌──────────────────────────────────────────┐
  │  RunRecord (JSONL)  →  분석 파이프라인     │
  └──────────────────────────────────────────┘
```

> **ℹ️ 왜 MockToolServer인가**
>
> 검증 단계에서는 실제 HR DB나 외부 API를 호출하지 않는다. 도구는 **결정론적 mock**으로 구현해, 같은 인자에 항상 같은 결과를 돌려준다. 이렇게 해야 "LLM 코드가 틀렸는지"와 "외부 시스템이 흔들렸는지"를 분리할 수 있고, 채점의 기준 답(ground truth)을 미리 계산해둘 수 있다. 실제 MCP 서버 연동은 하네스 검증을 통과한 후 별도 단계에서 다룬다.

### 1.3 저장소 레이아웃

프로덕션 ONEAI 저장소를 오염시키지 않도록 **독립 저장소 `oneai-ptc-harness`**를 둔다. Cargo 워크스페이스로 구성해 크레이트 경계를 명확히 한다.

```
oneai-ptc-harness/
├── Cargo.toml                 # workspace
├── crates/
│   ├── ptc-dsl/               # 2장: DSL 파서·AST·인터프리터
│   │   ├── src/lexer.rs
│   │   ├── src/parser.rs
│   │   ├── src/ast.rs
│   │   ├── src/validator.rs
│   │   └── src/interp.rs
│   ├── ptc-llm/               # LlmProvider trait + 3 구현체
│   │   ├── src/lib.rs         # trait 정의
│   │   ├── src/anthropic.rs
│   │   ├── src/ollama.rs
│   │   └── src/openai.rs
│   ├── ptc-tools/             # MockToolServer + 도구 카탈로그
│   ├── ptc-harness/           # 러너·채점기·로깅
│   │   ├── src/runner.rs
│   │   ├── src/grader.rs
│   │   └── src/record.rs
│   └── ptc-analyze/           # 5장: McNemar·bootstrap (또는 Python)
├── tasks/                     # 골든 태스크 정의 (TOML/JSON)
├── prompts/                   # 시스템 프롬프트 템플릿 (버전 관리)
└── results/                   # RunRecord JSONL 출력
```

---

## 2. 커스텀 DSL 명세

LLM은 범용 Python을 쓰고 싶어 하지만, 임베디드 환경에서 Python 인터프리터(30MB+)를 싣는 것은 비현실적이고 보안 표면도 너무 넓다. 대신 ONEAI는 **도구 오케스트레이션에만 특화된 작은 DSL**을 정의한다. SQL·정규식·CSS처럼 "한 가지 일만 잘하는" 언어다. 목표는 인터프리터를 약 18KB 규모로 유지하면서, LLM이 자연스럽게 생성할 수 있을 만큼 Python과 닮은 문법을 갖는 것이다.

### 2.1 설계 원칙

- **Python 부분집합 문법.** LLM이 별도 학습 없이 생성할 수 있도록 대입·for·if·함수 호출을 Python과 동일한 표기로 둔다.
- **부수효과 없음.** 파일·네트워크·import·eval이 문법 자체에 존재하지 않는다. 유일한 외부 효과는 등록된 도구 호출뿐이다.
- **정적 검증 가능.** 실행 전에 AST를 훑어 미등록 도구 호출, 중첩 깊이 초과, 허용되지 않은 노드 종류를 모두 거부할 수 있다.
- **결정론적.** 같은 코드 + 같은 도구 mock = 항상 같은 트레이스. 채점과 재현의 전제.

### 2.2 문법 (EBNF)

초기 버전(v0)의 문법은 다음과 같다. 의도적으로 최소 집합만 포함한다:

```ebnf
program     = statement* ;
statement   = assign | for_stmt | if_stmt | emit_stmt | expr_stmt ;

assign      = IDENT "=" expr ;
for_stmt    = "for" IDENT "in" expr ":" block ;
if_stmt     = "if" expr ":" block ( "else" ":" block )? ;
emit_stmt   = "emit" "(" expr ")" ;          // 최종 결과 반환
block       = INDENT statement+ DEDENT ;

expr        = literal
            | IDENT
            | member                              // a.b
            | index                               // a[b]
            | call                                // f(args)
            | binop                               // a + b, a > b ...
            | list_lit ;                          // [a, b, c]

call        = ( IDENT | member ) "(" arglist? ")" ;
member      = expr "." IDENT ;
arglist     = arg ( "," arg )* ;
arg         = expr | IDENT "=" expr ;            // positional or kwarg
literal     = NUMBER | STRING | "True" | "False" | "None" ;
binop       = expr OP expr ;   OP = + - * / > < >= <= == != and or ;
```

> **⚠️ v0에서 의도적으로 제외한 것**
>
> while 루프(무한 루프 위험), 사용자 정의 함수(복잡도), 람다, 컴프리헨션, 예외 처리, 딕셔너리 리터럴. 이들은 M3 이후 태스크가 실제로 요구할 때만 점진적으로 추가한다. 문법이 작을수록 검증과 인터프리터가 단순해진다.

### 2.3 AST 정의 (Rust)

AST는 Rust enum으로 표현한다. `Box`로 재귀 노드를 감싸고, 위치 정보(span)를 붙여 검증 오류 메시지에 줄·열을 표시한다.

```rust
// crates/ptc-dsl/src/ast.rs
#[derive(Debug, Clone)]
pub enum Stmt {
    Assign { name: String, value: Expr, span: Span },
    For    { var: String, iter: Expr, body: Vec<Stmt>, span: Span },
    If     { cond: Expr, then: Vec<Stmt>, els: Vec<Stmt>, span: Span },
    Emit   { value: Expr, span: Span },
    Expr   { expr: Expr, span: Span },
}

#[derive(Debug, Clone)]
pub enum Expr {
    Num(f64), Str(String), Bool(bool), None,
    Var(String),
    List(Vec<Expr>),
    Member { base: Box<Expr>, field: String },
    Index  { base: Box<Expr>, idx: Box<Expr> },
    Call   { callee: Box<Expr>, args: Vec<Arg> },
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
}

#[derive(Debug, Clone)]
pub enum Arg { Pos(Expr), Kw(String, Expr) }
```

### 2.4 값 모델과 도구 호출 경계

인터프리터가 다루는 런타임 값은 좁은 집합으로 제한한다. JSON과 1:1로 매핑되어 도구 호출 결과를 그대로 받을 수 있어야 한다.

```rust
// crates/ptc-dsl/src/interp.rs
#[derive(Debug, Clone)]
pub enum Value {
    Num(f64), Str(String), Bool(bool), Null,
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),   // 도구가 돌려준 객체
}

// 도구 호출은 이 trait 하나로 외부와 만난다.
pub trait ToolSink {
    fn call(&mut self, tool: &str, args: BTreeMap<String, Value>)
        -> Result<Value, ToolError>;
}
```

> **✅ 핵심: 인터프리터는 외부를 모른다**
>
> 인터프리터는 `Call` 노드를 만나면 인자를 평가한 뒤 `ToolSink::call`로 위임할 뿐, MCP·JSON-RPC·HTTP를 전혀 모른다. 검증 단계에서는 `MockToolServer`가, 프로덕션에서는 실제 MCP 클라이언트가 이 trait를 구현한다. 덕분에 같은 인터프리터를 검증과 프로덕션에서 그대로 쓴다.

### 2.5 인자 평가의 4가지 케이스

도구 호출의 인자는 단순 값이 아니라 평가가 필요한 표현식이다. 인터프리터의 `eval_args`는 네 경우를 처리한다. 특히 네 번째(중첩 호출)는 도구 호출이 또 다른 도구 호출을 유발하므로 검증에서 중요하다.

| 케이스 | 예시 | 평가 방식 | 외부 호출 |
|--------|------|----------|-----------|
| 리터럴 | `list_team("eng")` | 값 그대로 | 0 |
| 변수 참조 | `len(team)` | 환경에서 조회 | 0 |
| 필드 접근 | `get_expenses(m.id)` | 값 평가 후 필드 접근 | 0 |
| 중첩 호출 | `notify(format(x))` | 재귀 평가 → 안쪽 호출 먼저 | 1+ (추가 발생) |

---

## 3. 하네스 구조

### 3.1 LlmProvider trait

세 LLM 백엔드를 하나의 trait 뒤에 둔다. 핵심은 **토큰 회계(token accounting)**를 응답에 함께 싣는 것이다. 성능 비교의 1차 지표가 토큰이기 때문이다.

```rust
// crates/ptc-llm/src/lib.rs
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn complete(&self, req: CompletionReq)
        -> Result<CompletionResp, LlmError>;
}

pub struct CompletionReq {
    pub system: String,        // 버전 관리되는 시스템 프롬프트
    pub user: String,          // 태스크 질문 + 도구 스텁
    pub temperature: f32,      // 재현성: 0.0 권장
    pub seed: Option<u64>,     // provider가 지원 시 고정
    pub max_tokens: u32,
}

pub struct CompletionResp {
    pub text: String,          // 생성된 코드(추출 전 원문)
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub stop_reason: String,
    pub latency_ms: u64,
}
```

> **⚠️ 재현성 주의 — 시드와 temperature**
>
> Anthropic·OpenAI는 `temperature=0`에서도 완전 결정론을 보장하지 않으며, Ollama는 `seed` + `temperature=0`으로 비교적 안정적이다. 따라서 정확성은 "1회"가 아니라 "N회 반복 중 통과율"로 측정한다(4.3절). provider별 결정론 한계는 RunRecord에 그대로 기록해 분석 시 감안한다.

### 3.2 코드 추출

LLM은 보통 코드를 설명 문장과 함께, 종종 ` ``` ` 펜스로 감싸 반환한다. 러너는 응답 텍스트에서 코드만 안정적으로 추출해야 한다. 추출 규칙은 명시적으로 정의하고 버전 관리한다:

1. 코드 펜스(` ``` ... ``` `)가 있으면 첫 번째 펜스 블록 내용만 취한다.
2. 펜스가 없으면 전체 텍스트를 코드로 간주하되, 파서가 실패하면 그 응답은 `extraction_fail`로 기록한다.
3. 언어 태그(` ```python `, ` ```dsl `)는 무시한다. 우리 파서가 실제 문법을 검증하므로 태그를 신뢰하지 않는다.

### 3.3 MockToolServer와 도구 카탈로그

도구는 결정론적 mock으로 구현한다. 각 도구는 고정된 시드 데이터셋에서 응답을 만든다. 예를 들어 `list_team("eng")`는 항상 동일한 20명의 직원 목록을, `get_expenses(id, q)`는 직원·분기별로 고정된 지출액을 돌려준다.

```rust
// crates/ptc-tools/src/mock.rs
impl ToolSink for MockToolServer {
    fn call(&mut self, tool: &str, args: BTreeMap<String, Value>)
        -> Result<Value, ToolError>
    {
        self.trace.push(ToolCall { tool: tool.into(), args: args.clone() });
        match tool {
            "list_team"    => self.list_team(&args),
            "get_expenses" => self.get_expenses(&args),
            "get_budget"   => self.get_budget(&args),
            "send_email"   => self.record_email(&args),  // 부수효과도 기록만
            other => Err(ToolError::Unknown(other.into())),
        }
    }
}
```

> **ℹ️ 도구 트레이스가 곧 채점 근거**
>
> MockToolServer는 모든 호출을 순서대로 `trace`에 적재한다. 채점기는 최종 출력값뿐 아니라 **호출 시퀀스 자체**를 기대값과 비교할 수 있다. 예컨대 "20명 각각에 get_expenses를 정확히 1번씩 호출했는가"는 정확성의 중요한 신호다.

### 3.4 채점기 (Grader)

채점기도 trait로 두어 태스크 성격에 맞게 교체한다. 세 가지 레벨을 정의한다:

| 레벨 | 이름 | 판정 기준 | 적용 태스크 |
|------|------|----------|------------|
| L1 | ExactMatch | emit된 최종 값이 기대값과 정확히 일치 | 집계·계산형 (정답이 유일) |
| L2 | TraceMatch | 도구 호출 시퀀스가 기대 집합과 일치 | 절차 정확성이 중요한 태스크 |
| L3 | Semantic | LLM 채점관이 의미적 동치 판정 | 자연어 답변형 (표현 다양) |

**L3 주의.** LLM 채점관(LLM-as-judge)은 편향과 비결정성을 도입한다. 따라서 L3는 가능한 한 피하고, 태스크를 설계할 때 L1/L2로 채점 가능하도록 정답이 수렴하는 질문을 우선 만든다. L3가 불가피하면 채점관 프롬프트도 버전 관리하고, 인간 스팟체크로 채점관 자체를 보정한다.

### 3.5 RunRecord 스키마

모든 실행은 한 줄의 JSON(JSONL)으로 기록되어 분석 파이프라인으로 흘러간다. 재현과 사후 분석을 위해 입력 조건을 빠짐없이 포함한다:

```json
{
  "run_id": "uuid",
  "task_id": "expense_overflow_01",
  "mode": "ptc",              // "ptc" | "baseline_1_0"
  "provider": "anthropic",
  "model": "claude-...",
  "prompt_version": "sys-v3",
  "seed": 42,  "temperature": 0.0,  "repeat_idx": 3,
  "generated_code": "team = hr.list_team(...) ...",
  "extraction": "ok",         // ok | fenced | extraction_fail
  "validation": "pass",       // pass | reject:<reason>
  "tool_trace": [ {"tool":"list_team","args":{...}}, ... ],
  "final_output": "[{...}]",
  "grade": {"level":"L1","pass":true},
  "metrics": {
    "llm_calls": 1, "tool_calls": 22,
    "input_tokens": 1840, "output_tokens": 210,
    "latency_ms": 3120 },
  "error": null
}
```

---

## 4. LLM 검증 프로토콜

이 장은 "LLM을 어떻게 신뢰성 있게 검증 도구로 쓰는가"를 정의한다. LLM은 비결정적이고 프롬프트에 민감하므로, 검증 절차 자체를 통제하지 않으면 측정값을 신뢰할 수 없다.

### 4.1 두 가지 모드 — PTC vs Baseline 1.0

성능 이득을 주장하려면 비교 기준이 필요하다. 하네스는 같은 태스크를 두 모드로 실행한다:

- **PTC 모드.** LLM이 한 번 호출되어 전체 오케스트레이션 코드를 생성하고, 인터프리터가 도구를 N번 호출한다. LLM 호출 = 1.
- **Baseline 1.0 모드.** LLM이 도구를 한 번에 하나씩 호출한다. 도구 결과를 받을 때마다 LLM을 다시 호출해 다음 행동을 결정한다(ReAct 스타일). LLM 호출 = 도구 호출 수 + 1 수준.

두 모드 모두 **동일한 MockToolServer와 동일한 채점기**를 쓴다. 차이는 오직 "LLM을 어떻게 부르는가"뿐이며, 따라서 측정된 호출·토큰 차이는 패러다임 차이에 귀속할 수 있다.

> **⚠️ 공정한 비교를 위한 통제**
>
> Baseline 1.0이 불리하게 보이도록 일부러 약하게 만들면 안 된다. 두 모드의 도구 스텁·시스템 프롬프트 품질을 동등하게 맞추고, baseline에도 합리적인 ReAct 루프(관찰→사고→행동)를 제공한다. 비교의 목적은 "PTC 승리 선언"이 아니라 "어떤 조건에서 PTC가 실제로 유리한가"의 정량화다.

### 4.2 골든 태스크 설계

태스크는 다음 속성을 갖도록 설계한다:

1. **결정론적 정답.** MockToolServer의 고정 데이터로부터 기대 출력을 사전 계산할 수 있어야 한다(L1/L2 채점 가능).
2. **다중 도구 호출 유발.** PTC의 이점이 드러나려면 루프나 다단계 호출이 필요하다. 단일 호출로 끝나는 태스크는 비교 가치가 낮다.
3. **난이도 계층화.** 쉬움(단일 루프)→중간(루프+조건)→어려움(중첩 호출, 교차 참조)으로 나눠, 어디서 PTC 이점/한계가 갈리는지 본다.
4. **도메인 다양성.** HR·재무·일정 등 도구 카탈로그를 여러 도메인으로 구성해 namespace 라우팅도 함께 검증한다.

태스크는 선언적으로 정의한다(코드가 아님). 예:

```toml
# tasks/expense_overflow_01.toml
id = "expense_overflow_01"
tier = "medium"
domains = ["hr", "finance"]
question = "엔지니어링팀에서 Q3 출장 예산을 초과한 사람의 이름을 모두 알려줘."
grader = "L1"

# 사전 계산된 기대값 (MockToolServer 고정 데이터 기준)
expected_output = ["Bob", "Dana", "Frank"]

# 선택: 절차 정확성 검증용 기대 호출 집합
expected_tool_calls = [
  { tool = "list_team", count = 1 },
  { tool = "get_expenses", count = 20 },
  { tool = "get_budget",  count_min = 1 },
]
```

### 4.3 반복과 시드

LLM 비결정성을 다루기 위해 각 (태스크 × provider × 모드) 조합을 **R회 반복**한다(초기값 R=5, M4에서 R=10–20으로 상향). 정확성 지표는 단일 통과가 아니라 **통과율(pass rate) = 통과 횟수 / R**로 정의한다. 이렇게 하면 "운 좋게 한 번 맞춘" 경우와 "안정적으로 맞추는" 경우를 구분할 수 있다.

### 4.4 실패 분류 (Taxonomy)

실패를 단일 'fail'로 뭉뚱그리지 않고 원인별로 분류해, 인터프리터 버그와 LLM 약점을 분리한다:

| 분류 | 의미 | 책임 소재 |
|------|------|----------|
| `EXTRACTION_FAIL` | 응답에서 코드를 추출 못 함 | 프롬프트 / 추출 규칙 |
| `PARSE_ERROR` | 추출된 코드가 DSL 문법 위반 | LLM (잘못된 문법 생성) |
| `VALIDATION_REJECT` | 미등록 도구·깊이 초과 등 | LLM (없는 도구 호출 등) |
| `RUNTIME_ERROR` | 실행 중 타입 오류·키 없음 등 | LLM 로직 / 인터프리터 |
| `WRONG_ANSWER` | 실행됐으나 답이 틀림 | LLM 로직 |
| `HARNESS_BUG` | 채점기·mock 자체 오류 | 하네스 (우리 책임) |

> **🚨 HARNESS_BUG는 게이트를 막는다**
>
> 분석 중 HARNESS_BUG가 1건이라도 발견되면 해당 마일스톤은 통과로 인정하지 않는다. 측정 도구가 고장 난 상태의 숫자는 의미가 없기 때문이다. 이 분류가 있기에 "우리 탓"과 "모델 탓"을 정직하게 가른다.

---

## 5. 측정과 통계 분석

이 장은 RunRecord 모음에서 결론을 끌어내는 방법을 정의한다. 목표는 두 가지 질문에 통계적으로 답하는 것이다: **(Q1) PTC가 baseline보다 정확한가(또는 최소한 비등한가)? (Q2) PTC가 호출·토큰을 유의하게 절감하는가?**

### 5.1 Q1 정확성 — McNemar 검정

같은 태스크 집합을 두 모드로 풀면, 각 태스크는 (PTC 정오, baseline 정오)로 짝지어진다. 이런 **쌍체(paired) 이분 데이터**에는 McNemar 검정이 적합하다. 2×2 분할표에서 두 모드가 엇갈린 칸(b, c)만 본다:

```
                      baseline_1.0
                   pass        fail
  PTC  pass   │    a       │    b     │   ← b: PTC만 맞춤
       fail   │    c       │    d     │   ← c: baseline만 맞춤

  McNemar 통계량 (연속성 보정):
      chi^2 = (|b - c| - 1)^2 / (b + c)
  b+c 가 작으면( < 25 ) 정확검정(이항검정)을 사용:
      p = 2 * sum_{k=0}^{min(b,c)} C(b+c, k) * 0.5^(b+c)
```

귀무가설은 "두 모드의 정확성이 같다"이다. p < 0.05이면 차이가 유의하다고 판정한다. 통과율(연속값)을 함께 보고하되, **태스크 단위 정오는 통과율을 임계값(예: ≥0.5)으로 이진화**하거나, 반복을 고려한 혼합효과 모델로 확장할 수 있다(M4에서 결정).

### 5.2 Q2 성능 — 부트스트랩 신뢰구간

토큰·호출 수는 태스크마다 분포가 크게 다르고 정규분포를 따르지 않는다. 따라서 평균 차이에 모수적 검정을 쓰는 대신, **부트스트랩 재표집**으로 "PTC 토큰 / baseline 토큰" 비율의 신뢰구간을 추정한다:

1. 각 태스크에서 두 모드의 토큰(또는 LLM 호출 수)을 수집한다.
2. 태스크 집합에서 복원추출로 동일 크기 표본을 B회(예: 10,000) 재구성한다.
3. 각 재표본에서 총 PTC 토큰 / 총 baseline 토큰 비율을 계산한다.
4. B개 비율의 2.5/97.5 분위수가 95% 신뢰구간이다.

예: 비율의 95% CI가 `[0.18, 0.27]`이면 "PTC가 baseline 대비 토큰을 73–82% 절감"이라고 신뢰구간과 함께 보고한다. 점추정만 제시하지 않는다.

> **ℹ️ LLM 호출 수는 거의 자명하지만 토큰은 아니다**
>
> LLM 호출 수는 구조상 PTC=1, baseline≈N+1로 거의 결정적이다. 그러나 **토큰**은 다르다. PTC는 긴 코드를 한 번에 생성하고 도구 결과가 인터프리터에 머물러 컨텍스트에 누적되지 않는 반면, baseline은 매 스텝 누적 컨텍스트를 다시 보낸다. 실제 절감폭은 태스크의 루프 횟수·중간 데이터 크기에 따라 달라지므로 반드시 측정해야 한다.

### 5.3 보고 산출물

분석 파이프라인(ptc-analyze 또는 Python 노트북)은 다음을 자동 생성한다:

- provider × tier 별 통과율 표 (PTC vs baseline)
- McNemar p-value 및 분할표 (Q1)
- 토큰·호출 절감 비율과 95% 부트스트랩 CI (Q2)
- 실패 분류 분포 막대그래프 (4.4절 taxonomy)
- 재현 정보: 시드, 프롬프트 버전, 모델 ID, 커밋 해시

---

## 6. 마일스톤 상세 (M0–M4)

은 **산출물(deliverable)**과 **게이트(gate)**를 가진다. 게이트는 객관적으로 판정 가능한 조건이어야 하며, 통과 전까지 다음 단계의 코드를 작성하지 않는다.

### M0 — 스캐폴딩 (Scaffolding)

**목표.** 측정 인프라가 동작함을 먼저 보장한다. 구현 대상(DSL)은 아직 없다.

**산출물.**

- Cargo 워크스페이스, 5개 크레이트 빈 골격
- `LlmProvider` trait + 3 구현체의 `complete()` 가 'ping' 프롬프트에 응답
- Rcord JSONL 로깅, 시드·프롬프트 버전 기록 동작

**게이트.** 3개 provider 모두에 "2+2=?" 프롬프트를 보내 응답·토큰 수·지연시간이 RunRecord에 정상 적재된다. CI에서 mock provider로 전체 파이프라인이 그린.

### M1 — DSL 코어

**목표.** LLM 없이, 손으로 작성한 DSL 스크립트가 파서→검증기→인터프리터를 거쳐 mock 도구를 정확히 호출함을 보장한다.

**산출물.**

- lexer·parser·AST·validator·interpreter êoolServer + 최소 도구 4종 (list_team, get_expenses, get_budget, send_email)
- 수기 DSL 스크립트 10개 + 각 기대 트레이스/출력

**게이트.** 10개 수기 스크립트가 100% 기대 트레이스·출력과 일치. 검증기가 미등록 도구·깊이 초과·금지 노드를 모두 거부(음성 테스트 포함). 인터프리터 단위테스트 커버리지 ≥ 80%.

> **🚨 M1이 가장 중요하다**
>
> M1 게이트를 통과하면 "인터프리터는 옳다"는 신뢰 기준선2E에서 답이 틀리면 원인을 LLM 쪽으로 좁힐 수 있다. M1을 대충 넘기면 이후 모든 측정의 신뢰가 무너진다.

### M2 — 단일 태스크 E2E

**목표.** LLM이 실제로 우리 DSL을 생성하고, 그 코드가 1개 골든 태스크에서 정답에 도달하는 전체 경로를 처음으로 연결한다.

**산출물.**

- 시스템 프롬프트 v1 (DSL 문법 + 도구 스텁 + 예시 1개 포함)
- 코드 추출 로직 (3.2절)
- 골든 태스크 1개 (tier=easy)에 대한이트.** provider 1종(예: Anthropic)으로 R=5 반복 시 통과율 ≥ 0.8. 실패 사례가 4.4 taxonomy로 분류되어 기록됨. HARNESS_BUG 0건.

### M3 — 태스크 스위트

**목표.** 규모를 키워 정확성을 통계적으로 의미 있게 측정하고, 3개 provider를 비교한다.

**산출물.**

- 골든 태스크 20개 이상 (tier·domain 계층화, 4.2절)
- 3개 provider × 전 태스크 × R회 배치 실행 러너
- 통과율 표 + 실패 분류 분포 자동 생성 (5.3절 ì¸.** 동일 커밋·시드로 두 번 실행 시 통과율 표가 재현됨(provider 비결정성 범위 내). 문법 부족으로 인한 PARSE_ERROR가 특정 임계(예: 전체의 10%) 미만 — 초과 시 DSL에 기능 추가 후 재측정.

### M4 — 1.0 vs 2.0 비교

**목표.** PTC의 정확성·성능 이점을 baseline 1.0 대비 통계적으로 입증한다. 이 문서의 최종 목표.

**산출물.**

- Baseline 1.0(ReAct) 모드 구현 (4.1절)
- McNemar + 부트스트랩 CI 분석 (ptc-ana- 최종 리포트: 통과율·절감 비율·CI·실패 분류·재현 정보

**게이트.** Q1: PTC 통과율이 baseline 대비 비열등(non-inferior) 이상(McNemar로 유의한 악화 없음). Q2: 토큰·호출 절감의 95% CI 상한이 1.0 미만(즉 유의한 절감). R≥10. 결과가 시드 변경에도 방향성 유지.

### 6.1 마일스톤 의존성 요약

| 단계 | 핵심 산출물 | 게이트 한 줄 요약 | 선행 |
|------|------------|------------------|------|
| M0 | trait·로깅 ê3 provider ping + RunRecord 적재 | — |
| M1 | DSL 인터프리터 | 수기 스크립트 10개 트레이스 일치 | M0 |
| M2 | 단일 E2E | 1 태스크 통과율 ≥0.8 (1 provider) | M1 |
| M3 | 20+ 태스크 스위트 | 재현 가능한 통과율 표 (3 provider) | M2 |
| M4 | 1.0 vs 2.0 | McNemar+CI로 절감 입증 | M3 |

---

## 7. 리스크와 완화

| 리스크 | 영향 | 완화책 |
|--------|------|--------|
| LLM 비결정성으로 측정 불안정 | 통과율이 실행마다 출렁여 ê 시드 고정 + 통과율로 측정, CI로 불확실성 표현 |
| DSL 문법 부족으로 PARSE_ERROR 빈발 | LLM 잘못이 아닌데 fail로 집계 | PARSE_ERROR를 taxonomy로 분리, 임계 초과 시 문법 점진 확장(M3 게이트) |
| Baseline 1.0을 불공정하게 약화 | PTC 이점이 과장됨 | 두 모드 프롬프트·스텁 품질 동등화, 합리적 ReAct 루프 제공 |
| L3(LLM 채점관) 편향·비결정성 | 채점 자체를 신뢰 못 함 | L1/L2 우선 설계, L3 불가피 시 ì´크로 보정 |
| Mock과 실제 MCP 서버의 괴리 | 하네스 통과해도 프로덕션서 실패 | ToolSink trait 공유로 인터프리터 동일, 별도 통합 단계에서 실서버 검증 |
| provider API 변경·비용 | 재현 불가·예산 초과 | 모델 ID·API 버전 RunRecord에 고정 기록, Ollama 로컬로 비용 0 경로 확보 |

---

## 8. 참고 문헌

**Anthropic — Advanced Tool Use (Tool Calling 2.0)**

- [Advanced Tool Use 발표 (engineering blog)](https://www.anthropic.co/engineering/advanced-tool-use)
- [Programmatic Tool Calling 문서](https://platform.claude.com/docs/en/agents-and-tools/tool-use/programmatic-tool-calling)

**MCP 및 검증 방법론**

- [Model Context Protocol 사양](https://modelcontextprotocol.io)
- [Cloudflare — Code Mode](https://blog.cloudflare.com/code-mode)

**LLM Provider API**

- [Anthropic Messages API](https://docs.claude.com)
- [OpenAI Responses / migrate guide](https://developers.openai.com/api/docs/guides/migrate-to-responses)
- [Ollamps://ollama.com)

**통계**

- McNemar, Q. (1947). Note on the sampling error of the difference between correlated proportions or percentages.
- Efron, B. & Tibshirani, R. (1993). An Introduction to the Bootstrap.

