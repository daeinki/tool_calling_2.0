//! RunRecord — 실행 한 건을 한 줄 JSON(JSONL)으로 남긴다 (M2-T04).
//!
//! 재현과 사후 분석을 위해 입력 조건(시드·프롬프트 버전·모델 ID·temperature)을
//! 빠짐없이 담는다. 분석 파이프라인은 이 JSONL을 읽어 통과율·절감 비율을 낸다.
//!
//! **경계(clean-code §7):** 직렬화는 여기(harness)의 관심사다. 코어 [`ptc_dsl::Value`]는
//! serde를 모른 채로 두고, [`value_to_json`]이 경계에서 자연스러운 JSON으로 옮긴다.

use ptc_dsl::Value;
use ptc_tools::ToolCall;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};

/// 실행 모드. 성능 비교의 두 축.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[serde(rename = "ptc")]
    Ptc,
    #[serde(rename = "baseline_1_0")]
    Baseline1_0,
}

/// 도구 호출 한 건의 기록(인자는 JSON으로 정규화).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool: String,
    pub args: serde_json::Value,
}

/// 채점 결과.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grade {
    pub level: String,
    pub pass: bool,
}

/// 성능 지표. LLM 호출 수가 1차 비교 지표다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub llm_calls: u32,
    pub tool_calls: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub latency_ms: u64,
}

/// 실행 한 건의 구조화된 로그.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub task_id: String,
    /// 난이도 계층(easy/medium/hard) — provider×tier 통과율 표 집계용(M3).
    pub tier: String,
    pub mode: Mode,
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
    pub seed: Option<u64>,
    pub temperature: f32,
    pub repeat_idx: u32,
    pub generated_code: String,
    /// `ok` | `fenced` | `extraction_fail`.
    pub extraction: String,
    /// `pass` | `reject:<reason>`.
    pub validation: String,
    pub tool_trace: Vec<ToolCallRecord>,
    pub final_output: Option<serde_json::Value>,
    pub grade: Option<Grade>,
    /// 실패 분류 라벨(통과면 `None`) — JSONL만으로 실패 분포를 집계하려 함(M3).
    pub failure: Option<String>,
    pub metrics: Metrics,
    pub error: Option<String>,
}

/// 코어 [`Value`]를 자연스러운 JSON으로 옮긴다(외부 태그 없이 1:1).
pub fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Num(n) => serde_json::Value::from(*n),
        Value::Str(s) => serde_json::Value::String(s.clone()),
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Null => serde_json::Value::Null,
        Value::List(items) => serde_json::Value::Array(items.iter().map(value_to_json).collect()),
        Value::Map(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), value_to_json(value)))
                .collect(),
        ),
    }
}

/// 자연스러운 JSON을 코어 [`Value`]로 되돌린다([`value_to_json`]의 역).
/// baseline ReAct가 도구 인자·관측·최종값을 텍스트(JSON)로 주고받을 때 쓴다(M4).
pub fn json_to_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => Value::Num(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::Str(s.clone()),
        serde_json::Value::Array(items) => Value::List(items.iter().map(json_to_value).collect()),
        serde_json::Value::Object(map) => Value::Map(
            map.iter()
                .map(|(key, value)| (key.clone(), json_to_value(value)))
                .collect(),
        ),
    }
}

/// 도구 트레이스를 기록용으로 정규화한다.
pub fn record_trace(trace: &[ToolCall]) -> Vec<ToolCallRecord> {
    trace
        .iter()
        .map(|call| ToolCallRecord {
            tool: call.tool.clone(),
            args: serde_json::Value::Object(
                call.args
                    .iter()
                    .map(|(key, value)| (key.clone(), value_to_json(value)))
                    .collect(),
            ),
        })
        .collect()
}

/// 레코드들을 한 줄에 하나씩 JSON으로 쓴다(JSONL).
pub fn write_jsonl<W: Write>(writer: &mut W, records: &[RunRecord]) -> io::Result<()> {
    for record in records {
        let line = serde_json::to_string(record).map_err(io::Error::other)?;
        writeln!(writer, "{line}")?;
    }
    Ok(())
}

/// 레코드들을 JSONL 문자열로 만든다.
pub fn to_jsonl(records: &[RunRecord]) -> String {
    let mut buffer = Vec::new();
    write_jsonl(&mut buffer, records).expect("writing to a Vec never fails");
    String::from_utf8(buffer).expect("serde_json emits valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn sample_record() -> RunRecord {
        RunRecord {
            run_id: "run-1".into(),
            task_id: "easy_01".into(),
            tier: "easy".into(),
            mode: Mode::Ptc,
            provider: "mock".into(),
            model: "mock-v0".into(),
            prompt_version: "sys-v1".into(),
            seed: Some(42),
            temperature: 0.0,
            repeat_idx: 0,
            generated_code: "emit(1)".into(),
            extraction: "fenced".into(),
            validation: "pass".into(),
            tool_trace: vec![ToolCallRecord {
                tool: "list_team".into(),
                args: serde_json::json!({ "arg0": "eng" }),
            }],
            final_output: Some(serde_json::json!(1.0)),
            grade: Some(Grade {
                level: "L1".into(),
                pass: true,
            }),
            failure: None,
            metrics: Metrics {
                llm_calls: 1,
                tool_calls: 1,
                input_tokens: 100,
                output_tokens: 5,
                latency_ms: 0,
            },
            error: None,
        }
    }

    #[test]
    fn record_round_trips_through_json() {
        let record = sample_record();
        let json = serde_json::to_string(&record).unwrap();
        let parsed: RunRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record, parsed);
    }

    #[test]
    fn mode_serializes_to_taxonomy_strings() {
        assert_eq!(serde_json::to_string(&Mode::Ptc).unwrap(), "\"ptc\"");
        assert_eq!(
            serde_json::to_string(&Mode::Baseline1_0).unwrap(),
            "\"baseline_1_0\""
        );
    }

    #[test]
    fn value_converts_to_natural_json() {
        let mut map = BTreeMap::new();
        map.insert("name".to_string(), Value::Str("Alice".into()));
        map.insert("id".to_string(), Value::Num(1.0));
        let value = Value::List(vec![Value::Map(map), Value::Bool(true), Value::Null]);

        assert_eq!(
            value_to_json(&value),
            serde_json::json!([{ "name": "Alice", "id": 1.0 }, true, null])
        );
    }

    #[test]
    fn jsonl_writes_one_object_per_line() {
        let records = vec![sample_record(), sample_record()];
        let jsonl = to_jsonl(&records);
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            // 각 줄이 독립적으로 파싱 가능한 객체여야 한다.
            let _: RunRecord = serde_json::from_str(line).unwrap();
        }
    }

    #[test]
    fn value_json_round_trips_both_ways() {
        let mut map = BTreeMap::new();
        map.insert("name".to_string(), Value::Str("Alice".into()));
        map.insert("id".to_string(), Value::Num(1.0));
        let value = Value::List(vec![Value::Map(map), Value::Bool(true), Value::Null]);
        assert_eq!(json_to_value(&value_to_json(&value)), value);
    }

    #[test]
    fn null_error_is_preserved_as_json_null() {
        let json = serde_json::to_value(sample_record()).unwrap();
        assert_eq!(json["error"], serde_json::Value::Null);
    }
}
