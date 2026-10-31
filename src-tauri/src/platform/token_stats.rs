//! 本地 WorkBuddy / WorkBuddy 国际版 / CodeBuddy IDE Token 统计。
//!
//! 设计参考 workbuddy-switch 的 token_stats 模块：日志只在这里解码、去重和按时间
//! 聚合，Tauri 层只负责转发结果。响应只包含聚合数字和脱敏标识，不返回消息正文、
//! arguments 或认证信息。
//!
//! CodeBuddy IDE 不写 JSONL，用量在 `CodeBuddyExtension/Data/**/history/**/index.json`
//! 的 `requests[].usage` 中；消息正文文件（`messages/`）不会被扫描。
//!
//! WorkBuddy 与 WorkBuddy 国际版**分开统计**（`workbuddy` / `workbuddy-ai` 两个 source），
//! 不合并：两档位是不同账号体系，混算会让用量无法归因。

use chrono::{Datelike, Local, Timelike};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Usage {
    input: u64,
    output: u64,
    read: u64,
    write: u64,
}

#[derive(Clone, Debug, Default)]
struct Totals {
    usage: Usage,
    records: u64,
}

#[derive(Clone, Debug)]
struct SessionTotals {
    key: String,
    title: Option<String>,
    project: String,
    session_id: String,
    totals: Totals,
}

/// 展示总量：`input` 已包含 cache reads，因此只追加 output 与 cache write，
/// 避免把缓存命中重复计入。
fn usage_total(usage: Usage) -> u64 {
    usage
        .input
        .saturating_add(usage.output)
        .saturating_add(usage.write)
}

impl Totals {
    fn add(&mut self, usage: Usage) {
        self.usage.input = self.usage.input.saturating_add(usage.input);
        self.usage.output = self.usage.output.saturating_add(usage.output);
        self.usage.read = self.usage.read.saturating_add(usage.read);
        self.usage.write = self.usage.write.saturating_add(usage.write);
        self.records = self.records.saturating_add(1);
    }

    fn value(&self) -> Value {
        let cache_hit_rate =
            (self.usage.input > 0).then(|| self.usage.read as f64 / self.usage.input as f64);
        let total = usage_total(self.usage);
        json!({
            "total": total,
            "input": self.usage.input,
            "output": self.usage.output,
            "cacheRead": self.usage.read,
            "cacheWrite": self.usage.write,
            "uncachedInput": self.usage.input.saturating_sub(self.usage.read),
            "records": self.records,
            "cacheHitRate": cache_hit_rate,
        })
    }
}

// ---------- 字段解码 ----------

/// 从 JSON number 或 string 读取非负整数。
fn number(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| {
            value
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .map(|n| n as u64)
        })
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
}

fn field(object: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(number))
}

fn positive_field(object: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(number).filter(|value| *value > 0))
}

fn cached_input_field(object: &Map<String, Value>) -> u64 {
    positive_field(
        object,
        &[
            "cache_read_input_tokens",
            "cacheReadInputTokens",
            "prompt_cache_hit_tokens",
            "cached_tokens",
        ],
    )
    .or_else(|| {
        object
            .get("prompt_tokens_details")
            .and_then(Value::as_object)
            .and_then(|details| positive_field(details, &["cached_tokens"]))
    })
    .or_else(|| {
        object
            .get("inputTokensDetails")
            .and_then(Value::as_array)
            .and_then(|details| {
                details.iter().find_map(|detail| {
                    detail
                        .as_object()
                        .and_then(|detail| positive_field(detail, &["cached_tokens"]))
                })
            })
    })
    .unwrap_or(0)
}

const CACHE_WRITE_KEYS: &[&str] = &[
    "cache_write_input_tokens",
    "cacheWriteInputTokens",
    "cache_creation_input_tokens",
    "prompt_cache_write_tokens",
];

fn usage_fields(object: &Map<String, Value>) -> Usage {
    Usage {
        input: field(object, &["input_tokens", "inputTokens", "prompt_tokens"]).unwrap_or(0),
        output: field(
            object,
            &["output_tokens", "outputTokens", "completion_tokens"],
        )
        .unwrap_or(0),
        read: cached_input_field(object),
        write: positive_field(object, CACHE_WRITE_KEYS).unwrap_or(0),
    }
}

fn usage_object(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value?.as_object().filter(|object| {
        field(object, &["input_tokens", "inputTokens", "prompt_tokens"]).is_some()
    })
}

/// 解码一条记录。Usage 优先级：message.usage > providerData.usage > 顶层 usage。
fn usage(value: &Value) -> Option<Usage> {
    let provider = value.get("providerData");
    let candidates = [
        value
            .get("message")
            .and_then(|message| message.get("usage")),
        provider.and_then(|data| data.get("usage")),
        value.get("usage"),
    ];
    let selected = candidates.iter().copied().find_map(usage_object)?;
    let mut result = usage_fields(selected);

    if result.write == 0 {
        result.write = candidates
            .iter()
            .copied()
            .filter_map(|candidate| candidate.and_then(Value::as_object))
            .chain(
                provider
                    .and_then(|data| data.get("rawUsage"))
                    .and_then(Value::as_object),
            )
            .find_map(|object| positive_field(object, CACHE_WRITE_KEYS))
            .unwrap_or(0);
    }

    Some(result)
}

fn timestamp(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .or_else(|| value.get("ts"))
        .and_then(|timestamp| {
            timestamp
                .as_i64()
                .or_else(|| timestamp.as_u64().and_then(|n| i64::try_from(n).ok()))
                .or_else(|| timestamp.as_str()?.trim().parse::<i64>().ok())
        })
}

fn date(value: &Value) -> Option<String> {
    let timestamp = timestamp(value)?;
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|date| date.with_timezone(&Local).format("%Y-%m-%d").to_string())
}

fn hour(value: &Value) -> Option<String> {
    let timestamp = timestamp(value)?;
    chrono::DateTime::from_timestamp_millis(timestamp).map(|date| {
        let local = date.with_timezone(&Local);
        format!(
            "{}-{}",
            local.weekday().num_days_from_monday(),
            local.hour()
        )
    })
}

fn model(value: &Value) -> String {
    value
        .get("providerData")
        .and_then(|data| data.get("model"))
        .or_else(|| value.get("model"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or("未知模型")
        .to_string()
}

// ---------- 文件遍历 ----------

fn files(root: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|name| name.to_str()) != Some("subagents") {
                files(&path, output);
            }
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl") {
            output.push(path);
        }
    }
}

fn project_name(root: &Path, file: &Path) -> String {
    let name = file
        .strip_prefix(root)
        .ok()
        .and_then(|relative| relative.components().next())
        .and_then(|component| component.as_os_str().to_str())
        .filter(|name| !name.is_empty() && !name.ends_with(".jsonl"));
    match name {
        Some(name) if !name.starts_with("Users-") && !name.starts_with("home-") => name.to_string(),
        _ => "未知项目".to_string(),
    }
}

fn record_project(value: &Value, fallback: &str) -> String {
    value
        .get("cwd")
        .and_then(Value::as_str)
        .map(Path::new)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= 120)
        .unwrap_or(fallback)
        .to_string()
}

fn non_empty_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

// ---------- 聚合输出 ----------

fn groups(groups: HashMap<String, Totals>) -> Vec<Value> {
    let mut values: Vec<_> = groups
        .into_iter()
        .map(|(key, totals)| {
            let mut value = totals.value();
            value["key"] = json!(key);
            value
        })
        .collect();
    values.sort_by_key(|right| std::cmp::Reverse(total_value(right)));
    values
}

fn session_groups(sessions: Vec<SessionTotals>) -> Vec<Value> {
    let mut values: Vec<_> = sessions
        .into_iter()
        .map(|session| {
            let mut value = session.totals.value();
            value["key"] = json!(session.key);
            value["title"] = json!(session.title);
            value["project"] = json!(session.project);
            value["sessionId"] = json!(session.session_id);
            value
        })
        .collect();
    values.sort_by_key(|right| std::cmp::Reverse(total_value(right)));
    values
}

fn total_value(value: &Value) -> u64 {
    value
        .get("total")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| {
            value
                .get("input")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .saturating_add(value.get("output").and_then(Value::as_u64).unwrap_or(0))
                .saturating_add(value.get("cacheWrite").and_then(Value::as_u64).unwrap_or(0))
        })
}

// ---------- 聚合器 ----------

#[derive(Default)]
struct SourceCollector {
    total: Totals,
    models: HashMap<String, Totals>,
    projects: HashMap<String, Totals>,
    sessions: Vec<SessionTotals>,
    daily: HashMap<String, Totals>,
    daily_by_model: HashMap<String, HashMap<String, Totals>>,
    hours: HashMap<String, Totals>,
    parse_errors: u64,
    coverage_start_at: Option<i64>,
    coverage_end_at: Option<i64>,
    session_key_counts: HashMap<String, usize>,
}

impl SourceCollector {
    fn note_parse_error(&mut self) {
        self.parse_errors = self.parse_errors.saturating_add(1);
    }

    fn add(&mut self, usage: Usage, value: &Value, project: &str) {
        self.total.add(usage);
        let model_name = model(value);
        self.models
            .entry(model_name.clone())
            .or_default()
            .add(usage);
        self.projects
            .entry(project.to_string())
            .or_default()
            .add(usage);
        if let Some(day) = date(value) {
            self.daily.entry(day.clone()).or_default().add(usage);
            self.daily_by_model
                .entry(model_name)
                .or_default()
                .entry(day)
                .or_default()
                .add(usage);
        }
        if let Some(hour) = hour(value) {
            self.hours.entry(hour).or_default().add(usage);
        }
        if let Some(timestamp) = timestamp(value) {
            self.coverage_start_at = Some(
                self.coverage_start_at
                    .map_or(timestamp, |current| current.min(timestamp)),
            );
            self.coverage_end_at = Some(
                self.coverage_end_at
                    .map_or(timestamp, |current| current.max(timestamp)),
            );
        }
    }

    fn push_session(
        &mut self,
        session_id: String,
        title: Option<String>,
        project: String,
        totals: Totals,
    ) {
        if totals.records == 0 {
            return;
        }
        let base_key = format!("{project} · {session_id}");
        let count = self.session_key_counts.entry(base_key.clone()).or_default();
        *count += 1;
        let key = if *count == 1 {
            base_key
        } else {
            format!("{base_key} · {}", *count)
        };
        self.sessions.push(SessionTotals {
            key,
            title,
            project,
            session_id,
            totals,
        });
    }

    fn into_value(self, name: &str, files_scanned: usize) -> Value {
        let daily_by_model = self
            .daily_by_model
            .into_iter()
            .map(|(model, points)| (model, Value::Array(groups(points))))
            .collect::<Map<String, Value>>();
        let value = json!({
            "source": name,
            "summary": self.total.value(),
            "models": groups(self.models),
            "projects": groups(self.projects),
            "sessions": session_groups(self.sessions),
            "daily": groups(self.daily),
            "dailyByModel": daily_by_model,
            "hours": groups(self.hours),
            "filesScanned": files_scanned,
            "parseErrors": self.parse_errors,
            "coverageStartAt": self.coverage_start_at,
            "coverageEndAt": self.coverage_end_at,
        });
        value
    }
}

// ---------- JSONL 数据源扫描 ----------

fn source_from_roots(roots: &[PathBuf], name: &str, cutoff: Option<i64>) -> Value {
    let mut paths: Vec<(PathBuf, PathBuf)> = Vec::new();
    for root in roots {
        let mut found = Vec::new();
        files(root, &mut found);
        paths.extend(found.into_iter().map(|file| (root.clone(), file)));
    }
    let mut collector = SourceCollector::default();

    paths.sort_by_key(|(_, path)| {
        std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(u64::MAX)
    });
    let mut seen: HashSet<(i64, u64, u64, u64, u64, String)> = HashSet::new();

    for (root, path) in &paths {
        let session_id = path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("未知会话")
            .to_string();
        let fallback_project = project_name(root, path);
        let Ok(file) = std::fs::File::open(path) else {
            collector.note_parse_error();
            continue;
        };

        let mut session_totals = Totals::default();
        let mut session_project: Option<String> = None;
        let mut ai_title: Option<String> = None;
        let mut summary: Option<String> = None;

        for line in BufReader::new(file).lines() {
            let Ok(line) = line else {
                collector.note_parse_error();
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                collector.note_parse_error();
                continue;
            };
            if let Some(title) = non_empty_text(value.get("aiTitle")) {
                ai_title = Some(title);
            }
            if let Some(value) = non_empty_text(value.get("summary")) {
                summary = Some(value);
            }
            if cutoff.is_some_and(|minimum| timestamp(&value).is_none_or(|ts| ts < minimum)) {
                continue;
            }
            let Some(usage) = usage(&value) else {
                continue;
            };
            let ts = timestamp(&value);
            let model_name = model(&value);
            let duplicate = ts.is_some_and(|ts| {
                !seen.insert((
                    ts,
                    usage.input,
                    usage.output,
                    usage.read,
                    usage.write,
                    model_name.clone(),
                ))
            });
            if duplicate {
                continue;
            }
            let project = record_project(&value, &fallback_project);
            if session_project.is_none() {
                session_project = Some(project.clone());
            }
            session_totals.add(usage);
            collector.add(usage, &value, &project);
        }

        let title = ai_title.or(summary);
        collector.push_session(
            session_id,
            title,
            session_project.unwrap_or(fallback_project),
            session_totals,
        );
    }

    collector.into_value(name, paths.len())
}

// ---------- 数据根 ----------

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// WorkBuddy jsonl 根。
fn workbuddy_cn_roots() -> Vec<PathBuf> {
    let root = home().join(".workbuddy").join("projects");
    if root.is_dir() {
        vec![root]
    } else {
        vec![]
    }
}

/// 国际版 jsonl 根（projects 和 sessions 都探测）。
fn workbuddy_ai_roots() -> Vec<PathBuf> {
    let base = home().join(".workbuddy-ai");
    let mut roots = Vec::new();
    let projects = base.join("projects");
    if projects.is_dir() {
        roots.push(projects);
    }
    let sessions = base.join("sessions");
    if sessions.is_dir() {
        roots.push(sessions);
    }
    roots
}

fn codebuddy_extension_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("CodeBuddyExtension")
        .join("Data")
}

// ---------- CodeBuddy IDE 数据源 ----------

fn is_ide_conversation_index(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("index.json")
        && path
            .parent()
            .and_then(|parent| parent.parent())
            .and_then(|parent| parent.parent())
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            == Some("history")
}

fn ide_index_files(root: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|name| name.to_str());
            if !matches!(
                name,
                Some("messages" | "check-point" | "backups" | "Public")
            ) {
                ide_index_files(&path, output);
            }
        } else if is_ide_conversation_index(&path) {
            output.push(path);
        }
    }
}

fn decode_genie_workspace(name: &str) -> Option<String> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    let try_decode = |value: &str| -> Option<String> {
        let padded = match value.len() % 4 {
            0 => value.to_string(),
            remainder => format!("{value}{}", "=".repeat(4 - remainder)),
        };
        let bytes = engine.decode(padded).ok()?;
        let text = String::from_utf8(bytes).ok()?;
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.contains('\0') {
            return None;
        }
        Some(trimmed.to_string())
    };
    try_decode(name)
        .or_else(|| try_decode(&name.replace('_', "/")))
        .or_else(|| try_decode(&name.replace('_', "+")))
}

/// 从 CodeBuddy CN 的 genie-history 建立 session_id -> project 映射。
fn ide_project_by_session() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(root) = crate::platform::codebuddy_cn::data_dir().map(|dir| {
        dir.join("User")
            .join("globalStorage")
            .join("tencent-cloud.coding-copilot")
            .join("genie-history")
    }) else {
        return map;
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return map;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let folder = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let project = decode_genie_workspace(folder)
            .as_deref()
            .map(Path::new)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.len() <= 120)
            .unwrap_or("未知项目")
            .to_string();
        let Ok(conversations) = std::fs::read_dir(path.join("conversations")) else {
            continue;
        };
        for conversation in conversations.flatten() {
            if let Some(id) = conversation.file_name().to_str() {
                if !id.is_empty() {
                    map.insert(id.to_string(), project.clone());
                }
            }
        }
    }
    map
}

fn ide_workspace_meta(conv_index: &Path, conv_id: &str) -> (Option<String>, String) {
    let Some(ws_index) = conv_index
        .parent()
        .and_then(|parent| parent.parent())
        .map(|parent| parent.join("index.json"))
    else {
        return (None, "未知模型".to_string());
    };
    let Ok(text) = std::fs::read_to_string(ws_index) else {
        return (None, "未知模型".to_string());
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return (None, "未知模型".to_string());
    };
    let Some(conversations) = value.get("conversations").and_then(Value::as_array) else {
        return (None, "未知模型".to_string());
    };
    for conversation in conversations {
        if conversation.get("id").and_then(Value::as_str) != Some(conv_id) {
            continue;
        }
        let title = non_empty_text(conversation.get("name"))
            .or_else(|| non_empty_text(conversation.get("title")));
        let model = non_empty_text(conversation.get("selectedModelId"))
            .or_else(|| non_empty_text(conversation.get("modelId")))
            .or_else(|| non_empty_text(conversation.get("model")))
            .unwrap_or_else(|| "未知模型".to_string());
        return (title, model);
    }
    (None, "未知模型".to_string())
}

fn ide_request_usage(request: &Value) -> Option<Usage> {
    let object = request.get("usage")?.as_object()?;
    field(object, &["inputTokens", "input_tokens", "prompt_tokens"])?;
    Some(Usage {
        input: field(object, &["inputTokens", "input_tokens", "prompt_tokens"]).unwrap_or(0),
        output: field(
            object,
            &["outputTokens", "output_tokens", "completion_tokens"],
        )
        .unwrap_or(0),
        read: field(
            object,
            &[
                "cacheTokens",
                "cacheReadInputTokens",
                "cache_read_input_tokens",
            ],
        )
        .unwrap_or(0),
        write: positive_field(
            object,
            &[
                "cachedWriteTokens",
                "cacheWriteInputTokens",
                "cache_write_input_tokens",
                "cache_creation_input_tokens",
            ],
        )
        .unwrap_or(0),
    })
}

fn ide_request_timestamp(request: &Value) -> Option<i64> {
    timestamp(request).or_else(|| {
        request.get("startedAt").and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|n| i64::try_from(n).ok()))
                .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
        })
    })
}

fn ide_request_fingerprint(
    request: &Value,
    usage: &Usage,
    timestamp: Option<i64>,
) -> Option<String> {
    Some(match timestamp {
        Some(timestamp) => format!(
            "{timestamp}:{}:{}:{}:{}",
            usage.input, usage.output, usage.read, usage.write
        ),
        None => non_empty_text(request.get("id"))?.to_string(),
    })
}

fn ide_session_id(path: &Path) -> String {
    path.parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("未知会话")
        .to_string()
}

fn ide_file_weight(path: &Path) -> Option<(String, usize, u64)> {
    let text = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&text).ok()?;
    let requests = value.get("requests")?.as_array()?;
    let mut key: Option<String> = None;
    let mut count = 0usize;
    let mut total = 0u64;
    for request in requests {
        let Some(usage) = ide_request_usage(request) else {
            continue;
        };
        if key.is_none() {
            key = ide_request_fingerprint(request, &usage, ide_request_timestamp(request));
        }
        count += 1;
        total = total.saturating_add(usage_total(usage));
    }
    Some((key?, count, total))
}

fn collect_ide_requests(
    collector: &mut SourceCollector,
    requests: &[Value],
    cutoff: Option<i64>,
    model_name: &str,
    project: &str,
    seen: &mut HashSet<String>,
    session_totals: &mut Totals,
) {
    for request in requests {
        let ts = ide_request_timestamp(request);
        if cutoff.is_some_and(|minimum| ts.is_none_or(|value| value < minimum)) {
            continue;
        }
        let Some(usage) = ide_request_usage(request) else {
            continue;
        };
        if let Some(fingerprint) = ide_request_fingerprint(request, &usage, ts) {
            if !seen.insert(fingerprint) {
                continue;
            }
        }
        session_totals.add(usage);
        collector.add(
            usage,
            &json!({
                "timestamp": ts,
                "providerData": { "model": model_name },
            }),
            project,
        );
    }
}

fn ide_source(
    root: PathBuf,
    name: &str,
    cutoff: Option<i64>,
    project_by_session: &HashMap<String, String>,
) -> Value {
    let mut paths = Vec::new();
    ide_index_files(&root, &mut paths);
    paths.sort();

    let weights: Vec<Option<(String, usize, u64)>> =
        paths.iter().map(|path| ide_file_weight(path)).collect();
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut lead: HashMap<&str, (usize, usize, u64)> = HashMap::new();
    for (index, weight) in weights.iter().enumerate() {
        let Some((fingerprint, count, total)) = weight else {
            continue;
        };
        groups.entry(fingerprint.as_str()).or_default().push(index);
        let replace = lead
            .get(fingerprint.as_str())
            .is_none_or(|current| (*count, *total) > (current.1, current.2));
        if replace {
            lead.insert(fingerprint.as_str(), (index, *count, *total));
        }
    }

    let mut collector = SourceCollector::default();

    for (index, _) in paths.iter().enumerate() {
        let fingerprint = weights[index]
            .as_ref()
            .map(|(fingerprint, _, _)| fingerprint.as_str());
        if let Some(fingerprint) = fingerprint {
            if groups.get(fingerprint).and_then(|members| members.first()) != Some(&index) {
                continue;
            }
        }
        let lead_index = fingerprint
            .and_then(|fingerprint| lead.get(fingerprint).map(|current| current.0))
            .unwrap_or(index);
        let lead_session_id = ide_session_id(&paths[lead_index]);
        let (title, model_name) = ide_workspace_meta(&paths[lead_index], &lead_session_id);
        let project = project_by_session
            .get(&lead_session_id)
            .cloned()
            .unwrap_or_else(|| "未知项目".to_string());
        let members: &[usize] = match fingerprint {
            Some(fingerprint) => groups
                .get(fingerprint)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            None => std::slice::from_ref(&index),
        };
        let mut session_totals = Totals::default();
        let mut seen: HashSet<String> = HashSet::new();
        for member in members {
            let Ok(text) = std::fs::read_to_string(&paths[*member]) else {
                collector.note_parse_error();
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                collector.note_parse_error();
                continue;
            };
            let Some(requests) = value.get("requests").and_then(Value::as_array) else {
                continue;
            };
            collect_ide_requests(
                &mut collector,
                requests,
                cutoff,
                &model_name,
                &project,
                &mut seen,
                &mut session_totals,
            );
        }

        collector.push_session(lead_session_id, title, project, session_totals);
    }

    collector.into_value(name, paths.len())
}

/// 返回四个独立 source 的聚合统计。`days` 只接受 7/30/90，其它值视为全量。
pub fn get_statistics(days: Option<i64>) -> Value {
    let generated_at = now_ms();
    let range_days = match days {
        Some(7) => Some(7),
        Some(30) => Some(30),
        Some(90) => Some(90),
        _ => None,
    };
    let cutoff = range_days.map(|value| generated_at - value * 86_400_000);
    let ide_projects = ide_project_by_session();
    json!({
        "generatedAt": generated_at,
        "rangeDays": range_days,
        "sources": [
            source_from_roots(&workbuddy_cn_roots(), "workbuddy", cutoff),
            source_from_roots(&workbuddy_ai_roots(), "workbuddy-ai", cutoff),
            ide_source(
                codebuddy_extension_data_dir(),
                "codebuddy-ide",
                cutoff,
                &ide_projects,
            ),
        ],
    })
}
