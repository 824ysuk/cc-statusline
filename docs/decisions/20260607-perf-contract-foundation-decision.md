# 性能契約と計測 foundation の導入

- **Status**: Accepted
- **Date**: 2026-06-07
- **Deciders**: 824ysuk
- **Related**: Issue #21 (closed), Issue #22 (closed), Issue #28 (open), Issue #29 (open), Issue #30 (open), Issue #31 (open), PR #48 (merged)

## Context

cc-statusline は Claude Code が毎ターン spawn する低レイテンシ component である。直近で性能・正確性に関する Issue が連続して起票・close されたが、いずれも **before/after の測定なし** に仮説駆動で fix が入った。

- Issue #21: 「NFS でシェルがフリーズする」→ 500ms timeout を導入（PR #48）
- Issue #22: 「untracked スキャンが遅い」→ `-uno` を導入（PR #48）

5 名の top-tier エンジニア persona を独立に走らせて理想実装を検討した結果、**全員が「measure before optimize」を結論の末尾に置いた**。さらに adversarial critique は「誰も Issue #21/#22 の元 user 環境（FS 種別・repo 規模・`core.fsmonitor` 設定）を確認していない」「synthetic bench より前に対象環境を特定すべき」と指摘した。

つまり、PR #48 は「現状での工学的最適」ではあるが、その判断を支える計測基盤が repo に存在しない。

このリポジトリには以下が**一切ない**:

- `CONTEXT.md` / `docs/decisions/` 等の設計記録（本 ADR がこの欠落への最初の対処）
- latency budget の明文化（「何 ms 以下なら OK か」が未定義）
- bench harness（`cargo bench` / 時間計測 test なし）
- git_info / render path のユニットテスト可能な構造（Issue #29: subprocess 直依存）

結果として性能 Issue は仮説駆動で close され、改善の効果も regression の検知も保証されない。これは個別 Issue ではなく **systemic gap** である。

## Decision

「性能契約 + 計測 + テスト可能化」の foundation を一度入れる。以後の性能関連変更はこの foundation に基づいて判断する。

### 1. `docs/PERFORMANCE.md` — 性能契約の明文化

latency budget と計測方法論を文書化する。最小骨格:

- **目的**: cc-statusline がプロンプト毎に許容される latency 上限を宣言する
- **budget**(初期値、実測後に修正可能):
  - local SSD, 通常 repo: p50 < 10ms / p99 < 30ms
  - local SSD, 巨大 repo (node_modules 等): p99 < 50ms
  - NFS / SMB 等 network FS: p99 < 500ms（timeout 上限に等しい）
- **計測方法**: `cargo bench` の使い方、stdin fixture の用意、再現手順
- **性能 Issue の受付基準**: 報告者に環境（OS / FS / repo 規模 / `core.fsmonitor` 有無）と測定値を要求する
- **fsmonitor 案内**: 巨大 repo で遅さを訴える user 向けに `git config core.fsmonitor true` を最初に案内する FAQ

### 2. `benches/statusline.rs` — bench harness

[criterion](https://crates.io/crates/criterion) を dev-dependency として導入し、以下を計測:

- `git_info`(local SSD, clean repo)
- `git_info`(local SSD, dirty repo)
- `git_info`(巨大 repo, node_modules あり)
- `render_full`(stdin fixture から statusline 1 行を組み立てる E2E)

criterion 採用理由: warmup・統計処理・regression detection が組み込み済み。dev-only 依存のため production binary には影響しない。

代替案 1: 自前 `std::time::Instant` での簡易計測 → warmup・統計が無いと p50/p99 を正しく取れない。却下。
代替案 2: `cargo bench` の組み込み（unstable）→ stable Rust で使えない。却下。

### 3. Issue #29 対応 — git_info の trait 化

`GitBackend` trait を導入し、`git_info` を以下に分離:

- `trait GitBackend { fn status(&self, cwd: &str) -> Option<Vec<u8>>; }`
- `struct SubprocessBackend;`（現状の実装）
- `struct MockBackend { stdout: Vec<u8> }`（test 用）

これにより `parse_porcelain_v2` だけでなく `git_info` 全体（timeout 分岐・exit code 分岐）が unit test 可能になる。Issue #29 を直接 close する。

### 4. CI に bench smoke test 追加

`cargo bench --no-run` で compile を verify する step を CI に追加。frequent regression 検知の自動化は `criterion-cmp` 等で将来検討するが、初期は compile-only で十分。

## Consequences

### Positive

- 性能 Issue は (a) 報告者に環境 + 測定値を要求 → (b) bench で fix 効果を検証 → (c) CI で regression 検知、の 3 段で守られる
- 「PR #48 で十分か」は実測で答えられるようになる（現状は推測）
- Issue #28, #29, #30, #31 が foundation の枠内で順次解決可能になる
- 設計記録（ADR）の最初の 1 件として、以後の構造変更の規範を作る

### Negative

- 初期工数: PERFORMANCE.md(~100 行) + criterion 導入 + bench 4 本 + trait 化 refactor + test = 推定 1〜2 日
- criterion を dev-dependency に追加（production binary には影響なし）
- bench fixture の保守責任が発生（巨大 repo fixture は CI 外で実行など工夫が必要）

### Neutral

- PR #48 のコードは変更しない（現状で性能契約を満たすか実測で評価する）
- 元 Issue #21/#22 の reporter に環境問い合わせを並走させるかは別判断

## Rollout

PR 構成（小さく分けて review しやすく）:

1. **PR-A**: `docs/PERFORMANCE.md` + 本 ADR を Accepted に昇格 → repo の規範として確立
2. **PR-B**: criterion 導入 + `benches/statusline.rs` 雛形（git_info + render_full の 2 本から開始）
3. **PR-C**: Issue #29 対応 — `GitBackend` trait 化 + Mock-based unit test 追加 → Issue #29 close
4. **PR-D**: CI に `cargo bench --no-run` 追加

PR-A をマージしてから PR-B 以降に進む。Issue #21/#22 の元 reporter への環境問い合わせは PR-A マージ後に並走で実施。

## Open Questions

- bench fixture（巨大 repo / NFS 環境）をどう用意するか — 初期は local SSD のみで、NFS / 巨大 repo は user 報告ベースで対応する案で start するか
- latency budget の初期値（p50 < 10ms 等）が現実的か — PR-B の bench 実装後に実測で校正する
