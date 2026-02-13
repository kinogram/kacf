use serde::Serialize;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::web_ui::RuntimeStatus;

#[derive(Debug, Serialize)]
pub(crate) struct CategoryCount {
    pub(crate) category: String,
    pub(crate) count: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct TimePoint {
    pub(crate) minute_ago: u32,
    pub(crate) success_rate: Option<f64>,
}

pub(crate) fn parse_perf_ms(line: &str, prefix: &str) -> Option<u32> {
    let tail = line.strip_prefix(prefix)?;
    let n = tail.strip_suffix("ms")?;
    n.trim().parse::<u32>().ok()
}

pub(crate) fn parse_eval_ms(line: &str) -> Option<u32> {
    if !line.starts_with("[Perf] eval_") {
        return None;
    }
    let idx = line.find('=')?;
    let tail = &line[idx + 1..];
    let n = tail.strip_suffix("ms")?;
    n.trim().parse::<u32>().ok()
}

pub(crate) fn push_sample(samples: &mut Vec<u32>, value: u32, max_len: usize) {
    samples.push(value);
    if samples.len() > max_len {
        let drop_n = samples.len() - max_len;
        samples.drain(0..drop_n);
    }
}

pub(crate) fn percentile_ms(samples: &[u32], p: usize) -> Option<u32> {
    if samples.is_empty() || p == 0 {
        return None;
    }
    let mut v = samples.to_vec();
    v.sort_unstable();
    let idx = ((v.len() - 1) * p.min(100)) / 100;
    v.get(idx).copied()
}

pub(crate) fn parse_eval_digest_category(line: &str) -> Option<String> {
    let prefix = "[Eval-Digest] category=";
    let tail = line.strip_prefix(prefix)?;
    let category = tail.split_whitespace().next()?.trim();
    if category.is_empty() {
        return None;
    }
    Some(category.to_string())
}

pub(crate) fn parse_eval_digest_signature(line: &str) -> Option<String> {
    let key = " signature=";
    let idx = line.find(key)?;
    let sig = line[idx + key.len()..].trim();
    if sig.is_empty() {
        return None;
    }
    Some(sig.chars().take(120).collect::<String>())
}

pub(crate) fn push_digest(
    history: &mut Vec<(u64, String)>,
    ts: u64,
    category: String,
    max_len: usize,
) {
    history.push((ts, category));
    if history.len() > max_len {
        let drop_n = history.len() - max_len;
        history.drain(0..drop_n);
    }
}

pub(crate) fn digest_recent_counts(
    history: &[(u64, String)],
    window_secs: u64,
) -> Vec<CategoryCount> {
    let now = now_unix();
    let mut map: BTreeMap<String, u64> = BTreeMap::new();
    for (ts, cat) in history {
        if now.saturating_sub(*ts) <= window_secs {
            *map.entry(cat.clone()).or_insert(0) += 1;
        }
    }
    let mut out = map
        .into_iter()
        .map(|(category, count)| CategoryCount { category, count })
        .collect::<Vec<_>>();
    out.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.category.cmp(&b.category))
    });
    out.truncate(8);
    out
}

pub(crate) fn push_done_history(history: &mut Vec<(u64, bool)>, ts: u64, ok: bool, max_len: usize) {
    history.push((ts, ok));
    if history.len() > max_len {
        let drop_n = history.len() - max_len;
        history.drain(0..drop_n);
    }
}

pub(crate) fn done_recent_counts(history: &[(u64, bool)], window_secs: u64) -> (u64, u64) {
    let now = now_unix();
    let mut ok = 0u64;
    let mut fail = 0u64;
    for (ts, is_ok) in history.iter().copied() {
        if now.saturating_sub(ts) <= window_secs {
            if is_ok {
                ok += 1;
            } else {
                fail += 1;
            }
        }
    }
    (ok, fail)
}

pub(crate) fn readiness_level(
    runtime: &RuntimeStatus,
    done_5m_ok: u64,
    done_5m_fail: u64,
) -> &'static str {
    if runtime.running {
        return "running";
    }
    if done_5m_fail > 0 {
        return "red";
    }
    if done_5m_ok > 0 {
        return "green";
    }
    if runtime.total_done_fail > runtime.total_done_ok {
        return "yellow";
    }
    "idle"
}

pub(crate) fn readiness_score(
    runtime: &RuntimeStatus,
    done_5m_ok: u64,
    done_5m_fail: u64,
    done_5m_success_rate: Option<f64>,
    api_p95_ms: Option<u32>,
    eval_p95_ms: Option<u32>,
) -> u8 {
    let mut score: i32 = 100;
    if runtime.running {
        score -= 10;
    }
    if done_5m_fail > 0 {
        score -= 30;
    }
    if let Some(rate) = done_5m_success_rate {
        if rate < 60.0 {
            score -= 25;
        } else if rate < 85.0 {
            score -= 10;
        }
    }
    if done_5m_ok == 0 && done_5m_fail == 0 {
        score -= 5;
    }
    if let Some(v) = api_p95_ms {
        if v > 60_000 {
            score -= 10;
        }
    }
    if let Some(v) = eval_p95_ms {
        if v > 120_000 {
            score -= 15;
        }
    }
    if !runtime.last_error.trim().is_empty() {
        score -= 5;
    }
    score.clamp(0, 100) as u8
}

pub(crate) fn release_blockers(
    runtime: &RuntimeStatus,
    done_5m_fail: u64,
    done_5m_success_rate: Option<f64>,
    eval_p95_ms: Option<u32>,
) -> Vec<String> {
    let mut out = Vec::new();
    if done_5m_fail > 0 {
        out.push("最近5分钟存在失败结果".to_string());
    }
    if let Some(rate) = done_5m_success_rate {
        if rate < 80.0 {
            out.push(format!("最近5分钟成功率偏低: {:.1}%", rate));
        }
    }
    if let Some(v) = eval_p95_ms {
        if v > 120_000 {
            out.push(format!("评测耗时 p95 过高: {}ms", v));
        }
    }
    if runtime.last_error.to_lowercase().contains("session error") {
        out.push("最近会话出现 Session error".to_string());
    }
    out
}

pub(crate) fn release_actions(
    runtime: &RuntimeStatus,
    api_p95_ms: Option<u32>,
    eval_p95_ms: Option<u32>,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(v) = api_p95_ms {
        if v > 60_000 {
            out.push("检查网络质量，或提高 AUTOCODING_API_TIMEOUT_SECS".to_string());
        }
    }
    if let Some(v) = eval_p95_ms {
        if v > 120_000 {
            out.push("检查评测脚本性能，必要时提高 AUTOCODING_EVAL_TIMEOUT_SECS".to_string());
        }
    }
    if !runtime.last_error.trim().is_empty() {
        out.push("根据 last_error 优先修复根因，再继续迭代".to_string());
    }
    if out.is_empty() {
        out.push("当前无明显阻断，可进行灰度发布".to_string());
    }
    out
}

pub(crate) fn success_rate_series(
    history: &[(u64, bool)],
    minutes: u32,
    bucket_secs: u64,
) -> Vec<TimePoint> {
    let now = now_unix();
    let mut out = Vec::new();
    for i in (0..minutes).rev() {
        let start = now.saturating_sub((i as u64 + 1) * bucket_secs);
        let end = now.saturating_sub((i as u64) * bucket_secs);
        let mut ok = 0u64;
        let mut total = 0u64;
        for (ts, is_ok) in history.iter().copied() {
            if ts >= start && ts < end {
                total += 1;
                if is_ok {
                    ok += 1;
                }
            }
        }
        let success_rate = if total == 0 {
            None
        } else {
            Some((ok as f64) * 100.0 / (total as f64))
        };
        out.push(TimePoint {
            minute_ago: i + 1,
            success_rate,
        });
    }
    out
}

pub(crate) fn release_gate(
    readiness_score: u8,
    gate_threshold: u8,
    blockers: &[String],
    running: bool,
) -> (bool, String) {
    if running {
        return (false, "任务仍在运行中".to_string());
    }
    if !blockers.is_empty() {
        return (false, blockers.join("；"));
    }
    if readiness_score < gate_threshold {
        return (
            false,
            format!(
                "readiness_score={} 低于阈值 {}",
                readiness_score, gate_threshold
            ),
        );
    }
    (true, "通过发布门禁".to_string())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
