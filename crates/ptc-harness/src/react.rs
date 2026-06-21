//! ReAct 액션 프로토콜 — baseline 1.0이 한 턴에 내는 행동의 표현 (M4-T01).
//!
//! PTC가 한 번에 프로그램을 내는 것과 달리, baseline은 매 턴 **하나의 행동**만 낸다:
//! - `CALL <tool> <json-args>` — 도구 하나를 부른다(결과를 보고 다음 턴으로).
//! - `FINAL <json-value>` — 최종 답을 낸다(루프 종료).
//!
//! 인자·값은 JSON으로 주고받으며, 코어 [`Value`]와의 변환은 harness 경계의
//! [`value_to_json`]/[`json_to_value`]를 재사용한다(직렬화는 한 곳에서 — §7).

use crate::record::{json_to_value, value_to_json};
use ptc_dsl::Value;
use std::collections::BTreeMap;

/// baseline이 한 턴에 내는 행동.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Call {
        tool: String,
        args: BTreeMap<String, Value>,
    },
    Final {
        value: Value,
    },
}

impl Action {
    /// 행동을 한 줄 텍스트로 렌더한다(mock 응답·로깅용).
    pub fn render(&self) -> String {
        match self {
            Action::Call { tool, args } => {
                let object: serde_json::Map<String, serde_json::Value> = args
                    .iter()
                    .map(|(key, value)| (key.clone(), value_to_json(value)))
                    .collect();
                format!("CALL {tool} {}", serde_json::Value::Object(object))
            }
            Action::Final { value } => format!("FINAL {}", value_to_json(value)),
        }
    }
}

/// 한 줄 텍스트를 행동으로 파싱한다. 형식 위반은 사람이 읽는 에러로 돌려준다.
pub fn parse_action(text: &str) -> Result<Action, String> {
    let line = text.trim();
    let (keyword, rest) = match line.split_once(char::is_whitespace) {
        Some((keyword, rest)) => (keyword, rest.trim()),
        // FINAL만 단독으로 와도(값 없음) null 최종값으로 허용.
        None => (line, ""),
    };
    match keyword {
        "CALL" => parse_call(rest),
        "FINAL" => parse_final(rest),
        other => Err(format!("알 수 없는 액션 키워드: {other:?}")),
    }
}

fn parse_call(rest: &str) -> Result<Action, String> {
    let (tool, args_json) = rest
        .split_once(char::is_whitespace)
        .ok_or_else(|| "CALL에 인자 JSON이 없음".to_string())?;
    let json: serde_json::Value = serde_json::from_str(args_json.trim())
        .map_err(|e| format!("CALL 인자 JSON 파싱 실패: {e}"))?;
    let object = json
        .as_object()
        .ok_or_else(|| "CALL 인자는 JSON 객체여야 함".to_string())?;
    let args = object
        .iter()
        .map(|(key, value)| (key.clone(), json_to_value(value)))
        .collect();
    Ok(Action::Call {
        tool: tool.to_string(),
        args,
    })
}

fn parse_final(rest: &str) -> Result<Action, String> {
    if rest.is_empty() {
        return Ok(Action::Final { value: Value::Null });
    }
    let json: serde_json::Value =
        serde_json::from_str(rest).map_err(|e| format!("FINAL 값 JSON 파싱 실패: {e}"))?;
    Ok(Action::Final {
        value: json_to_value(&json),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn call_round_trips_through_text() {
        let action = Action::Call {
            tool: "get_expenses".into(),
            args: args(&[("arg0", Value::Num(1.0)), ("arg1", Value::Str("Q3".into()))]),
        };
        assert_eq!(parse_action(&action.render()).unwrap(), action);
    }

    #[test]
    fn final_round_trips_for_scalar_and_string() {
        for value in [
            Value::Num(13000.0),
            Value::Str("Alice".into()),
            Value::Bool(true),
        ] {
            let action = Action::Final {
                value: value.clone(),
            };
            assert_eq!(parse_action(&action.render()).unwrap(), action);
        }
    }

    #[test]
    fn parses_a_hand_written_call() {
        let action = parse_action("CALL list_team {\"arg0\":\"eng\"}").unwrap();
        assert_eq!(
            action,
            Action::Call {
                tool: "list_team".into(),
                args: args(&[("arg0", Value::Str("eng".into()))]),
            }
        );
    }

    #[test]
    fn final_without_value_is_null() {
        assert_eq!(
            parse_action("FINAL").unwrap(),
            Action::Final { value: Value::Null }
        );
    }

    #[test]
    fn unknown_keyword_is_an_error() {
        assert!(parse_action("THINK hard").is_err());
    }

    #[test]
    fn call_without_args_json_is_an_error() {
        assert!(parse_action("CALL list_team").is_err());
    }

    #[test]
    fn call_with_non_object_args_is_an_error() {
        assert!(parse_action("CALL list_team [1,2]").is_err());
    }
}
