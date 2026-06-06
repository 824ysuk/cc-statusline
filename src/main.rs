use std::io::{self, Read};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use serde::Deserialize;
use serde_json::Value;

// ── ANSI escape codes ─────────────────────────────────────────────────────────
const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const MAGENTA: &str = "\x1b[35m";
const CYAN_BRIGHT: &str = "\x1b[96m";
const GREEN: &str = "\x1b[32m";
const ORANGE: &str = "\x1b[38;5;208m";
const RED: &str = "\x1b[31m";
const BLUE_BRIGHT: &str = "\x1b[94m";

// ── stdin JSON schema ─────────────────────────────────────────────────────────
#[derive(Deserialize, Default)]
struct StdinData {
    cwd: Option<String>,
    #[serde(default)]
    workspace: Option<Workspace>,
    model: Option<Model>,
    context_window: Option<ContextWindow>,
    rate_limits: Option<RateLimits>,
    // effort: string | { level: string } | null (Claude Code 2.1.115+)
    effort: Option<Value>,
}

#[derive(Deserialize, Default)]
struct Workspace {
    current_dir: Option<String>,
}

#[derive(Deserialize)]
struct Model {
    display_name: Option<String>,
    id: Option<String>,
}

#[derive(Deserialize)]
struct ContextWindow {
    context_window_size: Option<u64>,
    used_percentage: Option<f64>,
    current_usage: Option<CurrentUsage>,
}

#[derive(Deserialize)]
struct CurrentUsage {
    input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct RateLimits {
    five_hour: Option<RateLimit>,
    seven_day: Option<RateLimit>,
}

#[derive(Deserialize)]
struct RateLimit {
    used_percentage: Option<f64>,
    resets_at: Option<f64>,
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn read_stdin() -> String {
    let mut buf = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut buf) {
        eprintln!("statusline-rs: stdin read error: {e}");
        std::process::exit(1);
    }
    buf
}

/// 末尾 N 階層のディレクトリ名を返す（pathLevels 相当）
fn dir_name(cwd: &str, levels: usize) -> String {
    let home = std::env::var("HOME").ok();
    dir_name_impl(cwd, levels, home.as_deref())
}

fn dir_name_impl(cwd: &str, levels: usize, home: Option<&str>) -> String {
    let expanded = if let Some(h) = home {
        cwd.replacen(h, "~", 1)
    } else {
        cwd.to_string()
    };

    let parts: Vec<&str> = expanded.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return "/".to_string();
    }
    let start = parts.len().saturating_sub(levels);
    // 先頭が ~ のとき prefix を維持
    if expanded.starts_with("~/") || expanded == "~" {
        let tail = parts[start..].to_vec();
        if start == 0 {
            format!("~/{}", tail.join("/"))
        } else {
            tail.join("/")
        }
    } else {
        parts[start..].join("/")
    }
}

struct GitInfo {
    branch: String,
    is_dirty: bool,
}

struct LocationInfo {
    display: String,
    git_root: String,
}

/// cwd から表示用ロケーション情報を解決する
/// `.claude/worktrees/<name>/` が含まれる場合: "repo > wt_name"
/// 通常の場合: dir_name(cwd, 1)
fn resolve_location(cwd: &str) -> LocationInfo {
    // `.claude/worktrees/<name>` パターンを検索
    if let Some(pos) = cwd.find("/.claude/worktrees/") {
        let after = &cwd[pos + "/.claude/worktrees/".len()..];
        let wt_name = after.split('/').next().unwrap_or("");
        if !wt_name.is_empty() {
            let repo_root = &cwd[..pos];
            let repo_name = repo_root.split('/').last().unwrap_or(repo_root);
            let git_root = format!("{repo_root}/.claude/worktrees/{wt_name}");
            return LocationInfo {
                display: format!("{repo_name} > {wt_name}"),
                git_root,
            };
        }
    }
    // 通常パス
    LocationInfo {
        display: dir_name(cwd, 1),
        git_root: cwd.to_string(),
    }
}

/// git -C <dir> でブランチと dirty 状態を取得（失敗時は None）
fn git_info(cwd: &str) -> Option<GitInfo> {
    let branch_out = Command::new("git")
        .args(["-C", cwd, "symbolic-ref", "--short", "HEAD"])
        .output()
        .ok()?;

    if !branch_out.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&branch_out.stdout).trim().to_string();
    if branch.is_empty() {
        return None;
    }

    let dirty_out = Command::new("git")
        .args(["-C", cwd, "status", "--porcelain"])
        .output()
        .ok()?;
    let is_dirty = !dirty_out.stdout.is_empty();

    Some(GitInfo { branch, is_dirty })
}

/// effort フィールドを文字列に変換（string | {level} の両形式対応）
fn parse_effort(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(map) => map
            .get("level")
            .and_then(|lv| lv.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// context 使用率を 0-100 で返す
fn context_pct(ctx: &ContextWindow) -> u32 {
    // Claude Code 2.1.6+ は used_percentage を直接送る
    if let Some(pct) = ctx.used_percentage {
        if pct > 0.0 {
            return pct.max(0.0).round() as u32;
        }
    }
    // フォールバック: トークン合計から計算
    let size = ctx.context_window_size.unwrap_or(0);
    if size == 0 {
        return 0;
    }
    if let Some(usage) = &ctx.current_usage {
        let total = usage.input_tokens.unwrap_or(0)
            + usage.cache_creation_input_tokens.unwrap_or(0)
            + usage.cache_read_input_tokens.unwrap_or(0);
        return ((total as f64 / size as f64) * 100.0).max(0.0) as u32;
    }
    0
}

/// claudeline の 4 ゾーン配色
fn context_color(pct: u32) -> &'static str {
    match pct {
        0..=40 => GREEN,
        41..=60 => YELLOW,
        61..=80 => ORANGE,
        81..=100 => RED,
        _ => MAGENTA,
    }
}

// btop スタイル 8段階ブライユバー。partial char は単色だが文字自体が充填率を表現する。空: ⡀（DIM）
const BRAILLE_LEVELS: [char; 9] = ['⠀', '⡀', '⣀', '⣄', '⣤', '⣦', '⣶', '⣷', '⣿'];

fn render_bar(pct: u32, width: usize, color: &str) -> String {
    let filled_eighths = ((pct as usize * width * 8) / 100).min(width * 8);
    let full_chars = filled_eighths / 8;
    let partial = filled_eighths % 8;
    let has_partial = partial > 0 && full_chars < width;
    let empty_chars = width - full_chars - if has_partial { 1 } else { 0 };

    let mut s = format!("{color}");
    for _ in 0..full_chars {
        s.push('⣿');
    }
    if has_partial {
        s.push(BRAILLE_LEVELS[partial]);
    }
    s.push_str(DIM);
    for _ in 0..empty_chars {
        s.push('⡀');
    }
    s.push_str(RESET);
    s
}

fn format_reset(resets_at: f64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs_f64();
    format_reset_impl(resets_at, now)
}

fn format_reset_impl(resets_at: f64, now_secs: f64) -> String {
    let secs = (resets_at - now_secs).max(0.0) as u64;
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    if d > 0 {
        format!("{}d {}h {}m", d, h, m)
    } else if h > 0 {
        format!("{}h {}m", h, m)
    } else {
        format!("{}m", m)
    }
}

// ── render ────────────────────────────────────────────────────────────────────

fn render_dir_line(stdin: &StdinData) -> Option<String> {
    // workspace.current_dir を優先、なければ cwd
    let cwd = stdin
        .workspace
        .as_ref()
        .and_then(|w| w.current_dir.as_deref())
        .or(stdin.cwd.as_deref())?;

    let loc = resolve_location(cwd);
    let mut s = format!("{YELLOW}{}{RESET}", loc.display);

    if let Some(git) = git_info(&loc.git_root) {
        let dirty = if git.is_dirty { "*" } else { "" };
        s.push_str(&format!(
            " {DIM}on{RESET} {CYAN_BRIGHT}{branch}{dirty}{RESET}",
            branch = git.branch,
        ));
    }
    Some(s)
}

fn render_identity_line(stdin: &StdinData) -> String {
    let mut parts: Vec<String> = Vec::new();

    // [Model | effort]
    let model_name = stdin
        .model
        .as_ref()
        .and_then(|m| m.display_name.as_deref().or(m.id.as_deref()))
        .unwrap_or("Unknown");

    let effort = stdin.effort.as_ref().and_then(|e| parse_effort(e));
    let badge = match effort.as_deref() {
        Some(e) => format!("{CYAN}[{model_name} | {e}]{RESET}"),
        None => format!("{CYAN}[{model_name}]{RESET}"),
    };
    parts.push(badge);

    // Context bar
    if let Some(ctx) = &stdin.context_window {
        let pct = context_pct(ctx);
        let color = context_color(pct);
        let bar = render_bar(pct, 10, color);
        parts.push(format!(
            "{DIM}Context{RESET} {bar} {color}{pct}%{RESET}"
        ));
    }

    // 5h usage
    if let Some(rl) = &stdin.rate_limits {
        if let Some(fh) = &rl.five_hour {
            if let Some(pct_f) = fh.used_percentage {
                let pct = pct_f.max(0.0).round() as u32;
                let bar = render_bar(pct, 10, BLUE_BRIGHT);
                let reset = fh
                    .resets_at
                    .map(|t| format!(" {DIM}(resets in {}){RESET}", format_reset(t)))
                    .unwrap_or_default();
                parts.push(format!(
                    "{DIM}5h{RESET} {bar} {BLUE_BRIGHT}{pct}%{RESET}{reset}"
                ));
            }
        }

        // 7d usage
        if let Some(sd) = &rl.seven_day {
            if let Some(pct_f) = sd.used_percentage {
                let pct = pct_f.max(0.0).round() as u32;
                let bar = render_bar(pct, 10, BLUE_BRIGHT);
                let reset = sd
                    .resets_at
                    .map(|t| format!(" {DIM}(resets in {}){RESET}", format_reset(t)))
                    .unwrap_or_default();
                parts.push(format!(
                    "{DIM}7d{RESET} {bar} {BLUE_BRIGHT}{pct}%{RESET}{reset}"
                ));
            }
        }
    }

    let sep = format!(" {DIM}│{RESET} ");
    parts.join(&sep)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn strip_ansi(s: &str) -> String {
        let mut result = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for ch in chars.by_ref() {
                    if ch == 'm' {
                        break;
                    }
                }
            } else {
                result.push(c);
            }
        }
        result
    }

    // ── dir_name ──────────────────────────────────────────────────────────────

    #[test]
    fn test_dir_name_home_substitution() {
        let result = dir_name_impl("/Users/foo/Projects/cc-statusline", 1, Some("/Users/foo"));
        assert_eq!(result, "cc-statusline");
    }

    #[test]
    fn test_dir_name_last_level() {
        let result = dir_name_impl("/Users/foo/bar/baz", 1, None);
        assert_eq!(result, "baz");
    }

    #[test]
    fn test_dir_name_root() {
        let result = dir_name_impl("/", 1, None);
        assert_eq!(result, "/");
    }

    // ── context_pct ───────────────────────────────────────────────────────────

    #[test]
    fn test_context_pct_used_percentage_direct() {
        let ctx = ContextWindow {
            context_window_size: None,
            used_percentage: Some(26.0),
            current_usage: None,
        };
        assert_eq!(context_pct(&ctx), 26);
    }

    #[test]
    fn test_context_pct_token_fallback() {
        let ctx = ContextWindow {
            context_window_size: Some(200_000),
            used_percentage: None,
            current_usage: Some(CurrentUsage {
                input_tokens: Some(50_000),
                cache_creation_input_tokens: Some(0),
                cache_read_input_tokens: Some(0),
            }),
        };
        assert_eq!(context_pct(&ctx), 25);
    }

    #[test]
    fn test_context_pct_both_none() {
        let ctx = ContextWindow {
            context_window_size: None,
            used_percentage: None,
            current_usage: None,
        };
        assert_eq!(context_pct(&ctx), 0);
    }

    #[test]
    fn test_context_pct_used_percentage_zero_falls_through_to_token() {
        // used_percentage = 0.0 は「値なし」として扱い token fallback へ落ちる
        let ctx = ContextWindow {
            context_window_size: Some(200_000),
            used_percentage: Some(0.0),
            current_usage: Some(CurrentUsage {
                input_tokens: Some(50_000),
                cache_creation_input_tokens: Some(0),
                cache_read_input_tokens: Some(0),
            }),
        };
        assert_eq!(context_pct(&ctx), 25);
    }

    // ── render_bar ────────────────────────────────────────────────────────────

    #[test]
    fn test_render_bar_zero() {
        let bar = strip_ansi(&render_bar(0, 10, ""));
        let expected: String = std::iter::repeat('⡀').take(10).collect();
        assert_eq!(bar, expected);
    }

    #[test]
    fn test_render_bar_full() {
        let bar = strip_ansi(&render_bar(100, 10, ""));
        let expected: String = std::iter::repeat('⣿').take(10).collect();
        assert_eq!(bar, expected);
    }

    #[test]
    fn test_render_bar_half() {
        let bar = strip_ansi(&render_bar(50, 10, ""));
        let full: String = std::iter::repeat('⣿').take(5).collect();
        let empty: String = std::iter::repeat('⡀').take(5).collect();
        assert_eq!(bar, format!("{full}{empty}"));
    }

    #[test]
    fn test_render_bar_partial_low() {
        // 25% → filled_eighths=20, full=2, partial=4 (⣤), empty=7
        let bar = strip_ansi(&render_bar(25, 10, ""));
        let full: String = std::iter::repeat('⣿').take(2).collect();
        let empty: String = std::iter::repeat('⡀').take(7).collect();
        assert_eq!(bar, format!("{full}⣤{empty}"));
    }

    #[test]
    fn test_render_bar_partial_high() {
        // 87% → filled_eighths=69, full=8, partial=5 (⣦), empty=1
        let bar = strip_ansi(&render_bar(87, 10, ""));
        let full: String = std::iter::repeat('⣿').take(8).collect();
        assert_eq!(bar, format!("{full}⣦⡀"));
    }

    // ── format_reset ──────────────────────────────────────────────────────────

    #[test]
    fn test_format_reset_hours() {
        let now = 1_000_000.0_f64;
        assert_eq!(format_reset_impl(now + 7200.0, now), "2h 0m");
    }

    #[test]
    fn test_format_reset_days() {
        let now = 1_000_000.0_f64;
        assert_eq!(format_reset_impl(now + 3.0 * 86400.0, now), "3d 0h 0m");
    }

    #[test]
    fn test_format_reset_past() {
        let now = 1_000_000.0_f64;
        assert_eq!(format_reset_impl(now - 100.0, now), "0m");
    }

    // ── parse_effort ──────────────────────────────────────────────────────────

    #[test]
    fn test_parse_effort_string() {
        let v = json!("medium");
        assert_eq!(parse_effort(&v), Some("medium".to_string()));
    }

    #[test]
    fn test_parse_effort_object() {
        let v = json!({"level": "high"});
        assert_eq!(parse_effort(&v), Some("high".to_string()));
    }

    #[test]
    fn test_parse_effort_null() {
        let v = json!(null);
        assert_eq!(parse_effort(&v), None);
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let raw = read_stdin();
    let stdin: StdinData = serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!("statusline-rs: JSON parse error: {e}");
        eprintln!("input: {raw}");
        std::process::exit(1);
    });

    // Line 1: directory + git branch
    if let Some(line) = render_dir_line(&stdin) {
        println!("{RESET}{line}");
    }

    // Line 2: [model | effort] │ Context bar │ Usage bar
    let identity = render_identity_line(&stdin);
    if !identity.is_empty() {
        print!("{RESET}{identity}");
    }
}
