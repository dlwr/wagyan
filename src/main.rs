use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use lyon_path::math::Point;
use lyon_path::path::Builder as PathBuilder;
use lyon_path::Path;
use lyon_tessellation::geometry_builder::VertexBuffers;
use lyon_tessellation::{BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex};
use stl_io::Triangle;
use ttf_parser::{Face, GlyphId, OutlineBuilder};

const EMBEDDED_FONT: &[u8] = include_bytes!("../assets/fonts/NotoSansJP-Regular.otf");
const DEFAULT_TOLERANCE: f32 = 0.01;
const DEFAULT_TOLERANCE_SIZE: f32 = 72.0;
const MIN_TOLERANCE: f32 = 0.0005;
const MAX_TOLERANCE: f32 = 0.2;

/// Simple CLI that extrudes text into an ASCII STL
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Text to render
    text: String,
    /// Font file (.ttf/.otf). Falls back to embedded Noto Sans JP Regular
    #[arg(short, long)]
    font: Option<PathBuf>,
    /// Face index for font collections (.ttc). 0-based.
    #[arg(long, default_value_t = 0)]
    face_index: u32,
    /// Font size (px-ish units)
    #[arg(long, default_value_t = 72.0)]
    size: f32,
    /// Tessellation tolerance (smaller = finer). Default scales with --size.
    #[arg(long)]
    tolerance: Option<f32>,
    /// Extrusion depth (same units as layout)
    #[arg(long, default_value_t = 10.0)]
    depth: f32,
    /// Additional spacing between glyphs
    #[arg(long, default_value_t = 0.0)]
    spacing: f32,
    /// Apply kerning when available (disable with --no-kerning)
    #[arg(long, default_value_t = true, action = clap::ArgAction::SetTrue, conflicts_with = "no_kerning")]
    kerning: bool,
    /// Disable kerning adjustments
    #[arg(long = "no-kerning", action = clap::ArgAction::SetTrue, conflicts_with = "kerning")]
    no_kerning: bool,
    /// Back plate thickness (0 disables)
    #[arg(long, default_value_t = 0.0)]
    plate: f32,
    /// Margin to expand the plate
    #[arg(long, default_value_t = 2.0)]
    plate_margin: f32,
    /// Plane orientation (flat: XY floor, front: XZ facing viewer)
    #[arg(long, value_enum, default_value_t = Orientation::Front)]
    orient: Orientation,
    /// Keep literal "\\n" (do not convert to newline)
    #[arg(long)]
    no_escape: bool,
    /// Disable auto-centering to origin
    #[arg(long)]
    no_center: bool,
    /// Output file (stdout by default)
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum Orientation {
    Flat,
    Front,
}

fn resolve_tolerance(size: f32, cli_value: Option<f32>) -> f32 {
    let scaled = DEFAULT_TOLERANCE * (size / DEFAULT_TOLERANCE_SIZE);
    let value = cli_value.unwrap_or(scaled);
    value.clamp(MIN_TOLERANCE, MAX_TOLERANCE)
}

fn main() -> Result<()> {
    let args = Args::parse();
    run(args).context("conversion failed")
}

fn run(args: Args) -> Result<()> {
    // Load font (fallback to embedded Noto Sans JP Regular)
    let font_bytes: Cow<[u8]> = if let Some(path) = args.font.as_ref() {
        Cow::Owned(
            fs::read(path)
                .with_context(|| format!("failed to read font file: {}", path.display()))?,
        )
    } else {
        Cow::Borrowed(EMBEDDED_FONT)
    };

    let face_count = ttf_parser::fonts_in_collection(&font_bytes).unwrap_or(1);
    anyhow::ensure!(face_count > 0, "font file appears to have no faces");
    anyhow::ensure!(
        args.face_index < face_count,
        "face index {} is out of range (available 0..={}; font has {} face{})",
        args.face_index,
        face_count - 1,
        face_count,
        if face_count == 1 { "" } else { "s" },
    );

    let face = Face::parse(&font_bytes, args.face_index)
        .with_context(|| format!("failed to parse font (face index {})", args.face_index))?;

    // Unit conversion
    let units_per_em = face.units_per_em() as f32;
    let scale = args.size / units_per_em;
    let baseline_y = face.ascender() as f32 * scale;
    let tolerance = resolve_tolerance(args.size, args.tolerance);

    // Convert literal "\\n" to newline unless disabled
    let text = if args.no_escape {
        args.text.clone()
    } else {
        args.text.replace("\\n", "\n")
    };

    let kerning = if args.no_kerning { false } else { args.kerning };

    // Build a single path from all glyph outlines
    let mut path_builder = Path::builder();
    layout_text_to_path(
        &face,
        &mut path_builder,
        &text,
        scale,
        baseline_y,
        args.spacing,
        kerning,
    )?;
    let path = path_builder.build();

    // Tessellate and extrude
    let mut mesh = tessellate_path(&path, tolerance)?;
    if !args.no_center {
        center_mesh_xy(&mut mesh);
    }

    let mut triangles = Vec::new();

    if args.plate > 0.0 {
        if let Some((min_x, max_x, min_y, max_y)) = mesh_bounds(&mesh) {
            let plate_mesh = rectangle_mesh(
                min_x - args.plate_margin,
                max_x + args.plate_margin,
                min_y - args.plate_margin,
                max_y + args.plate_margin,
            );
            let plate_offset = -(args.depth * 0.5 + args.plate * 0.5);
            triangles.extend(extrude_mesh_with_offset(
                &plate_mesh,
                args.plate,
                args.orient.clone(),
                plate_offset,
            ));
        }
    }

    triangles.extend(extrude_mesh(&mesh, args.depth, args.orient.clone()));

    // Write STL: default to stdout, file when --output is set
    if let Some(path) = args.output.as_ref() {
        write_stl_ascii(path, &triangles)
            .with_context(|| format!("failed to write ASCII STL: {}", path.display()))?;
        println!("✅ wrote: {}", path.display());
    } else {
        let mut out = BufWriter::new(std::io::stdout().lock());
        write_stl_ascii_to_writer(&mut out, "mesh", &triangles)
            .context("failed to write ASCII STL to stdout")?;
    }
    Ok(())
}

fn kerning_value(face: &Face<'_>, left: GlyphId, right: GlyphId) -> Option<i16> {
    let kern = face.tables().kern.as_ref()?;
    for subtable in kern.subtables.into_iter() {
        if !subtable.horizontal || subtable.has_cross_stream || subtable.has_state_machine {
            continue;
        }
        if let Some(value) = subtable.glyphs_kerning(left, right) {
            return Some(value);
        }
    }
    None
}

/// Simple left-to-right layout; collects glyph outlines into a path
fn layout_text_to_path(
    face: &Face<'_>,
    builder: &mut PathBuilder,
    text: &str,
    scale: f32,
    baseline_y: f32,
    spacing: f32,
    kerning: bool,
) -> Result<()> {
    let mut pen_x = 0.0;
    let mut pen_baseline = baseline_y;
    let line_advance = face.height() as f32 * scale;
    let mut prev_gid = None;

    for ch in text.chars() {
        if ch == '\n' {
            pen_x = 0.0;
            pen_baseline -= line_advance;
            prev_gid = None;
            continue;
        }

        let gid = match face.glyph_index(ch) {
            Some(id) => id,
            None => {
                eprintln!("⚠️ Skip missing glyph: '{}'", ch);
                continue;
            }
        };

        // Apply kerning relative to previous glyph when available
        if kerning {
            if let Some(prev) = prev_gid {
                if let Some(kern) = kerning_value(face, prev, gid) {
                    pen_x += kern as f32 * scale;
                }
            }
        }

        // Add outline to path
        let mut adapter = LyonOutlineBuilder {
            builder,
            offset_x: pen_x,
            offset_y: pen_baseline,
            scale,
        };
        face.outline_glyph(gid, &mut adapter)
            .ok_or_else(|| anyhow::anyhow!("failed to get outline for '{}'", ch))?;

        // Advance: glyph advance + spacing
        let advance = face.glyph_hor_advance(gid).unwrap_or(0) as f32 * scale + spacing;
        pen_x += advance;
        prev_gid = Some(gid);
    }

    Ok(())
}

/// Adapter: ttf-parser outline -> lyon PathBuilder
struct LyonOutlineBuilder<'a> {
    builder: &'a mut PathBuilder,
    offset_x: f32,
    offset_y: f32,
    scale: f32,
}

impl OutlineBuilder for LyonOutlineBuilder<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.begin(Point::new(
            x * self.scale + self.offset_x,
            y * self.scale + self.offset_y,
        ));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(Point::new(
            x * self.scale + self.offset_x,
            y * self.scale + self.offset_y,
        ));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.builder.quadratic_bezier_to(
            Point::new(
                x1 * self.scale + self.offset_x,
                y1 * self.scale + self.offset_y,
            ),
            Point::new(
                x * self.scale + self.offset_x,
                y * self.scale + self.offset_y,
            ),
        );
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.builder.cubic_bezier_to(
            Point::new(
                x1 * self.scale + self.offset_x,
                y1 * self.scale + self.offset_y,
            ),
            Point::new(
                x2 * self.scale + self.offset_x,
                y2 * self.scale + self.offset_y,
            ),
            Point::new(
                x * self.scale + self.offset_x,
                y * self.scale + self.offset_y,
            ),
        );
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

struct Mesh2D {
    vertices: Vec<Point>,
    indices: Vec<u16>,
}

fn center_mesh_xy(mesh: &mut Mesh2D) {
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;

    for p in &mesh.vertices {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }

    let cx = (min_x + max_x) * 0.5;
    let cy = (min_y + max_y) * 0.5;

    for p in &mut mesh.vertices {
        p.x -= cx;
        p.y -= cy;
    }
}

fn mesh_bounds(mesh: &Mesh2D) -> Option<(f32, f32, f32, f32)> {
    if mesh.vertices.is_empty() {
        return None;
    }

    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;

    for p in &mesh.vertices {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }

    Some((min_x, max_x, min_y, max_y))
}

fn rectangle_mesh(min_x: f32, max_x: f32, min_y: f32, max_y: f32) -> Mesh2D {
    Mesh2D {
        vertices: vec![
            Point::new(min_x, min_y),
            Point::new(max_x, min_y),
            Point::new(max_x, max_y),
            Point::new(min_x, max_y),
        ],
        indices: vec![0u16, 1, 2, 0, 2, 3],
    }
}

fn tessellate_path(path: &Path, tolerance: f32) -> Result<Mesh2D> {
    let mut buffers: VertexBuffers<Point, u16> = VertexBuffers::new();
    let mut tess = FillTessellator::new();
    tess.tessellate_path(
        path,
        &FillOptions::default()
            .with_fill_rule(FillRule::NonZero)
            .with_tolerance(tolerance),
        &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| v.position()),
    )
    .context("failed to tessellate polygon")?;

    Ok(Mesh2D {
        vertices: buffers.vertices,
        indices: buffers.indices,
    })
}

fn extrude_mesh_with_offset(
    mesh: &Mesh2D,
    depth: f32,
    orient: Orientation,
    z_offset: f32,
) -> Vec<Triangle> {
    let mut triangles = Vec::new();
    let z0 = -depth * 0.5 + z_offset;
    let z1 = depth * 0.5 + z_offset;

    // Top face
    for idx in mesh.indices.chunks(3) {
        let a = mesh.vertices[idx[0] as usize];
        let b = mesh.vertices[idx[1] as usize];
        let c = mesh.vertices[idx[2] as usize];
        triangles.push(triangle_with_normal(
            map_point(a, z1, &orient),
            map_point(b, z1, &orient),
            map_point(c, z1, &orient),
        ));
    }

    // Bottom face (reverse winding so normal points down)
    for idx in mesh.indices.chunks(3) {
        let a = mesh.vertices[idx[0] as usize];
        let b = mesh.vertices[idx[1] as usize];
        let c = mesh.vertices[idx[2] as usize];
        triangles.push(triangle_with_normal(
            map_point(c, z0, &orient),
            map_point(b, z0, &orient),
            map_point(a, z0, &orient),
        ));
    }

    // Side faces: detect boundary edges, create quads -> two triangles
    for (i0, i1) in boundary_edges(&mesh.indices) {
        let p0 = mesh.vertices[i0 as usize];
        let p1 = mesh.vertices[i1 as usize];

        let top0 = map_point(p0, z1, &orient);
        let top1 = map_point(p1, z1, &orient);
        let bot0 = map_point(p0, z0, &orient);
        let bot1 = map_point(p1, z0, &orient);

        triangles.push(triangle_with_normal(top0, top1, bot1));
        triangles.push(triangle_with_normal(top0, bot1, bot0));
    }

    triangles
}

fn extrude_mesh(mesh: &Mesh2D, depth: f32, orient: Orientation) -> Vec<Triangle> {
    extrude_mesh_with_offset(mesh, depth, orient, 0.0)
}

/// Return boundary edges (true = edge orientation matches triangle winding)
fn boundary_edges(indices: &[u16]) -> Vec<(u16, u16)> {
    let mut counts: HashMap<(u16, u16), u32> = HashMap::new();
    let mut oriented: HashMap<(u16, u16), (u16, u16)> = HashMap::new();

    for tri in indices.chunks(3) {
        let edges = [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])];
        for &(a, b) in &edges {
            let key = if a < b { (a, b) } else { (b, a) };
            *counts.entry(key).or_insert(0) += 1;
            oriented.entry(key).or_insert((a, b));
        }
    }

    counts
        .into_iter()
        .filter(|(_, cnt)| *cnt == 1)
        .map(|(k, _)| oriented[&k])
        .collect()
}

fn triangle_with_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Triangle {
    Triangle {
        normal: calc_normal(a, b, c),
        vertices: [a, b, c],
    }
}

fn calc_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len == 0.0 {
        [0.0, 0.0, 0.0]
    } else {
        [n[0] / len, n[1] / len, n[2] / len]
    }
}

fn map_point(p: Point, z: f32, orient: &Orientation) -> [f32; 3] {
    match orient {
        Orientation::Flat => [p.x, p.y, z],
        // Front orientation: keep X, rotate +Z to up, +Y faces viewer
        // (original +Z normals become +Y; text keeps its vertical sense)
        Orientation::Front => [p.x, -z, p.y],
    }
}

fn write_stl_ascii(path: &PathBuf, tris: &[Triangle]) -> Result<()> {
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("mesh");
    let file = File::create(path)?;
    let buf = BufWriter::new(file);
    write_stl_ascii_to_writer(buf, name, tris)
}

fn write_stl_ascii_to_writer<W: Write>(mut writer: W, name: &str, tris: &[Triangle]) -> Result<()> {
    writeln!(writer, "solid {}", name)?;
    for tri in tris {
        writeln!(
            writer,
            "  facet normal {} {} {}",
            tri.normal[0], tri.normal[1], tri.normal[2]
        )?;
        writeln!(writer, "    outer loop")?;
        for v in &tri.vertices {
            writeln!(writer, "      vertex {} {} {}", v[0], v[1], v[2])?;
        }
        writeln!(writer, "    endloop")?;
        writeln!(writer, "  endfacet")?;
    }
    writeln!(writer, "endsolid {}", name)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_edges_filters_shared_edges() {
        let indices = vec![0u16, 1, 2, 2, 1, 3];
        let edges: std::collections::HashSet<(u16, u16)> =
            boundary_edges(&indices).into_iter().collect();

        let expected: std::collections::HashSet<(u16, u16)> =
            [(0, 1), (2, 0), (3, 2), (1, 3)].into_iter().collect();

        assert_eq!(edges, expected);
    }

    #[test]
    fn calc_normal_returns_unit_z_for_xy_triangles() {
        let n = calc_normal([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert!((n[0]).abs() < 1e-6);
        assert!((n[1]).abs() < 1e-6);
        assert!((n[2] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn calc_normal_handles_degenerate_triangles() {
        let n = calc_normal([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [2.0, 2.0, 2.0]);
        assert_eq!(n, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn tolerance_scales_with_size() {
        let base = resolve_tolerance(72.0, None);
        let bigger = resolve_tolerance(144.0, None);
        let smaller = resolve_tolerance(24.0, None);

        assert!(bigger > base);
        assert!(smaller < base);
    }

    #[test]
    fn tolerance_is_clamped() {
        let min = resolve_tolerance(1.0, Some(0.00001));
        let max = resolve_tolerance(10_000.0, Some(10.0));

        assert_eq!(min, MIN_TOLERANCE);
        assert_eq!(max, MAX_TOLERANCE);
    }
}
