# Wagyan: Text → ASCII STL CLI

Pronunciation: ワギャン (IPA:[waɡʲaɴ])

## Installation

### Homebrew

```bash
brew install dlwr/wagyan/wagyan
```

### Cargo

```bash
cargo install wagyan
```

## Usage

```bash
# Default embedded font (Noto Sans JP Regular), write to stdout
wagyan --size 72 --depth 10 "HELLO" | pbcopy

# Japanese + newline with external font
wagyan --font "/System/Library/Fonts/ヒラギノ角ゴシック W4.ttc" --size 72 --depth 5 "こんにちは\n世界"
```

Options:
- `--font <path>`: TTF/OTF font. Defaults to embedded Noto Sans JP Regular.
- `--face-index <n>`: Face index for font collections (.ttc). Default `0`.
- `--size <px>`: Font size. Default `72`.
- `--tolerance <value>`: Tessellation tolerance (smaller = finer). Defaults to size-proportional value (clamped between `0.0005` and `0.2`).
- `--depth <value>`: Extrusion depth (centered ±depth/2). Default `10`.
- `--spacing <value>`: Extra spacing between glyphs.
- `--kerning` / `--no-kerning`: Toggle kerning (default on when font provides pairs).
- `--plate <value>`: Back plate thickness (0 disables). Default `0`.
- `--plate-margin <value>`: Margin to expand the plate. Default `2`.
- `--orient <flat|front>`: Text plane (front faces the viewer on XZ; flat is XY).
- `--no-escape`: Keep literal `\\n` (no newline conversion). Default converts `\\n` to newline.
- `--no-center`: Disable auto-centering to origin.
- `--output <path>`: Write to a file; otherwise stdout.

## Notes
- Supports multiline (`\n`), rendered top to bottom.
- Default tessellation tolerance scales with `--size` to keep detail level consistent across sizes.
- Any script is OK as long as the font has glyphs.
- Simple LTR layout; kerning is applied when provided by the font (disable with `--no-kerning`).
- Missing glyphs are skipped with a warning.
- Bundled font: Noto Sans JP Regular (SIL Open Font License 1.1). See `assets/fonts/OFL.txt`.
- Project license: MIT (see `LICENSE`).
