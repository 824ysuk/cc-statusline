# statusline-rs

Claude Code 用の Rust 製ステータスライン。Node.js 版 claude-hud の置き換え。

## 表示例

通常ディレクトリ（🟢 Green zone、26%）:

```
cc-statusline on main
[Sonnet 4.6 | medium] │ Context ⣿⣿⣤⡀⡀⡀⡀⡀⡀⡀ 26% │ 5h ⣿⣿⡀⡀⡀⡀⡀⡀⡀⡀ 21% (resets in 2h 5m) │ 7d ⣿⣿⣿⡀⡀⡀⡀⡀⡀⡀ 31% (resets in 3d 1h 20m)
```

🟡 Yellow zone（53%）:

```
[Sonnet 4.6 | medium] │ Context ⣿⣿⣿⣿⣿⣀⡀⡀⡀⡀ 53%
```

🟠 Orange zone（69%）:

```
[Sonnet 4.6 | medium] │ Context ⣿⣿⣿⣿⣿⣿⣷⡀⡀⡀ 69%
```

🔴 Red zone（87%）:

```
[Sonnet 4.6 | medium] │ Context ⣿⣿⣿⣿⣿⣿⣿⣿⣦⡀ 87%
```

🟣 Magenta zone（101%+、compaction 超過）:

```
[Sonnet 4.6 | medium] │ Context ⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿ 101%
```

worktree 内（`.claude/worktrees/<name>/`）:

```
dotfiles > feat-my-feature on worktree-feat-my-feature*
[Sonnet 4.6 | medium] │ Context ⣿⣿⡀⡀⡀⡀⡀⡀⡀⡀ 20%
```

- Line 1: ディレクトリ（worktree 時は `repo > wt_name`）/ git ブランチ / dirty マーク `*`
- Line 2: `[モデル名 | effort]` │ Context バー │ 5h 使用率 │ 7d 使用率

## なぜ作ったか

- `claude-hud`（Node.js）は起動コストが高く、セキュリティ脆弱性も報告されていた
- 表示順を変えたかった（ディレクトリ/ブランチ → モデル名/effort）
- 依存を最小化したかった（`serde_json` のみ）

## インストール

```zsh
# Rust が未インストールの場合
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

git clone https://github.com/824ysuk/cc-statusline
cd cc-statusline
cargo install --path .
```

`~/.cargo/bin/statusline-rs` にインストールされます。rustup の標準設定では `~/.cargo/bin` が PATH に含まれるため、そのまま `statusline-rs` コマンドとして呼び出せます。

## セットアップ

Claude Code の `StatusLine` フックとして登録します。`.claude/settings.json` に追加してください:

```json
{
  "hooks": {
    "StatusLine": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "statusline-rs"
          }
        ]
      }
    ]
  }
}
```

バイナリは Claude Code から JSON を stdin で受け取り、ステータスライン文字列を stdout に出力します。

### 開発時のバイナリ切り替え

`cargo install` せずに手動ビルド（`cargo build --release`）のバイナリを使いたい場合は `STATUSLINE_RS_BIN` 環境変数で上書きできます:

```zsh
export STATUSLINE_RS_BIN="$HOME/Projects/cc-statusline/target/release/statusline-rs"
```

## context カラーゾーン（claudeline 準拠）

| 使用率 | 色 | 意味 |
|---|---|---|
| 0–40% | 🟢 Green | Smart zone（フル性能） |
| 41–60% | 🟡 Yellow | Dumb zone（品質低下開始） |
| 61–80% | 🟠 Orange | Danger zone |
| 81–100% | 🔴 Red | Near compaction |
| 101%+ | 🟣 Magenta | Compaction 超過 |
