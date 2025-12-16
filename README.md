# Wagyan: Text → ASCII STL CLI

## Usage

```bash
# Default embedded font (Noto Sans JP Regular), write to stdout
cargo run -- --size 72 --depth 10 "HELLO" | pbcopy

# Japanese + newline with external font
cargo run -- --font "/System/Library/Fonts/ヒラギノ角ゴシック W4.ttc" --size 72 --depth 5 "こんにちは\n世界"
```

Options:
- `--font <path>`: TTF/OTF font. Defaults to embedded Noto Sans JP Regular.
- `--size <px>`: Font size. Default `72`.
- `--depth <value>`: Extrusion depth (centered ±depth/2). Default `10`.
- `--spacing <value>`: Extra spacing between glyphs.
- `--plate <value>`: Back plate thickness (0 disables). Default `2`.
- `--plate-margin <value>`: Margin to expand the plate. Default `2`.
- `--orient <flat|front>`: Text plane (front faces the viewer on XZ; flat is XY).
- `--no-escape`: Keep literal `\\n` (no newline conversion). Default converts `\\n` to newline.
- `--no-center`: Disable auto-centering to origin.
- `--output <path>`: Write to a file; otherwise stdout.

## Notes
- Supports multiline (`\n`), rendered top to bottom.
- Any script is OK as long as the font has glyphs.
- Simple LTR layout; kerning is not applied.
- Missing glyphs are skipped with a warning.
- Bundled font: Noto Sans JP Regular (SIL Open Font License 1.1). See `assets/fonts/OFL.txt`.
