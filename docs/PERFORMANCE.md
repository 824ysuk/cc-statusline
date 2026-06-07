# Performance Contract

cc-statusline は Claude Code が**毎ターン spawn する低レイテンシ component** である。本書は許容 latency と計測方法論を宣言し、以後の性能関連変更の判断基準とする。

設計判断の経緯は [`docs/decisions/20260607-perf-contract-foundation-decision.md`](./decisions/20260607-perf-contract-foundation-decision.md) を参照。

## Latency Budget

| 環境 | p50 | p99 | 上限 |
|---|---|---|---|
| local SSD, 通常 repo | < 10ms | < 30ms | — |
| local SSD, 巨大 repo（node_modules 等を含む） | < 20ms | < 50ms | — |
| network FS（NFS / SMB） | — | < 500ms | timeout で強制（[`src/main.rs`](../src/main.rs) `GIT_TIMEOUT`） |

初期値は仮置きである。`benches/statusline.rs` の実装後（PR-B 以降）に実測で校正する。

### 設計上の前提

- cc-statusline は **毎ターン新規プロセスで spawn される**ため、in-memory cache は意味を持たない
- fork/exec コスト・dynamic linker・libc init の固定費が p50 の主因
- subprocess 呼び出しは git に限定し、`--no-optional-locks` + `-uno` + `--porcelain=v2` + 500ms timeout で工学的最適に近づける（PR #48 で達成済み）

## 計測方法

### bench harness（PR-B 以降）

```zsh
cargo bench
```

bench target は `benches/statusline.rs` に集約する。最低限以下を計測:

- `git_info` — local SSD / clean repo
- `git_info` — local SSD / dirty repo
- `git_info` — 巨大 repo（fixture は別途検討）
- `render_full` — stdin fixture から statusline 1 行を組み立てる E2E

[criterion](https://crates.io/crates/criterion) を採用する。warmup・統計処理・regression detection が組み込み済みのため。dev-dependency のみで production binary には影響しない。

### CI smoke test

CI では `cargo bench --no-run` で compile を verify する。実 bench 実行は手元で行う（CI runner の性能ばらつきが大きく、絶対値の regression 検知は別途 `criterion-cmp` 等で検討する）。

## 性能 Issue の受付基準

「遅い」「重い」「フリーズする」報告を受けたとき、コード変更の検討前に以下を確認する:

1. **環境**: OS（macOS / Linux / WSL / Windows）、FS 種別（local SSD / NFS / SMB / overlay）、cc-statusline のバージョン
2. **repo 規模**: total file 数、`.git/` サイズ、`node_modules` 等の大型 untracked dir の有無
3. **`core.fsmonitor` 設定**: `git config --get core.fsmonitor`、watchman の有無
4. **測定値**: 体感ではなく実測（`time cc-statusline < fixture.json` 等）
5. **再現条件**: 「常に遅い」か「特定操作後に遅い」か

これらが揃わない Issue は、まず情報収集 label を付与し報告者に問い合わせる。仮説駆動の fix は再発する。

## 巨大 repo で遅さを訴える user へ

コード変更ではなく、まず `core.fsmonitor` 設定を案内する:

```zsh
git config core.fsmonitor true
```

fsmonitor が効いた `git status` は数万ファイルの repo でも数 ms で返る。cc-statusline 側の最適化より効果が大きい場合が多い。

## 関連

- [PR #48](https://github.com/824ysuk/cc-statusline/pull/48) — NFS 対策 timeout + `-uno` 統合
- [Issue #29](https://github.com/824ysuk/cc-statusline/issues/29) — `GitBackend` trait 化（PR-C 予定）
- [Issue #21](https://github.com/824ysuk/cc-statusline/issues/21) / [#22](https://github.com/824ysuk/cc-statusline/issues/22) — 仮説駆動 fix の事例（本契約導入の動機）
