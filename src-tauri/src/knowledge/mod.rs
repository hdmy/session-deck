//! Local, deterministic knowledge extraction.
//!
//! This module deliberately has no storage or model dependency.  It turns a
//! normalized session into a small, inspectable knowledge card and a fixed
//! feature vector suitable for a local cosine scan.

use crate::domain::{FileChangeSummary, ParsedSession};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const VECTOR_DIM: usize = 256;
pub const MAX_CORPUS_CHARS: usize = 64 * 1024;
pub const MAX_CHANGE_SUMMARY_CHARS: usize = 8 * 1024;
pub const MAX_BODY_CHARS: usize = 8 * 1024;
const MAX_SEGMENT_CHARS: usize = 8 * 1024;
const MAX_TEXT_ITEMS: usize = 1024;
const MAX_CHANGE_ITEMS: usize = 256;
const MAX_TOOL_NAMES: usize = 256;
const MAX_TOOL_NAME_CHARS: usize = 256;
const MAX_SUMMARY_CHARS: usize = 320;
const MAX_SENTENCE_CHARS: usize = 180;
const MAX_ITEMS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct KnowledgeInput {
    pub session_id: String,
    pub provider_id: String,
    pub title: String,
    pub user_texts: Vec<String>,
    pub final_assistant_texts: Vec<String>,
    pub change_summaries: Vec<FileChangeSummary>,
    pub tool_names: Vec<String>,
}

impl KnowledgeInput {
    pub fn from_session(session: &ParsedSession) -> Self {
        let mut user_texts = Vec::new();
        let mut final_assistant_texts = Vec::new();
        let mut tool_names = Vec::new();
        let mut changes = Vec::new();
        for turn in &session.turns {
            if let Some(prompt) = turn
                .user_prompt
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                if user_texts.len() < MAX_TEXT_ITEMS {
                    user_texts.push(truncate(prompt, MAX_SEGMENT_CHARS));
                }
            }
            if let Some(response) = turn
                .final_response
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                if final_assistant_texts.len() < MAX_TEXT_ITEMS {
                    final_assistant_texts.push(truncate(response, MAX_SEGMENT_CHARS));
                }
            }
            for activity in &turn.activities {
                if activity.kind == "tool_use" {
                    if let Some(name) = activity
                        .tool_name
                        .as_deref()
                        .filter(|name| !name.trim().is_empty())
                    {
                        tool_names.push(truncate(name.trim(), MAX_TOOL_NAME_CHARS));
                    }
                }
            }
        }
        // ParsedBranch owns the provider-neutral change summaries; preserve
        // their fields exactly and do not infer paths from transcript text.
        for branch in &session.branches {
            for insight in &branch.turn_insights {
                let remaining = MAX_CHANGE_ITEMS.saturating_sub(changes.len());
                changes.extend(insight.file_changes.iter().take(remaining).cloned());
                if changes.len() == MAX_CHANGE_ITEMS {
                    break;
                }
            }
            if changes.len() == MAX_CHANGE_ITEMS {
                break;
            }
        }
        dedup_changes(&mut changes);
        Self {
            session_id: session.summary.id.clone(),
            provider_id: session.summary.provider_id.clone(),
            title: truncate(&session.summary.title, MAX_SEGMENT_CHARS),
            user_texts,
            final_assistant_texts,
            change_summaries: bounded_changes(&changes),
            tool_names: bounded_tool_names(&tool_names),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AutoKnowledge {
    pub summary: String,
    pub topics: Vec<String>,
    pub tags: Vec<String>,
    pub decisions: Vec<String>,
    pub troubleshooting: Vec<String>,
    pub change_summary: Vec<FileChangeSummary>,
    pub body_markdown: String,
    pub vector: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VectorError {
    WrongDimension { expected: usize, actual: usize },
    InvalidBlobLength { expected: usize, actual: usize },
    NonFiniteValue,
    ZeroVector,
    InvalidValue,
}

impl std::fmt::Display for VectorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongDimension { expected, actual } => {
                write!(
                    formatter,
                    "vector dimension must be {expected}, got {actual}"
                )
            }
            Self::InvalidBlobLength { expected, actual } => {
                write!(
                    formatter,
                    "vector blob must be {expected} bytes, got {actual}"
                )
            }
            Self::NonFiniteValue => write!(formatter, "vector contains a non-finite value"),
            Self::ZeroVector => write!(formatter, "zero vector has no cosine direction"),
            Self::InvalidValue => write!(formatter, "vector contains an invalid value"),
        }
    }
}

impl std::error::Error for VectorError {}

pub fn extract(input: &KnowledgeInput) -> AutoKnowledge {
    if input.user_texts.iter().all(|text| text.trim().is_empty())
        && input
            .final_assistant_texts
            .iter()
            .all(|text| text.trim().is_empty())
        && input.title.trim().is_empty()
        && input.change_summaries.is_empty()
        && input.tool_names.is_empty()
    {
        return AutoKnowledge::default();
    }
    let corpus = corpus(input);
    let topics = topics(&corpus);
    let tags = tags(&corpus);
    let decisions = marked_sentences(&corpus, DECISION_MARKERS);
    let troubleshooting = marked_sentences(&corpus, TROUBLESHOOTING_MARKERS);
    let summary = make_summary(input);
    let vector = feature_vector(input).ok();
    let change_summary = bounded_changes(&input.change_summaries);
    let body_markdown = make_body(
        &summary,
        &topics,
        &tags,
        &decisions,
        &troubleshooting,
        &change_summary,
    );
    AutoKnowledge {
        summary,
        topics,
        tags,
        decisions,
        troubleshooting,
        change_summary,
        body_markdown,
        vector,
    }
}

pub fn feature_vector(input: &KnowledgeInput) -> Result<Vec<f32>, VectorError> {
    let mut values = [0.0_f32; VECTOR_DIM];
    let mut saw_feature = false;
    for token in tokens(&corpus(input)) {
        saw_feature = true;
        add_hashed(&mut values, &token, 1.0);
    }
    for name in bounded_tool_names(&input.tool_names) {
        for token in tokens(&name) {
            saw_feature = true;
            add_hashed(&mut values, &format!("tool:{token}"), 1.0);
        }
    }
    if !saw_feature {
        return Err(VectorError::ZeroVector);
    }
    normalize(&values)
}

pub fn normalize(values: &[f32]) -> Result<Vec<f32>, VectorError> {
    validate_dimension(values)?;
    let mut norm = 0.0_f64;
    for value in values {
        if !value.is_finite() {
            return Err(VectorError::NonFiniteValue);
        }
        norm += f64::from(*value) * f64::from(*value);
    }
    if norm == 0.0 {
        return Err(VectorError::ZeroVector);
    }
    let norm = norm.sqrt() as f32;
    Ok(values.iter().map(|value| *value / norm).collect())
}

pub fn encode_vector(values: &[f32]) -> Result<Vec<u8>, VectorError> {
    validate_dimension(values)?;
    if values.iter().any(|value| !value.is_finite()) {
        return Err(VectorError::NonFiniteValue);
    }
    if values.iter().all(|value| *value == 0.0) {
        return Err(VectorError::ZeroVector);
    }
    let mut blob = Vec::with_capacity(VECTOR_DIM * std::mem::size_of::<f32>());
    for value in values {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    Ok(blob)
}

pub fn decode_vector(blob: &[u8]) -> Result<Vec<f32>, VectorError> {
    let expected = VECTOR_DIM * std::mem::size_of::<f32>();
    if blob.len() != expected {
        return Err(VectorError::InvalidBlobLength {
            expected,
            actual: blob.len(),
        });
    }
    let mut values = Vec::with_capacity(VECTOR_DIM);
    for bytes in blob.chunks_exact(4) {
        let value = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if !value.is_finite() {
            return Err(VectorError::NonFiniteValue);
        }
        values.push(value);
    }
    if values.iter().all(|value| *value == 0.0) {
        return Err(VectorError::ZeroVector);
    }
    Ok(values)
}

pub fn cosine(left: &[f32], right: &[f32]) -> Result<f32, VectorError> {
    validate_dimension(left)?;
    validate_dimension(right)?;
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (left, right) in left.iter().zip(right) {
        if !left.is_finite() || !right.is_finite() {
            return Err(VectorError::NonFiniteValue);
        }
        dot += f64::from(*left) * f64::from(*right);
        left_norm += f64::from(*left) * f64::from(*left);
        right_norm += f64::from(*right) * f64::from(*right);
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return Err(VectorError::ZeroVector);
    }
    Ok((dot / (left_norm.sqrt() * right_norm.sqrt())) as f32)
}

pub fn rank_similar(
    query: &[f32],
    candidates: &[(String, Vec<f32>)],
) -> Result<Vec<(String, f32)>, VectorError> {
    // ponytail: linear scan is enough for the local MVP; add an ANN index only
    // when candidate counts make measured search latency unacceptable.
    let mut ranked = candidates
        .iter()
        .map(|(id, vector)| Ok((id.clone(), cosine(query, vector)?)))
        .collect::<Result<Vec<_>, VectorError>>()?;
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
    Ok(ranked)
}

fn validate_dimension(values: &[f32]) -> Result<(), VectorError> {
    if values.len() != VECTOR_DIM {
        return Err(VectorError::WrongDimension {
            expected: VECTOR_DIM,
            actual: values.len(),
        });
    }
    Ok(())
}

fn corpus(input: &KnowledgeInput) -> String {
    let mut corpus = String::new();
    let mut used = 0;
    for segment in input
        .user_texts
        .iter()
        .chain(input.final_assistant_texts.iter())
        .chain(std::iter::once(&input.title))
    {
        if used >= MAX_CORPUS_CHARS {
            break;
        }
        if !corpus.is_empty() {
            corpus.push('\n');
            used += 1;
        }
        let bounded = truncate(
            segment,
            MAX_CORPUS_CHARS.saturating_sub(used).min(MAX_SEGMENT_CHARS),
        );
        used += bounded.chars().count();
        corpus.push_str(&bounded);
    }
    truncate(&corpus, MAX_CORPUS_CHARS)
}

fn make_summary(input: &KnowledgeInput) -> String {
    let problem = input
        .user_texts
        .iter()
        .find(|text| meaningful(text))
        .or_else(|| input.user_texts.iter().find(|text| !text.trim().is_empty()))
        .map(|text| truncate(text.trim(), MAX_SUMMARY_CHARS / 2))
        .unwrap_or_default();
    let response = input
        .final_assistant_texts
        .iter()
        .rev()
        .find(|text| !text.trim().is_empty())
        .map(|text| truncate(text.trim(), MAX_SUMMARY_CHARS / 2))
        .unwrap_or_default();
    match (problem.is_empty(), response.is_empty()) {
        (true, true) => truncate(input.title.trim(), MAX_SUMMARY_CHARS),
        (false, true) => problem,
        (true, false) => response,
        (false, false) => truncate(&format!("{problem} — {response}"), MAX_SUMMARY_CHARS),
    }
}

fn meaningful(text: &str) -> bool {
    let normalized = text.trim().to_ascii_lowercase();
    !normalized.is_empty()
        && !matches!(
            normalized.as_str(),
            "hi" | "hello" | "thanks" | "thank you" | "ok" | "okay"
        )
}

fn topics(text: &str) -> Vec<String> {
    let mut counts = HashMap::<String, usize>::new();
    for token in tokens(text) {
        *counts.entry(token).or_default() += 1;
    }
    let mut ranked = counts.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    ranked
        .into_iter()
        .take(MAX_ITEMS)
        .map(|(token, _)| token)
        .collect()
}

const TAG_RULES: &[(&str, &[&str])] = &[
    (
        "架构",
        &[
            "架构",
            "设计",
            "模块",
            "refactor",
            "interface",
            "architecture",
        ],
    ),
    (
        "排障",
        &[
            "bug", "error", "错误", "失败", "异常", "修复", "debug", "排查", "问题",
        ],
    ),
    (
        "测试",
        &["test", "测试", "spec", "验证", "coverage", "断言"],
    ),
    (
        "前端",
        &["vue", "react", "前端", "组件", "css", "ui", "页面"],
    ),
    ("后端", &["api", "服务", "后端", "server", "endpoint"]),
    (
        "数据库",
        &["sql", "sqlite", "数据库", "db", "migration", "索引"],
    ),
    (
        "性能",
        &["性能", "performance", "慢", "latency", "cache", "优化"],
    ),
    (
        "安全",
        &["安全", "security", "auth", "权限", "token", "secret"],
    ),
];

fn tags(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    TAG_RULES
        .iter()
        .filter(|(_, markers)| {
            markers.iter().any(|marker| {
                if marker.is_ascii() {
                    lower.contains(marker)
                } else {
                    text.contains(marker)
                }
            })
        })
        .map(|(tag, _)| (*tag).to_owned())
        .collect()
}

const DECISION_MARKERS: &[&str] = &[
    "决定", "采用", "选择", "decision", "decided", "we will", "改为", "prefer",
];
const TROUBLESHOOTING_MARKERS: &[&str] = &[
    "修复",
    "问题",
    "错误",
    "失败",
    "异常",
    "排查",
    "debug",
    "bug",
    "error",
    "root cause",
    "原因",
    "解决",
];

fn marked_sentences(text: &str, markers: &[&str]) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut output = Vec::new();
    let mut offset = 0;
    for sentence in text.split(|character| {
        matches!(
            character,
            '.' | '。' | '！' | '!' | '？' | '?' | '\n' | ';' | '；'
        )
    }) {
        let start = lower[offset..]
            .find(&sentence.to_ascii_lowercase())
            .unwrap_or(0)
            + offset;
        offset = start.saturating_add(sentence.len());
        let trimmed = sentence.trim();
        if trimmed.is_empty() {
            continue;
        }
        let sentence_lower = trimmed.to_ascii_lowercase();
        if markers.iter().any(|marker| {
            if marker.is_ascii() {
                sentence_lower.contains(marker)
            } else {
                trimmed.contains(marker)
            }
        }) {
            let item = truncate(trimmed, MAX_SENTENCE_CHARS);
            if !output.iter().any(|existing| existing == &item) {
                output.push(item);
            }
            if output.len() == MAX_ITEMS {
                break;
            }
        }
    }
    output
}

fn make_body(
    summary: &str,
    topics: &[String],
    tags: &[String],
    decisions: &[String],
    troubleshooting: &[String],
    changes: &[FileChangeSummary],
) -> String {
    let mut body = format!(
        "## Summary\n\n{summary}\n\n## Topics\n\n{}\n\n## Tags\n\n{}",
        topics.join(", "),
        tags.join(", ")
    );
    if !decisions.is_empty() {
        body.push_str("\n\n## Decisions\n\n");
        body.push_str(&bullets(decisions));
    }
    if !troubleshooting.is_empty() {
        body.push_str("\n\n## Troubleshooting\n\n");
        body.push_str(&bullets(troubleshooting));
    }
    if !changes.is_empty() {
        body.push_str("\n\n## Changes\n\n");
        for change in changes {
            body.push_str(&format!("- {} ({})\n", change.path, change.kind));
        }
    }
    truncate(&body, MAX_BODY_CHARS)
}

fn bullets(items: &[String]) -> String {
    items.iter().map(|item| format!("- {item}\n")).collect()
}

fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn tokens(text: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut ascii = String::new();
    let mut cjk = Vec::new();
    let flush_ascii = |output: &mut Vec<String>, ascii: &mut String| {
        if !ascii.is_empty() {
            if !STOPWORDS.contains(&ascii.as_str()) {
                output.push(std::mem::take(ascii));
            } else {
                ascii.clear();
            }
        }
    };
    let flush_cjk = |output: &mut Vec<String>, cjk: &mut Vec<char>| {
        for pair in cjk.windows(2) {
            let token = pair.iter().collect::<String>();
            if !STOPWORDS.contains(&token.as_str()) {
                output.push(token);
            }
        }
        cjk.clear();
    };
    for character in text.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            flush_cjk(&mut output, &mut cjk);
            ascii.push(character.to_ascii_lowercase());
        } else if is_cjk(character) {
            flush_ascii(&mut output, &mut ascii);
            cjk.push(character);
        } else {
            flush_ascii(&mut output, &mut ascii);
            flush_cjk(&mut output, &mut cjk);
        }
    }
    flush_ascii(&mut output, &mut ascii);
    flush_cjk(&mut output, &mut cjk);
    output
}

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "for", "in", "is", "it", "of", "on", "the", "this", "that", "to", "with",
    "请", "问题",
];

fn is_cjk(character: char) -> bool {
    matches!(character as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff)
}

fn add_hashed(values: &mut [f32; VECTOR_DIM], token: &str, weight: f32) {
    let hash = hash64(token.as_bytes());
    let index = (hash % VECTOR_DIM as u64) as usize;
    let sign = if hash & (1 << 63) == 0 { 1.0 } else { -1.0 };
    values[index] += sign * weight;
}

fn hash64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn bounded_tool_names(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for value in values {
        let value = truncate(value.trim(), MAX_TOOL_NAME_CHARS);
        if value.is_empty() || !seen.insert(value.clone()) {
            continue;
        }
        output.push(value);
        if output.len() == MAX_TOOL_NAMES {
            break;
        }
    }
    output
}

fn dedup_changes(values: &mut Vec<FileChangeSummary>) {
    let original = std::mem::take(values);
    let mut seen = HashSet::new();
    *values = original
        .into_iter()
        .filter(|value| {
            seen.insert((
                value.path.clone(),
                value.kind.clone(),
                value.turn_id,
                value.event_id,
            ))
        })
        .take(MAX_CHANGE_ITEMS)
        .collect();
}

pub fn bounded_changes(values: &[FileChangeSummary]) -> Vec<FileChangeSummary> {
    let mut output = Vec::new();
    let mut used = 0;
    for value in values {
        let cost = value.path.chars().count() + value.kind.chars().count() + 64;
        if cost > MAX_CHANGE_SUMMARY_CHARS {
            continue;
        }
        if used + cost > MAX_CHANGE_SUMMARY_CHARS || output.len() == MAX_CHANGE_ITEMS {
            break;
        }
        used += cost;
        output.push(value.clone());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(user: &str, response: &str) -> KnowledgeInput {
        KnowledgeInput {
            session_id: "s1".into(),
            provider_id: "test".into(),
            title: "修复数据库错误".into(),
            user_texts: vec![user.into()],
            final_assistant_texts: vec![response.into()],
            tool_names: vec!["cargo test".into()],
            ..Default::default()
        }
    }

    #[test]
    fn mixed_language_topics_tags_and_markers_are_transparent() {
        let knowledge = extract(&input(
            "请排查 SQLite 数据库 error",
            "决定采用 migration。修复问题并通过 test。",
        ));
        assert!(knowledge
            .topics
            .iter()
            .any(|topic| topic == "数据" || topic == "据库"));
        assert!(knowledge.tags.contains(&"数据库".into()));
        assert!(knowledge.tags.contains(&"排障".into()));
        assert!(knowledge.tags.contains(&"测试".into()));
        assert_eq!(knowledge.decisions.len(), 1);
        assert!(knowledge
            .troubleshooting
            .iter()
            .any(|item| item.contains("修复问题")));
    }

    #[test]
    fn unicode_truncation_does_not_split_emoji() {
        let input = input(&"🙂".repeat(400), "完成");
        let knowledge = extract(&input);
        assert!(knowledge.summary.chars().count() <= MAX_SUMMARY_CHARS);
        assert!(knowledge.summary.is_char_boundary(knowledge.summary.len()));
    }

    #[test]
    fn vector_is_stable_and_related_scores_higher() {
        let left = feature_vector(&input("修复 SQLite 数据库错误", "完成测试")).unwrap();
        let related = feature_vector(&input("SQLite 数据库错误排查", "测试通过")).unwrap();
        let unrelated = feature_vector(&input("烘焙巧克力蛋糕", "晚餐完成")).unwrap();
        assert_eq!(
            left,
            feature_vector(&input("修复 SQLite 数据库错误", "完成测试")).unwrap()
        );
        assert!(cosine(&left, &related).unwrap() > cosine(&left, &unrelated).unwrap());
    }

    #[test]
    fn blobs_reject_malformed_and_zero_values() {
        let vector = feature_vector(&input("hello", "world")).unwrap();
        let blob = encode_vector(&vector).unwrap();
        assert_eq!(decode_vector(&blob).unwrap(), vector);
        assert!(matches!(
            decode_vector(&blob[..3]),
            Err(VectorError::InvalidBlobLength { .. })
        ));
        assert!(matches!(
            encode_vector(&[0.0; VECTOR_DIM]),
            Err(VectorError::ZeroVector)
        ));
        assert!(matches!(
            feature_vector(&KnowledgeInput::default()),
            Err(VectorError::ZeroVector)
        ));
    }

    #[test]
    fn empty_input_is_empty_knowledge() {
        assert_eq!(
            extract(&KnowledgeInput::default()),
            AutoKnowledge::default()
        );
    }

    #[test]
    fn oversized_unicode_and_changes_are_bounded_and_stable() {
        let mut oversized = input(
            &format!("数据库🙂{}", "排障".repeat(20_000)),
            &"修复🙂".repeat(20_000),
        );
        oversized.tool_names = (0..10_000)
            .map(|index| format!("tool-{index}-{}", "🙂".repeat(200)))
            .collect();
        oversized.change_summaries = (0..10_000)
            .map(|index| FileChangeSummary {
                path: format!("src/超长文件-{index}.rs"),
                kind: "modified".into(),
                ..Default::default()
            })
            .collect();
        let first = extract(&oversized);
        let second = extract(&oversized);
        assert!(first.body_markdown.chars().count() <= MAX_BODY_CHARS);
        assert!(first.change_summary.len() <= MAX_CHANGE_ITEMS);
        assert!(
            first
                .change_summary
                .iter()
                .map(|change| change.path.chars().count() + change.kind.chars().count() + 64)
                .sum::<usize>()
                <= MAX_CHANGE_SUMMARY_CHARS
        );
        assert_eq!(first.vector, second.vector);
        assert_eq!(first.body_markdown, second.body_markdown);
        assert_eq!(first.vector.as_ref().map(Vec::len), Some(VECTOR_DIM));
    }
}
