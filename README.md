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

Claude Code の `statusLine` トップレベルキーとして登録します。`~/.claude/settings.json` に追加してください:

```json
{
  "statusLine": {
    "type": "command",
    "command": "statusline-rs"
  }
}
```

バイナリは Claude Code から JSON を stdin で受け取り、ステータスライン文字列を stdout に出力します。

### 開発時のバイナリ切り替え

`cargo install` せずに手動ビルド（`cargo build --release`）のバイナリを使いたい場合は `STATUSLINE_RS_BIN` 環境変数で上書きできます:

```zsh
export STATUSLINE_RS_BIN="$HOME/Projects/cc-statusline/target/release/statusline-rs"
```

## stdin JSON スキーマ

Claude Code はターンごとに以下の JSON を stdin に送信します（実測: Claude Code 2.1.169）。

### statusline-rs が使用するフィールド

| フィールド | 型 | 説明 |
|---|---|---|
| `cwd` | `string` | 現在のワーキングディレクトリ |
| `workspace.current_dir` | `string` | ワークスペースのカレントディレクトリ（`cwd` より優先） |
| `model.display_name` | `string?` | モデル表示名（例: `"Sonnet 4.6"`） |
| `model.id` | `string?` | モデル ID（`display_name` がない場合のフォールバック） |
| `effort` | `string \| {level: string} \| null` | 思考努力レベル（例: `"medium"` または `{"level": "high"}`） |
| `context_window.context_window_size` | `number?` | コンテキストウィンドウサイズ（tokens） |
| `context_window.used_percentage` | `number?` | 使用率 0–100（0.0 は未計測として token 合計で代替計算） |
| `context_window.current_usage.input_tokens` | `number?` | 入力トークン数 |
| `context_window.current_usage.cache_creation_input_tokens` | `number?` | キャッシュ作成トークン数 |
| `context_window.current_usage.cache_read_input_tokens` | `number?` | キャッシュ読み取りトークン数 |
| `rate_limits.five_hour.used_percentage` | `number?` | 5時間レートリミット使用率 |
| `rate_limits.five_hour.resets_at` | `number?` | リセット時刻（Unix timestamp） |
| `rate_limits.seven_day.used_percentage` | `number?` | 7日間レートリミット使用率 |
| `rate_limits.seven_day.resets_at` | `number?` | リセット時刻（Unix timestamp） |

### Claude Code が送信する全フィールド（未使用含む）

```json
{
  "session_id": "uuid",
  "transcript_path": "/path/to/session.jsonl",
  "session_name": "セッション名",
  "version": "2.1.169",
  "cwd": "/current/dir",
  "effort": { "level": "high" },
  "model": { "id": "claude-sonnet-4-6", "display_name": "Sonnet 4.6" },
  "workspace": {
    "current_dir": "/current/dir",
    "project_dir": "/project/dir",
    "added_dirs": ["/tmp"],
    "repo": { "host": "github.com", "owner": "824ysuk", "name": "cc-statusline" }
  },
  "output_style": { "name": "default" },
  "fast_mode": false,
  "thinking": { "enabled": true },
  "exceeds_200k_tokens": false,
  "cost": {
    "total_cost_usd": 2.88,
    "total_duration_ms": 3572098,
    "total_api_duration_ms": 1149951,
    "total_lines_added": 7,
    "total_lines_removed": 0
  },
  "context_window": {
    "context_window_size": 200000,
    "used_percentage": 72,
    "remaining_percentage": 28,
    "total_input_tokens": 143071,
    "total_output_tokens": 8,
    "current_usage": {
      "input_tokens": 3,
      "output_tokens": 8,
      "cache_creation_input_tokens": 582,
      "cache_read_input_tokens": 142486
    }
  },
  "rate_limits": {
    "five_hour": { "used_percentage": 15, "resets_at": 1780999800 },
    "seven_day": { "used_percentage": 28, "resets_at": 1781438400 }
  }
}
```

> **Note**: `pr` / `pr.url` フィールドは feature ブランチ + open PR が存在するときのみ出現する可能性があります（`main` ブランチ上では未観測）。serde の `deny_unknown_fields` を使用していないため、未知フィールドは黙って無視されます。

## context カラーゾーン（claudeline 準拠）

| 使用率 | 色 | 意味 |
|---|---|---|
| 0–40% | 🟢 Green | Smart zone（フル性能） |
| 41–60% | 🟡 Yellow | Dumb zone（品質低下開始） |
| 61–80% | 🟠 Orange | Danger zone |
| 81–100% | 🔴 Red | Near compaction |
| 101%+ | 🟣 Magenta | Compaction 超過 |
