use serde::Deserialize;
use serde_json::Value;
use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

    // cwd が HOME と完全一致するとき expanded == "~" になる。
    // このまま parts を作ると ["~"] となり "~/~" が返るため早期リターン。
    if expanded == "~" {
        return "~".to_string();
    }

    let parts: Vec<&str> = expanded.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return "/".to_string();
    }
    let start = parts.len().saturating_sub(levels);
    // 先頭が ~ のとき prefix を維持
    if expanded.starts_with("~/") || expanded == "~" {
        if start == 0 {
            format!("~/{}", parts[start..].join("/"))
        } else {
            parts[start..].join("/")
        }
    } else {
        parts[start..].join("/")
    }
}

struct GitInfo {
    branch: String,
    /// Some(true) = dirty, Some(false) = clean, None = status unknown (command failed)
    is_dirty: Option<bool>,
}

/// `git status --branch --porcelain=v2 -uno` の stdout からブランチ名と dirty 状態を解析する。
///
/// porcelain=v2 形式:
///   # branch.oid <sha>
///   # branch.head <name>          ("(detached)" のとき detached HEAD)
///   # branch.upstream <name>      (省略可)
///   # branch.ab +N -M             (省略可)
///   1 <xy> ...                    ← 変更エントリ（# 以外の非空行）
///
/// -uno により untracked 行（'?'）は出ない。`# branch.head` が無ければ None。
fn parse_porcelain_v2(stdout: &[u8]) -> Option<GitInfo> {
    let text = std::str::from_utf8(stdout).ok()?;

    let mut branch_head: Option<&str> = None;
    let mut branch_oid: Option<&str> = None;
    let mut is_dirty = false;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            branch_head = Some(rest);
        } else if let Some(rest) = line.strip_prefix("# branch.oid ") {
            branch_oid = Some(rest);
        } else if !line.starts_with('#') && !line.is_empty() {
            is_dirty = true;
        }
    }

    let head = branch_head?;
    let branch = if head == "(detached)" {
        // detached HEAD: SHA を 7 文字に切り詰めて表示（git rev-parse --short と同等）
        let oid = branch_oid?;
        let short_len = oid.len().min(7);
        format!("*({})", &oid[..short_len])
    } else {
        head.to_string()
    };

    Some(GitInfo {
        branch,
        is_dirty: Some(is_dirty),
    })
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
            let repo_name = repo_root.split('/').next_back().unwrap_or(repo_root);
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

/// git サブプロセスに許容する最大待機時間。
/// NFS/SMB マウントや index lock 競合で git がハングした場合に、
/// プロンプト全体がフリーズするのを防ぐため強制 kill する。
/// Starship (https://github.com/starship/starship) と同じ 500ms を採用。
const GIT_TIMEOUT: Duration = Duration::from_millis(500);

/// try_wait ポーリング間隔。短命プロンプトでは 5ms 単位で十分。
const GIT_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// 任意の `Command` をタイムアウト付きで実行し、成功した stdout を返す。
///
/// Issue #21: `Command::output()` は無期限ブロックするため、`spawn()` + `try_wait()`
/// ポーリングで `timeout` を超えたら子プロセスを kill + reap する。
/// kill 後の `wait()` を省くとゾンビが親 PID に紐付き session 終了まで残るため必須。
///
/// 引数 `cmd` は呼び出し側で `stdout(Stdio::piped())` / `stderr(Stdio::null())` 等の
/// `Stdio` 設定を済ませた状態で渡す。テストでは `Command::new("sleep")` 等を渡して
/// タイムアウト経路を検証する。
fn run_command_with_timeout(mut cmd: Command, timeout: Duration) -> Option<Vec<u8>> {
    let mut child = cmd.spawn().ok()?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut buf = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    out.read_to_end(&mut buf).ok()?;
                }
                return if status.success() { Some(buf) } else { None };
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                thread::sleep(GIT_POLL_INTERVAL);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// git コマンドをタイムアウト付きで実行し、成功した stdout を返す。
/// stderr は `Stdio::null()` でターミナルへの出力を抑制する。
fn run_git_with_timeout(args: &[&str]) -> Option<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::null());
    run_command_with_timeout(cmd, GIT_TIMEOUT)
}

/// git -C <dir> でブランチと dirty 状態を取得（失敗・タイムアウト時は None）。
///
/// Issue #22: サブプロセスを 3 本から 1 本に統合（fork/exec オーバーヘッド削減）。
/// `-uno` で untracked スキャンを省略し `node_modules` 等を持つ repo でも高速。
/// `--no-optional-locks` で index lock 競合時に読み取り専用で処理し競合を回避。
/// Issue #21: `run_git_with_timeout` で 500ms タイムアウトを強制し、
/// NFS ハング時もシェルがフリーズしない。
///
/// 設計判断: `-uno` を採用する代わりに untracked のみ存在する状態では dirty
/// マーカー (`*`) が出ない。これは Starship / Pure / Spaceship 等の主要プロンプト
/// と同じ振る舞いで、プロンプト毎呼び出しの p99 レイテンシを優先する選択。
/// untracked を別マーカーで出す案（Starship の `?` 等）も将来検討可能だが、
/// 現状はミニマリズム維持を優先する。要件が変われば `-uno` を環境変数で
/// 切り替え可能にする拡張余地を残す。
fn git_info(cwd: &str) -> Option<GitInfo> {
    let stdout = run_git_with_timeout(&[
        "-C",
        cwd,
        "--no-optional-locks",
        "status",
        "--branch",
        "--porcelain=v2",
        "-uno",
    ])?;
    parse_porcelain_v2(&stdout)
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
    // Claude Code 2.1.6+ は used_percentage を直接送る。
    // pct == 0.0 は「未計測 / セッション開始直後」の sentinel として扱い、
    // フォールバックの token 合計計算に落とす (両方 0 の場合はそのまま 0 が返る)。
    if let Some(pct) = ctx.used_percentage {
        if pct > 0.0 {
            return pct.clamp(0.0, 100.0).round() as u32;
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
        return ((total as f64 / size as f64) * 100.0) as u32;
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

// btop スタイル ブライユバー。partial 充填用 7 段階 (1/8 〜 7/8)。
// 0/8 は DIM 適用の '⡀' を空 cell に、8/8 は '⣿' を full cell にハードコードで使う
// ため、ここには partial 1..=7 だけを格納する (`partial - 1` で索引する)。
const BRAILLE_PARTIAL_LEVELS: [char; 7] = ['⡀', '⣀', '⣄', '⣤', '⣦', '⣶', '⣷'];

fn render_bar(pct: u32, width: usize, color: &str) -> String {
    let filled_eighths = ((pct as usize * width * 8) / 100).min(width * 8);
    let full_chars = filled_eighths / 8;
    let partial = filled_eighths % 8;
    let has_partial = partial > 0 && full_chars < width;
    let empty_chars = width - full_chars - if has_partial { 1 } else { 0 };

    let mut s = String::from(color);
    for _ in 0..full_chars {
        s.push('⣿');
    }
    if has_partial {
        s.push(BRAILLE_PARTIAL_LEVELS[partial - 1]);
    }
    s.push_str(DIM);
    for _ in 0..empty_chars {
        s.push('⡀');
    }
    s.push_str(RESET);
    s
}

fn render_rate_limit_part(label: &str, rl: Option<&RateLimit>, color: &str) -> Option<String> {
    let rl = rl?;
    let pct_f = rl.used_percentage?;
    let pct = pct_f.clamp(0.0, 100.0).round() as u32;
    let bar = render_bar(pct, 10, color);
    let reset = rl
        .resets_at
        .and_then(format_reset)
        .map(|s| format!(" {DIM}(resets in {s}){RESET}"))
        .unwrap_or_default();
    Some(format!(
        "{DIM}{label}{RESET} {bar} {color}{pct}%{RESET}{reset}"
    ))
}

fn format_reset(resets_at: f64) -> Option<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs_f64();
    format_reset_impl(resets_at, now)
}

fn format_reset_impl(resets_at: f64, now_secs: f64) -> Option<String> {
    if resets_at <= now_secs {
        return None;
    }
    let secs = (resets_at - now_secs) as u64;
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    Some(if d > 0 {
        format!("{}d {}h {}m", d, h, m)
    } else if h > 0 {
        format!("{}h {}m", h, m)
    } else {
        format!("{}m", m)
    })
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
        let dirty = match git.is_dirty {
            Some(true) => "*",
            Some(false) => "",
            None => "?",
        };
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

    let effort = stdin.effort.as_ref().and_then(parse_effort);
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
        parts.push(format!("{DIM}Context{RESET} {bar} {color}{pct}%{RESET}"));
    }

    if let Some(rl) = &stdin.rate_limits {
        parts.extend(render_rate_limit_part(
            "5h",
            rl.five_hour.as_ref(),
            BLUE_BRIGHT,
        ));
        parts.extend(render_rate_limit_part(
            "7d",
            rl.seven_day.as_ref(),
            BLUE_BRIGHT,
        ));
    }

    let sep = format!(" {DIM}│{RESET} ");
    parts.join(&sep)
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let raw = read_stdin();
    let stdin: StdinData = serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!("statusline-rs: JSON parse error: {e}");
        std::process::exit(1);
    });

    // Line 1: directory + git branch (改行で Line 2 と分離)
    if let Some(line) = render_dir_line(&stdin) {
        println!("{RESET}{line}");
    }

    // Line 2: [model | effort] │ Context bar │ Usage bar
    // 末尾改行を出さない — statusline は親プロセス (Claude Code) が改行を制御する。
    // `println!` で改行を足すと表示行が 1 行ずれる。
    print!("{RESET}{}", render_identity_line(&stdin));
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

    #[test]
    fn test_dir_name_cwd_equals_home() {
        // cwd が HOME と完全一致するとき "~" を返す（"~/~" にならない）
        let result = dir_name_impl("/Users/foo", 1, Some("/Users/foo"));
        assert_eq!(result, "~");
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
    fn test_context_pct_clamp_over_100() {
        // API が 100% 超の値を送っても 100 にクランプする (#24)
        let ctx = ContextWindow {
            context_window_size: None,
            used_percentage: Some(105.0),
            current_usage: None,
        };
        assert_eq!(context_pct(&ctx), 100);
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
        assert_eq!(bar, "⡀".repeat(10));
    }

    #[test]
    fn test_render_bar_full() {
        let bar = strip_ansi(&render_bar(100, 10, ""));
        assert_eq!(bar, "⣿".repeat(10));
    }

    #[test]
    fn test_render_bar_half() {
        let bar = strip_ansi(&render_bar(50, 10, ""));
        assert_eq!(bar, format!("{}{}", "⣿".repeat(5), "⡀".repeat(5)));
    }

    #[test]
    fn test_render_bar_partial_low() {
        // 25% → filled_eighths=20, full=2, partial=4 (⣤), empty=7
        let bar = strip_ansi(&render_bar(25, 10, ""));
        assert_eq!(bar, format!("{}⣤{}", "⣿".repeat(2), "⡀".repeat(7)));
    }

    #[test]
    fn test_render_bar_partial_high() {
        // 87% → filled_eighths=69, full=8, partial=5 (⣦), empty=1
        let bar = strip_ansi(&render_bar(87, 10, ""));
        assert_eq!(bar, format!("{}⣦⡀", "⣿".repeat(8)));
    }

    // ── format_reset ──────────────────────────────────────────────────────────

    #[test]
    fn test_format_reset_hours() {
        let now = 1_000_000.0_f64;
        assert_eq!(
            format_reset_impl(now + 7200.0, now),
            Some("2h 0m".to_string())
        );
    }

    #[test]
    fn test_format_reset_days() {
        let now = 1_000_000.0_f64;
        assert_eq!(
            format_reset_impl(now + 3.0 * 86400.0, now),
            Some("3d 0h 0m".to_string())
        );
    }

    #[test]
    fn test_format_reset_past() {
        // 期限切れ時は None — "(resets in 0m)" という誤表示を防ぐ (#37)
        let now = 1_000_000.0_f64;
        assert_eq!(format_reset_impl(now - 100.0, now), None);
    }

    #[test]
    fn test_format_reset_exact_now() {
        // resets_at == now のときも期限切れ扱い
        let now = 1_000_000.0_f64;
        assert_eq!(format_reset_impl(now, now), None);
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

    #[test]
    fn test_parse_effort_object_without_level_key() {
        // Issue #38: "level" キーがない Object は None を返す
        let v = json!({"mode": "high"});
        assert_eq!(parse_effort(&v), None);
    }

    // ── resolve_location ──────────────────────────────────────────────────────

    #[test]
    fn test_resolve_location_normal() {
        // Issue #19: 通常パス → display = 末尾ディレクトリ, git_root = cwd
        let loc = resolve_location("/home/user/my-project");
        assert_eq!(loc.display, "my-project");
        assert_eq!(loc.git_root, "/home/user/my-project");
    }

    #[test]
    fn test_resolve_location_worktree() {
        // Issue #19: worktree パス → "repo > wt_name", git_root は worktree root
        let loc = resolve_location("/home/user/repo/.claude/worktrees/feat-x");
        assert_eq!(loc.display, "repo > feat-x");
        assert_eq!(loc.git_root, "/home/user/repo/.claude/worktrees/feat-x");
    }

    #[test]
    fn test_resolve_location_worktree_with_subdir() {
        // worktree 内のサブディレクトリでも repo root と wt 名で表示する
        let loc = resolve_location("/home/user/repo/.claude/worktrees/feat-x/src");
        assert_eq!(loc.display, "repo > feat-x");
        assert_eq!(loc.git_root, "/home/user/repo/.claude/worktrees/feat-x");
    }

    #[test]
    fn test_resolve_location_worktree_empty_name() {
        // wt_name が空 (`.claude/worktrees/` で終わる) → 通常パスにフォールバック
        let loc = resolve_location("/home/user/repo/.claude/worktrees/");
        assert!(!loc.display.contains('>'));
    }

    // ── render_rate_limit_part ────────────────────────────────────────────────

    #[test]
    fn test_render_rate_limit_part_none_rl() {
        // Issue #30: rl が None → None
        assert!(render_rate_limit_part("5h", None, BLUE_BRIGHT).is_none());
    }

    #[test]
    fn test_render_rate_limit_part_none_percentage() {
        // Issue #30: used_percentage が None → None
        let rl = RateLimit {
            used_percentage: None,
            resets_at: Some(1_000_000.0),
        };
        assert!(render_rate_limit_part("5h", Some(&rl), BLUE_BRIGHT).is_none());
    }

    #[test]
    fn test_render_rate_limit_part_with_reset() {
        // Issue #30: resets_at あり → "(resets in ...)" を含む
        let rl = RateLimit {
            used_percentage: Some(42.0),
            resets_at: Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64()
                    + 3600.0,
            ),
        };
        let out = render_rate_limit_part("5h", Some(&rl), BLUE_BRIGHT).unwrap();
        let plain = strip_ansi(&out);
        assert!(plain.starts_with("5h "));
        assert!(plain.contains("42%"));
        assert!(plain.contains("(resets in "));
    }

    #[test]
    fn test_render_rate_limit_part_without_reset() {
        // Issue #30: resets_at なし → reset サフィックスなし
        let rl = RateLimit {
            used_percentage: Some(10.0),
            resets_at: None,
        };
        let out = render_rate_limit_part("7d", Some(&rl), BLUE_BRIGHT).unwrap();
        let plain = strip_ansi(&out);
        assert!(plain.contains("10%"));
        assert!(!plain.contains("resets in"));
    }

    // ── render_identity_line ──────────────────────────────────────────────────

    #[test]
    fn test_render_identity_line_model_only() {
        // Issue #31: モデルのみ。context_window / rate_limits / effort なし
        let stdin = StdinData {
            cwd: None,
            workspace: None,
            model: Some(Model {
                display_name: Some("Sonnet 4.6".to_string()),
                id: None,
            }),
            context_window: None,
            rate_limits: None,
            effort: None,
        };
        let plain = strip_ansi(&render_identity_line(&stdin));
        assert_eq!(plain, "[Sonnet 4.6]");
    }

    #[test]
    fn test_render_identity_line_model_id_fallback() {
        // display_name が None なら id にフォールバックする
        let stdin = StdinData {
            cwd: None,
            workspace: None,
            model: Some(Model {
                display_name: None,
                id: Some("opus-4-7".to_string()),
            }),
            context_window: None,
            rate_limits: None,
            effort: None,
        };
        let plain = strip_ansi(&render_identity_line(&stdin));
        assert_eq!(plain, "[opus-4-7]");
    }

    #[test]
    fn test_render_identity_line_unknown_model() {
        // model が None のとき "Unknown" badge を出す
        let stdin = StdinData::default();
        let plain = strip_ansi(&render_identity_line(&stdin));
        assert_eq!(plain, "[Unknown]");
    }

    #[test]
    fn test_render_identity_line_with_effort_and_context() {
        // effort + context bar を含むときセパレータでつながる
        let stdin = StdinData {
            cwd: None,
            workspace: None,
            model: Some(Model {
                display_name: Some("Opus 4.7".to_string()),
                id: None,
            }),
            context_window: Some(ContextWindow {
                context_window_size: None,
                used_percentage: Some(26.0),
                current_usage: None,
            }),
            rate_limits: None,
            effort: Some(json!("high")),
        };
        let plain = strip_ansi(&render_identity_line(&stdin));
        assert!(plain.starts_with("[Opus 4.7 | high]"));
        assert!(plain.contains(" │ "));
        assert!(plain.contains("Context "));
        assert!(plain.contains("26%"));
    }

    #[test]
    fn test_render_identity_line_with_rate_limits() {
        // rate_limits があれば 5h / 7d 部分が並ぶ
        let stdin = StdinData {
            cwd: None,
            workspace: None,
            model: Some(Model {
                display_name: Some("Sonnet 4.6".to_string()),
                id: None,
            }),
            context_window: None,
            rate_limits: Some(RateLimits {
                five_hour: Some(RateLimit {
                    used_percentage: Some(15.0),
                    resets_at: None,
                }),
                seven_day: Some(RateLimit {
                    used_percentage: Some(33.0),
                    resets_at: None,
                }),
            }),
            effort: None,
        };
        let plain = strip_ansi(&render_identity_line(&stdin));
        assert!(plain.contains("5h "));
        assert!(plain.contains("15%"));
        assert!(plain.contains("7d "));
        assert!(plain.contains("33%"));
    }

    // ── parse_porcelain_v2 ────────────────────────────────────────────────────

    #[test]
    fn test_parse_porcelain_v2_clean_branch() {
        // ブランチ上で変更なし: branch のみ、is_dirty=false
        let stdout = b"# branch.oid 0123456789abcdef0123456789abcdef01234567\n\
                       # branch.head main\n\
                       # branch.upstream origin/main\n\
                       # branch.ab +0 -0\n";
        let gi = parse_porcelain_v2(stdout).unwrap();
        assert_eq!(gi.branch, "main");
        assert_eq!(gi.is_dirty, Some(false));
    }

    #[test]
    fn test_parse_porcelain_v2_dirty_branch() {
        // 変更エントリ '1 .M ...' があれば is_dirty=true
        let stdout = b"# branch.oid 0123456789abcdef0123456789abcdef01234567\n\
                       # branch.head feature/x\n\
                       1 .M N... 100644 100644 100644 abc abc src/main.rs\n";
        let gi = parse_porcelain_v2(stdout).unwrap();
        assert_eq!(gi.branch, "feature/x");
        assert_eq!(gi.is_dirty, Some(true));
    }

    #[test]
    fn test_parse_porcelain_v2_detached_head() {
        // detached HEAD: SHA を 7 文字に切って *() で囲む
        let stdout = b"# branch.oid deadbeef1234567890abcdef0123456789abcdef\n\
                       # branch.head (detached)\n";
        let gi = parse_porcelain_v2(stdout).unwrap();
        assert_eq!(gi.branch, "*(deadbee)");
        assert_eq!(gi.is_dirty, Some(false));
    }

    #[test]
    fn test_parse_porcelain_v2_detached_head_dirty() {
        let stdout = b"# branch.oid deadbeef1234567890abcdef0123456789abcdef\n\
                       # branch.head (detached)\n\
                       2 R. N... 100644 100644 100644 abc def R100 new.rs\told.rs\n";
        let gi = parse_porcelain_v2(stdout).unwrap();
        assert_eq!(gi.branch, "*(deadbee)");
        assert_eq!(gi.is_dirty, Some(true));
    }

    #[test]
    fn test_parse_porcelain_v2_missing_branch_head() {
        // # branch.head が無ければ None（git の出力異常時）
        let stdout = b"# branch.oid 0123456789abcdef0123456789abcdef01234567\n";
        assert!(parse_porcelain_v2(stdout).is_none());
    }

    #[test]
    fn test_parse_porcelain_v2_invalid_utf8() {
        let stdout = &[0xffu8, 0xfe, 0xfd][..];
        assert!(parse_porcelain_v2(stdout).is_none());
    }

    // ── run_command_with_timeout ──────────────────────────────────────────────
    //
    // Unix の `true` / `false` / `sleep` を使って成功・失敗・タイムアウト経路を検証する。
    // Windows 環境では実行されないため `#[cfg(unix)]` でガード。

    #[cfg(unix)]
    #[test]
    fn test_run_command_with_timeout_success() {
        // `printf hello` は exit 0 で stdout に "hello" を出力
        let mut cmd = Command::new("printf");
        cmd.arg("hello")
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let out = run_command_with_timeout(cmd, Duration::from_secs(2)).unwrap();
        assert_eq!(out, b"hello");
    }

    #[cfg(unix)]
    #[test]
    fn test_run_command_with_timeout_nonzero_exit() {
        // `false` は exit 1 → None
        let mut cmd = Command::new("false");
        cmd.stdout(Stdio::piped()).stderr(Stdio::null());
        assert!(run_command_with_timeout(cmd, Duration::from_secs(2)).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn test_run_command_with_timeout_times_out_and_kills() {
        // `sleep 10` を 50ms タイムアウトで起動: kill + reap が走り、
        // 実時間 50ms 程度（10 秒待たない）で None が返ることを確認。
        let mut cmd = Command::new("sleep");
        cmd.arg("10").stdout(Stdio::piped()).stderr(Stdio::null());
        let start = Instant::now();
        let result = run_command_with_timeout(cmd, Duration::from_millis(50));
        let elapsed = start.elapsed();
        assert!(result.is_none());
        // kill が走らなければ 10 秒待つはずなので、1 秒以内なら kill 経路通過と判定
        assert!(
            elapsed < Duration::from_secs(1),
            "elapsed = {elapsed:?} (kill が走っていない可能性)"
        );
    }
}
