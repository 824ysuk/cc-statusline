# statusline-rs

Claude Code 用の Rust 製ステータスライン。Node.js 版 claude-hud の置き換え。

## 表示例

通常ディレクトリ:

```
cc-statusline on main
[Sonnet 4.6 | medium] │ Context ⣿⣿⣿⣿⡀⡀⡀⡀⡀⡀ 42% │ 5h ⣿⣿⡀⡀⡀⡀⡀⡀⡀⡀ 21% (resets in 2h 5m) │ 7d ⣿⣿⣿⡀⡀⡀⡀⡀⡀⡀ 31% (resets in 3d 1h 20m)
```

worktree 内（`.claude/worktrees/<name>/`）:

```
dotfiles > feat-my-feature on feat-my-feature*
[Sonnet 4.6 | medium] │ Context ⣿⣿⡀⡀⡀⡀⡀⡀⡀⡀ 20%
```

- Line 1: ディレクトリ（worktree 時は `repo > wt_name`）/ git ブランチ / dirty マーク `*`
- Line 2: モデル名 / effort │ Context バー │ 5h 使用率 │ 7d 使用率

## なぜ作ったか

- `claude-hud`（Node.js）は起動コストが高く、セキュリティ脆弱性も報告されていた
- 表示順を変えたかった（ディレクトリ/ブランチ → モデル名/effort）
- 依存を最小化したかった（`serde_json` のみ）

## ビルド

```zsh
# Rust が未インストールの場合
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# ビルド
bash ~/dotfiles/statusline-rs/build.sh
```

## セットアップ

ビルド後、`statusline-with-pr.sh` が自動的にこのバイナリを検出して使います。
バイナリが存在しない場合はステータスラインが表示されません。必ずビルドを実行してください。

## context カラーゾーン（claudeline 準拠）

| 使用率 | 色 | 意味 |
|---|---|---|
| 0–40% | 🟢 Green | Smart zone（フル性能） |
| 41–60% | 🟡 Yellow | Dumb zone（品質低下開始） |
| 61–80% | 🟠 Orange | Danger zone |
| 81%+ | 🔴 Red | Near compaction |
