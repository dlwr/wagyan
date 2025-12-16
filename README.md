# Wagyan: 文字をSTL(ASCII)に押し出すCLI

## 使い方

```bash
# デフォルト組み込みフォント（Noto Sans JP Regular）で出力（標準出力）
cargo run -- --size 72 --depth 10 "HELLO" | pbcopy

# 日本語（例: Hiragino Sans）と改行
cargo run -- --font /System/Library/Fonts/ヒラギノ角ゴシック W4.ttc --size 72 --depth 5 "こんにちは\n世界"
```

オプション:
- `--font <path>`: 使用するフォントファイル（TTF/OTF）。省略時は組み込みの Noto Sans JP Regular。
- `--size <px>`: フォントサイズ。デフォルト `72`
- `--depth <value>`: 押し出し深さ。デフォルト `10`（Z方向を中心 ±depth/2 で押し出し）
- `--spacing <value>`: 文字間の追加スペース
- `--plate <value>`: 背面プレートの厚み（0で無効）。デフォルト `2`
- `--plate-margin <value>`: プレートを文字より広げるマージン。デフォルト `2`
- `--orient <flat|front>`: 配置面の向き。`front` は正面向き（XZ面）で、ビューアーを開いた直後に文字が読みやすい。`flat` は従来のXY平面。
- `--no-escape`: 入力中の `\\n` を改行に変換せず、そのまま出力する（デフォルトは `\\n` を改行に置換）
- `--no-center`: 自動センタリングを無効化（デフォルトはXY原点付近に平行移動）
- `--output <path>`: 指定した場合のみファイルへ出力（デフォルトは標準出力）

## メモ
- 複数行（`\n`）に対応。上から下へ行送りします。
- 日本語などマルチバイト文字もフォントにグリフがあれば出力可能です。
- 現在はカーニングなしの単純横書きです。
- フォントに含まれない文字はスキップされます。
- 同梱フォント: Noto Sans JP Regular（SIL Open Font License 1.1）。`assets/fonts/OFL.txt` を参照してください。
