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
use std::hint::black_box;
use std::io::Write;
use std::process::{Command, Stdio};

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
    // 個別 sample あたり 1 spawn。低 sample で wall time の中央値を見る。
    group.sample_size(50);

    group.bench_function("minimal_stdin", |b| {
        b.iter(|| run_once(bin, STDIN_MINIMAL));
    });

    group.bench_function("full_stdin", |b| {
        b.iter(|| run_once(bin, STDIN_FULL));
    });

    group.finish();
}

criterion_group!(benches, bench_statusline_full);
criterion_main!(benches);
