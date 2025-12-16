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
use ttf_parser::{Face, OutlineBuilder};

const EMBEDDED_FONT: &[u8] = include_bytes!("../assets/fonts/NotoSansJP-Regular.otf");

/// Simple CLI that extrudes text into an ASCII STL
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Text to render
    text: String,
    /// Font file (.ttf/.otf). Falls back to embedded Noto Sans JP Regular
    #[arg(short, long)]
    font: Option<PathBuf>,
    /// Font size (px-ish units)
    #[arg(long, default_value_t = 72.0)]
    size: f32,
    /// Extrusion depth (same units as layout)
    #[arg(long, default_value_t = 10.0)]
    depth: f32,
    /// Additional spacing between glyphs
    #[arg(long, default_value_t = 0.0)]
    spacing: f32,
    /// Plane orientation (flat: XY floor, front: XZ facing viewer)
    #[arg(long, value_enum, default_value_t = Orientation::Front)]
    orient: Orientation,
    /// Keep literal "\n" (do not convert to newline)
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
    let face = Face::parse(&font_bytes, 0).context("failed to parse font")?;

    // Unit conversion
    let units_per_em = face.units_per_em() as f32;
    let scale = args.size / units_per_em;
    let baseline_y = face.ascender() as f32 * scale;

    // Convert literal "\n" to newline unless disabled
    let text = if args.no_escape {
        args.text.clone()
    } else {
        args.text.replace("\\n", "\n")
    };

    // Build a single path from all glyph outlines
    let mut path_builder = Path::builder();
    layout_text_to_path(
        &face,
        &mut path_builder,
        &text,
        scale,
        baseline_y,
        args.spacing,
    )?;
    let path = path_builder.build();

    // Tessellate and extrude
    let mut mesh = tessellate_path(&path)?;
    if !args.no_center {
        center_mesh_xy(&mut mesh);
    }
    let triangles = extrude_mesh(&mesh, args.depth, args.orient.clone());

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

/// Simple left-to-right layout; collects glyph outlines into a path
fn layout_text_to_path(
    face: &Face<'_>,
    builder: &mut PathBuilder,
    text: &str,
    scale: f32,
    baseline_y: f32,
    spacing: f32,
) -> Result<()> {
    let mut pen_x = 0.0;
    let mut pen_baseline = baseline_y;
    let line_advance = face.height() as f32 * scale;

    for ch in text.chars() {
        if ch == '\n' {
            pen_x = 0.0;
            pen_baseline -= line_advance;
            continue;
        }

        let gid = match face.glyph_index(ch) {
            Some(id) => id,
            None => {
                eprintln!("⚠️ Skip missing glyph: '{}'", ch);
                continue;
            }
        };

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

fn tessellate_path(path: &Path) -> Result<Mesh2D> {
    let mut buffers: VertexBuffers<Point, u16> = VertexBuffers::new();
    let mut tess = FillTessellator::new();
    tess.tessellate_path(
        path,
        &FillOptions::default()
            .with_fill_rule(FillRule::NonZero)
            .with_tolerance(0.01),
        &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| v.position()),
    )
    .context("failed to tessellate polygon")?;

    Ok(Mesh2D {
        vertices: buffers.vertices,
        indices: buffers.indices,
    })
}

fn extrude_mesh(mesh: &Mesh2D, depth: f32, orient: Orientation) -> Vec<Triangle> {
    let mut triangles = Vec::new();
    let z0 = -depth * 0.5;
    let z1 = depth * 0.5;

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
