use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let frontend_dir = manifest_dir.join("frontend");
    let dist_dir = frontend_dir.join("dist");

    println!("cargo:rerun-if-changed={}", dist_dir.display());
    println!(
        "cargo:rerun-if-changed={}",
        frontend_dir.join("src").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        frontend_dir.join("index.html").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        frontend_dir.join("package.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        frontend_dir.join("pnpm-lock.yaml").display()
    );

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let output = out_dir.join("frontend_assets.rs");
    let mut assets = Vec::new();
    if dist_dir.is_dir() {
        collect_assets(&dist_dir, &dist_dir, &mut assets)
            .expect("failed to collect frontend/dist assets");
    }
    assets.sort_by(|left, right| left.0.cmp(&right.0));

    if assets.is_empty() {
        println!(
            "cargo:warning=frontend/dist is missing or empty; embedded frontend assets will be unavailable"
        );
    }

    let mut file = fs::File::create(output).expect("failed to write generated frontend assets");
    writeln!(
        file,
        "pub static FRONTEND_ASSETS: &[crate::frontend_assets::EmbeddedAsset] = &["
    )
    .unwrap();
    for (relative_path, absolute_path) in assets {
        writeln!(
            file,
            "    crate::frontend_assets::EmbeddedAsset {{ path: {:?}, bytes: include_bytes!({:?}), mime: {:?} }},",
            relative_path,
            absolute_path.display().to_string(),
            mime_for_path(&relative_path),
        )
        .unwrap();
    }
    writeln!(file, "];").unwrap();
}

fn collect_assets(
    root: &Path,
    current: &Path,
    assets: &mut Vec<(String, PathBuf)>,
) -> io::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_assets(root, &path, assets)?;
        } else if path.is_file() {
            let relative_path = path
                .strip_prefix(root)
                .unwrap()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            assets.push((relative_path, path));
        }
    }
    Ok(())
}

fn mime_for_path(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
    {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "wasm" => "application/wasm",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}
