//! statusline-rs の E2E bench。
//!
//! cc-statusline は Claude Code が毎ターン新規プロセスで spawn するため、
//! ユーザー体感の latency は **コンパイル済み binary を subprocess として起動し、
//! stdin から JSON を流し込み、stdout を読み終えるまでの wall time** で測る。
//! fork/exec + dynamic linker + libc init + git subprocess を含む、現実の経路。
//!
//! 設計判断は docs/decisions/20260607-perf-contract-foundation-decision.md、
//! latency budget は docs/PERFORMANCE.md を参照。

use criterion::{criterion_group, criterion_main, Criterion};
use std::fs;
use std::hint::black_box;
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::TempDir;

/// criterion の 1 group あたりサンプル数。subprocess spawn が iteration ごとに走るため
/// criterion default (100) では bench 1 本に 1.5s 以上かかる。50 に絞り体感速度を保つ。
const BENCH_SAMPLE_SIZE: usize = 50;

/// huge_repo fixture が `node_modules` 相当として作る untracked file 数。
/// 中規模 Node.js project (express + 数十 dep) の `node_modules` 内ファイル数の目安。
/// `-uno` の効果を測るには 1000 未満では小さすぎ、10000 超では fixture setup が遅すぎる。
const HUGE_REPO_UNTRACKED_COUNT: usize = 5000;

/// 最小構成の stdin fixture。git 情報 + model + context のみ。
const STDIN_MINIMAL: &str = r#"{
  "cwd": ".",
  "workspace": { "current_dir": "." },
  "model": { "display_name": "Sonnet 4.6", "id": "claude-sonnet-4-6" },
  "context_window": { "context_window_size": 200000, "used_percentage": 26.0 }
}"#;

/// rate_limits 含む full fixture。
const STDIN_FULL: &str = r#"{
  "cwd": ".",
  "workspace": { "current_dir": "." },
  "model": { "display_name": "Sonnet 4.6", "id": "claude-sonnet-4-6" },
  "context_window": {
    "context_window_size": 200000,
    "used_percentage": 26.0,
    "current_usage": {
      "input_tokens": 50000,
      "cache_creation_input_tokens": 1000,
      "cache_read_input_tokens": 1000
    }
  },
  "rate_limits": {
    "five_hour":  { "used_percentage": 21.0, "resets_at": 9999999999.0 },
    "seven_day":  { "used_percentage": 31.0, "resets_at": 9999999999.0 }
  },
  "effort": "medium"
}"#;

/// `node_modules` 相当の大量 untracked file を持つ tempdir に git repo を作成する。
///
/// Issue #22 の `git status --porcelain` が untracked 全スキャンで遅くなる現場を再現し、
/// `-uno` を含む現状実装 (src/main.rs git_info) がこの環境でも budget 内に収まるかを測る。
fn setup_huge_repo(untracked_count: usize) -> TempDir {
    let dir = tempfile::tempdir().expect("create tempdir");
    let path = dir.path().to_str().expect("utf-8 path");

    let init = Command::new("git")
        .args(["-C", path, "init", "-q", "-b", "main"])
        .status()
        .expect("git init");
    assert!(init.success(), "git init failed");

    // HEAD を作るため空の初回 commit を打つ (config は -c で local override)。
    let commit = Command::new("git")
        .args([
            "-C", path,
            "-c", "user.email=bench@example.com",
            "-c", "user.name=bench",
            "-c", "commit.gpgsign=false",
            "commit", "--allow-empty", "-q", "-m", "init",
        ])
        .status()
        .expect("git commit");
    assert!(commit.success(), "git commit failed");

    // `node_modules` を模擬: 大量の untracked file を 1 ディレクトリに置く。
    let node_modules = dir.path().join("node_modules");
    fs::create_dir(&node_modules).expect("create node_modules");
    for i in 0..untracked_count {
        fs::write(node_modules.join(format!("pkg{i}.js")), "").expect("write file");
    }

    dir
}

/// huge_repo fixture を指すよう cwd を差し替えた stdin を生成する。
fn stdin_for_cwd(cwd: &str) -> String {
    format!(
        r#"{{
  "cwd": {cwd:?},
  "workspace": {{ "current_dir": {cwd:?} }},
  "model": {{ "display_name": "Sonnet 4.6", "id": "claude-sonnet-4-6" }},
  "context_window": {{ "context_window_size": 200000, "used_percentage": 26.0 }}
}}"#
    )
}

/// binary を 1 回 spawn して stdin/stdout を回す。bench の 1 iteration に相当。
fn run_once(bin: &str, stdin_payload: &str) {
    let mut child = Command::new(bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn statusline-rs");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(stdin_payload.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait child");
    black_box(output.stdout);
}

fn bench_statusline_full(c: &mut Criterion) {
    let bin = env!("CARGO_BIN_EXE_statusline-rs");

    let mut group = c.benchmark_group("statusline_e2e");
    group.sample_size(BENCH_SAMPLE_SIZE);

    group.bench_function("minimal_stdin", |b| {
        b.iter(|| run_once(bin, STDIN_MINIMAL));
    });

    group.bench_function("full_stdin", |b| {
        b.iter(|| run_once(bin, STDIN_FULL));
    });

    // huge_repo: 大量の untracked file を持つ git repo (定数は HUGE_REPO_UNTRACKED_COUNT)。
    // fixture 作成は bench 外で 1 回のみ、TempDir Drop で cleanup。
    let huge = setup_huge_repo(HUGE_REPO_UNTRACKED_COUNT);
    let huge_cwd = huge.path().to_str().expect("utf-8 path");
    let huge_stdin = stdin_for_cwd(huge_cwd);
    group.bench_function("huge_repo_5k_untracked", |b| {
        b.iter(|| run_once(bin, &huge_stdin));
    });

    group.finish();
}

criterion_group!(benches, bench_statusline_full);
criterion_main!(benches);
