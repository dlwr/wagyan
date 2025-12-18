use std::error::Error;
use std::str::FromStr;

#[test]
fn cli_outputs_stl_with_depth_and_unit_normals() -> Result<(), Box<dyn Error>> {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("wagyan");
    let assert = cmd
        .args([
            "--size",
            "32",
            "--depth",
            "2",
            "--plate",
            "0",
            "--tolerance",
            "0.05",
            "--orient",
            "flat",
            "--no-center",
            "A",
        ])
        .assert()
        .success();

    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.starts_with("solid "), "missing STL header");
    assert!(
        stdout.trim_end().ends_with("endsolid mesh"),
        "missing STL footer"
    );

    let mut facet_count = 0usize;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;

    for line in stdout.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("facet normal") {
            facet_count += 1;
            let parts: Vec<_> = rest.split_whitespace().collect();
            if parts.len() == 3 {
                let nx = f32::from_str(parts[0]).unwrap_or(0.0);
                let ny = f32::from_str(parts[1]).unwrap_or(0.0);
                let nz = f32::from_str(parts[2]).unwrap_or(0.0);
                let len = (nx * nx + ny * ny + nz * nz).sqrt();
                assert!(
                    (0.9..=1.1).contains(&len),
                    "normal length out of range: {} in {}",
                    len,
                    trimmed
                );
            }
        } else if let Some(rest) = trimmed.strip_prefix("vertex") {
            let parts: Vec<_> = rest.split_whitespace().collect();
            if parts.len() == 3 {
                let z = f32::from_str(parts[2]).unwrap_or(0.0);
                min_z = min_z.min(z);
                max_z = max_z.max(z);
            }
        }
    }

    assert!(facet_count > 0, "no facets were emitted");
    assert!(min_z.is_finite() && max_z.is_finite(), "no vertices parsed");
    assert!(
        (max_z - 1.0).abs() < 1e-2,
        "unexpected top z: {} (expected ~1.0)",
        max_z
    );
    assert!(
        (min_z + 1.0).abs() < 1e-2,
        "unexpected bottom z: {} (expected ~-1.0)",
        min_z
    );

    Ok(())
}
