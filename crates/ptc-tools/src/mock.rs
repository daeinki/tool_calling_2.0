//! MockToolServer — 결정론적 도구를 [`ToolSink`]로 제공한다 (M1-T09 · M3-T01).
//!
//! 검증 단계에서는 실제 HR DB나 외부 API를 부르지 않는다. 모든 도구는 고정
//! 시드 데이터에서 응답을 만들어, 같은 인자에 항상 같은 결과를 돌려준다.
//! 이렇게 해야 "LLM 코드가 틀렸는지"와 "외부 시스템이 흔들렸는지"를 분리할 수 있다.
//!
//! 모든 호출은 순서대로 [`trace`](MockToolServer::trace)에 적재되어 채점 근거가 된다.
//!
//! **도메인(M3-T01).** 도구는 `hr`·`finance`·`schedule` 세 도메인으로 묶인다.
//! 호출은 bare 이름(`list_team`)과 `domain.tool`(`schedule.list_events`) 두 형태를
//! 모두 허용하며, 라우팅은 마지막 마디(tool)만 본다([`base_tool`]).

use ptc_dsl::{ToolError, ToolSink, Value};
use std::collections::BTreeMap;

/// 도메인 → 그 도메인의 조회 도구들. 카탈로그는 bare 이름과 `domain.tool`
/// 별칭을 모두 등록해, LLM이 어느 형태로 불러도 검증을 통과시킨다(M3-T01).
const DOMAINS: [(&str, &[&str]); 3] = [
    ("hr", &["list_team"]),
    ("finance", &["get_expenses", "get_budget"]),
    ("schedule", &["list_events"]),
];

/// 도메인에 속하지 않는 부수효과(action) 도구. 네임스페이스 없이 bare로만 부른다.
const ACTIONS: [&str; 1] = ["send_email"];

/// 카탈로그에 등록할 모든 호출명: 도메인 도구의 bare + `domain.tool` 별칭, 그리고
/// action의 bare 이름. [`ToolCatalog`](ptc_dsl::ToolCatalog) 구성의 단일 출처다.
pub fn tool_names() -> Vec<String> {
    let domain_names = DOMAINS.iter().flat_map(|(namespace, tools)| {
        tools
            .iter()
            .flat_map(move |tool| [tool.to_string(), format!("{namespace}.{tool}")])
    });
    domain_names
        .chain(ACTIONS.iter().map(|tool| tool.to_string()))
        .collect()
}

/// `domain.tool` 형태면 마지막 마디(실제 도구명)를, 아니면 그대로 돌려준다.
/// 라우팅은 네임스페이스에 무관하다 — 같은 도구는 어느 도메인 접두로 불러도 동일.
/// L2 채점도 이 정규화를 공유해, bare·namespaced 호출을 같은 도구로 센다(DRY).
pub fn base_tool(tool: &str) -> &str {
    tool.rsplit('.').next().unwrap_or(tool)
}

/// 적재된 도구 호출 한 건.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub tool: String,
    pub args: BTreeMap<String, Value>,
}

/// 결정론적 mock 도구 서버. 호출 트레이스와 전송된 이메일을 기록한다.
#[derive(Debug, Default)]
pub struct MockToolServer {
    trace: Vec<ToolCall>,
    emails: Vec<BTreeMap<String, Value>>,
}

impl MockToolServer {
    pub fn new() -> Self {
        Self::default()
    }

    /// 들어온 순서대로의 호출 트레이스.
    pub fn trace(&self) -> &[ToolCall] {
        &self.trace
    }

    /// `send_email`로 기록된 이메일들(부수효과를 실행하지 않고 기록만 한다).
    pub fn emails(&self) -> &[BTreeMap<String, Value>] {
        &self.emails
    }

    fn list_team(&self, args: &BTreeMap<String, Value>) -> Result<Value, ToolError> {
        let dept = pick_str(args, 0, "dept").ok_or_else(|| fail("list_team: dept(arg0) 필요"))?;
        Ok(Value::List(members_of(dept)))
    }

    fn get_expenses(&self, args: &BTreeMap<String, Value>) -> Result<Value, ToolError> {
        let id = pick_num(args, 0, "member_id")
            .ok_or_else(|| fail("get_expenses: member_id(arg0) 필요"))?;
        let amount = match pick_str(args, 1, "quarter") {
            Some(quarter) => {
                let index =
                    quarter_index(quarter).ok_or_else(|| fail("get_expenses: 알 수 없는 분기"))?;
                expense_for(id, index)
            }
            // 분기 미지정 시 연간 합계(전 분기 합)를 돌려준다.
            None => (0..QUARTERS).map(|index| expense_for(id, index)).sum(),
        };
        Ok(Value::Num(amount))
    }

    fn get_budget(&self, args: &BTreeMap<String, Value>) -> Result<Value, ToolError> {
        let quarter =
            pick_str(args, 0, "quarter").ok_or_else(|| fail("get_budget: quarter(arg0) 필요"))?;
        let index = quarter_index(quarter).ok_or_else(|| fail("get_budget: 알 수 없는 분기"))?;
        Ok(Value::Num(budget_for(index)))
    }

    fn list_events(&self, args: &BTreeMap<String, Value>) -> Result<Value, ToolError> {
        let person =
            pick_str(args, 0, "person").ok_or_else(|| fail("list_events: person(arg0) 필요"))?;
        Ok(Value::List(events_of(person)))
    }

    fn record_email(&mut self, args: BTreeMap<String, Value>) -> Result<Value, ToolError> {
        self.emails.push(args);
        Ok(Value::Null)
    }
}

impl ToolSink for MockToolServer {
    fn call(&mut self, tool: &str, args: BTreeMap<String, Value>) -> Result<Value, ToolError> {
        // 트레이스에는 호출된 형태(네임스페이스 포함)를 그대로 적재한다 — L2 채점 근거.
        self.trace.push(ToolCall {
            tool: tool.to_string(),
            args: args.clone(),
        });
        match base_tool(tool) {
            "list_team" => self.list_team(&args),
            "get_expenses" => self.get_expenses(&args),
            "get_budget" => self.get_budget(&args),
            "list_events" => self.list_events(&args),
            "send_email" => self.record_email(args),
            _ => Err(ToolError::Unknown(tool.to_string())),
        }
    }
}

// ── 고정 시드 데이터 ──

const QUARTERS: usize = 4;

/// (이름, id, 부서) 고정 로스터.
fn roster() -> [(&'static str, f64, &'static str); 6] {
    [
        ("Alice", 1.0, "eng"),
        ("Bob", 2.0, "eng"),
        ("Carol", 3.0, "eng"),
        ("Dave", 4.0, "eng"),
        ("Erin", 5.0, "sales"),
        ("Frank", 6.0, "sales"),
    ]
}

fn members_of(dept: &str) -> Vec<Value> {
    roster()
        .into_iter()
        .filter(|(_, _, member_dept)| *member_dept == dept)
        .map(|(name, id, member_dept)| member_map(name, id, member_dept))
        .collect()
}

fn member_map(name: &str, id: f64, dept: &str) -> Value {
    let mut map = BTreeMap::new();
    map.insert("id".to_string(), Value::Num(id));
    map.insert("name".to_string(), Value::Str(name.to_string()));
    map.insert("dept".to_string(), Value::Str(dept.to_string()));
    Value::Map(map)
}

fn quarter_index(quarter: &str) -> Option<usize> {
    match quarter {
        "Q1" => Some(0),
        "Q2" => Some(1),
        "Q3" => Some(2),
        "Q4" => Some(3),
        _ => None,
    }
}

/// 분기별 1인 지출액(결정론 공식).
fn expense_for(id: f64, quarter_index: usize) -> f64 {
    id * 1000.0 + quarter_index as f64 * 250.0 + 250.0
}

/// 분기별 1인 출장 예산. Q1 1500 → Q4 3000.
fn budget_for(quarter_index: usize) -> f64 {
    1500.0 + quarter_index as f64 * 500.0
}

/// 사람별 고정 회의 일정. (id, 제목, 시간) 행을 `{id, title, hours}` 맵으로 만든다.
/// 이름은 [`roster`]와 맞물려 hr·schedule 교차 태스크를 가능하게 한다.
fn events_of(person: &str) -> Vec<Value> {
    let rows: &[(f64, &str, f64)] = match person {
        "Alice" => &[(101.0, "standup", 1.0), (102.0, "review", 2.0)],
        "Bob" => &[(201.0, "standup", 1.0), (202.0, "planning", 3.0)],
        "Carol" => &[(301.0, "standup", 1.0)],
        "Dave" => &[
            (401.0, "standup", 1.0),
            (402.0, "1on1", 1.0),
            (403.0, "demo", 2.0),
        ],
        "Erin" => &[(501.0, "pitch", 2.0)],
        "Frank" => &[(601.0, "pitch", 2.0), (602.0, "forecast", 1.0)],
        _ => &[],
    };
    rows.iter()
        .map(|(id, title, hours)| event_map(*id, title, *hours))
        .collect()
}

fn event_map(id: f64, title: &str, hours: f64) -> Value {
    let mut map = BTreeMap::new();
    map.insert("id".to_string(), Value::Num(id));
    map.insert("title".to_string(), Value::Str(title.to_string()));
    map.insert("hours".to_string(), Value::Num(hours));
    Value::Map(map)
}

// ── 인자 읽기 헬퍼 (위치 인자 argN 또는 키워드 이름 모두 허용) ──

fn pick<'a>(args: &'a BTreeMap<String, Value>, index: usize, name: &str) -> Option<&'a Value> {
    args.get(&format!("arg{index}")).or_else(|| args.get(name))
}

fn pick_str<'a>(args: &'a BTreeMap<String, Value>, index: usize, name: &str) -> Option<&'a str> {
    match pick(args, index, name)? {
        Value::Str(s) => Some(s),
        _ => None,
    }
}

fn pick_num(args: &BTreeMap<String, Value>, index: usize, name: &str) -> Option<f64> {
    match pick(args, index, name)? {
        Value::Num(n) => Some(*n),
        _ => None,
    }
}

fn fail(reason: &str) -> ToolError {
    ToolError::Failed(reason.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ptc_dsl::{parse, tokenize, Interpreter};

    /// 위치 인자 맵(arg0, arg1, ...)을 만든다.
    fn pos(values: &[Value]) -> BTreeMap<String, Value> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| (format!("arg{i}"), v.clone()))
            .collect()
    }

    fn str_val(s: &str) -> Value {
        Value::Str(s.to_string())
    }

    #[test]
    fn list_team_is_deterministic() {
        let mut server = MockToolServer::new();
        let first = server.call("list_team", pos(&[str_val("eng")])).unwrap();
        let second = server.call("list_team", pos(&[str_val("eng")])).unwrap();
        assert_eq!(first, second);
        match first {
            Value::List(members) => assert_eq!(members.len(), 4),
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn unknown_department_yields_empty_list() {
        let mut server = MockToolServer::new();
        assert_eq!(
            server.call("list_team", pos(&[str_val("legal")])).unwrap(),
            Value::List(vec![])
        );
    }

    #[test]
    fn quarterly_expense_is_deterministic_and_distinct_from_annual() {
        let mut server = MockToolServer::new();
        let q3 = server
            .call("get_expenses", pos(&[Value::Num(2.0), str_val("Q3")]))
            .unwrap();
        let annual = server
            .call("get_expenses", pos(&[Value::Num(2.0)]))
            .unwrap();
        assert_eq!(q3, Value::Num(2750.0));
        assert_ne!(q3, annual);
    }

    #[test]
    fn budget_table_is_fixed() {
        let mut server = MockToolServer::new();
        assert_eq!(
            server.call("get_budget", pos(&[str_val("Q3")])).unwrap(),
            Value::Num(2500.0)
        );
    }

    #[test]
    fn some_members_exceed_q3_budget_and_some_do_not() {
        // 결정론 데이터의 정합성: Bob(id2)은 초과, Alice(id1)는 미초과.
        let mut server = MockToolServer::new();
        let bob = server
            .call("get_expenses", pos(&[Value::Num(2.0), str_val("Q3")]))
            .unwrap();
        let alice = server
            .call("get_expenses", pos(&[Value::Num(1.0), str_val("Q3")]))
            .unwrap();
        let budget = server.call("get_budget", pos(&[str_val("Q3")])).unwrap();
        assert_eq!(
            (bob, alice, budget),
            (Value::Num(2750.0), Value::Num(1750.0), Value::Num(2500.0))
        );
    }

    #[test]
    fn send_email_records_without_side_effect_and_returns_null() {
        let mut server = MockToolServer::new();
        let mut email = BTreeMap::new();
        email.insert("to".to_string(), str_val("ceo@x.com"));
        let result = server.call("send_email", email.clone()).unwrap();
        assert_eq!(result, Value::Null);
        assert_eq!(server.emails(), &[email]);
    }

    #[test]
    fn unknown_tool_is_rejected() {
        let mut server = MockToolServer::new();
        match server.call("frobnicate", BTreeMap::new()) {
            Err(ToolError::Unknown(name)) => assert_eq!(name, "frobnicate"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn list_events_is_deterministic_and_keyed_by_name() {
        let mut server = MockToolServer::new();
        let first = server
            .call("list_events", pos(&[str_val("Alice")]))
            .unwrap();
        let second = server
            .call("list_events", pos(&[str_val("Alice")]))
            .unwrap();
        assert_eq!(first, second);
        match first {
            Value::List(events) => assert_eq!(events.len(), 2),
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn list_events_for_unknown_person_is_empty() {
        let mut server = MockToolServer::new();
        assert_eq!(
            server.call("list_events", pos(&[str_val("Zoe")])).unwrap(),
            Value::List(vec![])
        );
    }

    #[test]
    fn namespaced_call_routes_to_same_tool_as_bare() {
        // `schedule.list_events`와 `list_events`는 같은 도구로 라우팅된다(M3-T01).
        let mut server = MockToolServer::new();
        let namespaced = server
            .call("schedule.list_events", pos(&[str_val("Bob")]))
            .unwrap();
        let bare = server.call("list_events", pos(&[str_val("Bob")])).unwrap();
        assert_eq!(namespaced, bare);
        // 트레이스에는 호출된 형태가 그대로 남는다.
        assert_eq!(server.trace()[0].tool, "schedule.list_events");
        assert_eq!(server.trace()[1].tool, "list_events");
    }

    #[test]
    fn tool_names_cover_bare_and_namespaced_forms() {
        let names = tool_names();
        assert!(names.iter().any(|n| n == "list_events"));
        assert!(names.iter().any(|n| n == "schedule.list_events"));
        assert!(names.iter().any(|n| n == "hr.list_team"));
        // action은 bare로만 등록된다.
        assert!(names.iter().any(|n| n == "send_email"));
        assert!(!names.iter().any(|n| n == "finance.send_email"));
    }

    #[test]
    fn trace_records_calls_in_order() {
        let mut server = MockToolServer::new();
        server.call("list_team", pos(&[str_val("eng")])).unwrap();
        server.call("get_budget", pos(&[str_val("Q3")])).unwrap();
        let tools: Vec<&str> = server.trace().iter().map(|c| c.tool.as_str()).collect();
        assert_eq!(tools, vec!["list_team", "get_budget"]);
    }

    /// 통합: 인터프리터가 mock을 통해 DSL을 실행하면 인자 규약이 양쪽에서 맞물린다.
    #[test]
    fn interpreter_drives_mock_end_to_end() {
        let program =
            parse(tokenize("team = list_team(\"eng\")\nemit(team[0].name)").unwrap()).unwrap();
        let mut server = MockToolServer::new();
        let output = Interpreter::new(&mut server).run(&program).unwrap();
        assert_eq!(output, Some(str_val("Alice")));
        assert_eq!(server.trace().len(), 1);
    }
}
