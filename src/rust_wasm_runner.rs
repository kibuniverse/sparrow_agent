use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

const COMPILE_TIMEOUT: Duration = Duration::from_secs(10);
const OUTPUT_LIMIT_BYTES: usize = 64 * 1024;
const INITIAL_FUEL: u64 = 1_000_000;
const STDERR_LIMIT: usize = 16 * 1024;
const WASM_TARGET: &str = "wasm32-unknown-unknown";

const CARGO_TOML: &str = r#"[package]
name = "generated_wasm_run"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[profile.release]
opt-level = "z"
lto = true
"#;

const LIB_RS: &str = r#"mod user_code {
    include!("user_code.rs");
}

static mut RESULT_PTR: *const u8 = core::ptr::null();
static mut RESULT_LEN: usize = 0;

#[unsafe(no_mangle)]
pub extern "C" fn run() {
    let result = user_code::run();
    let leaked = Box::leak(result.into_boxed_str());

    unsafe {
        RESULT_PTR = leaked.as_ptr();
        RESULT_LEN = leaked.len();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn result_ptr() -> usize {
    unsafe { RESULT_PTR as usize }
}

#[unsafe(no_mangle)]
pub extern "C" fn result_len() -> usize {
    unsafe { RESULT_LEN }
}
"#;

pub async fn run_generated_rust(code: &str) -> Result<String> {
    let temp_dir = tempfile::tempdir().context("failed to create temp directory")?;
    let src_dir = temp_dir.path().join("src");
    std::fs::create_dir_all(&src_dir).context("failed to create src directory")?;

    std::fs::write(temp_dir.path().join("Cargo.toml"), CARGO_TOML)
        .context("failed to write Cargo.toml")?;
    std::fs::write(src_dir.join("lib.rs"), LIB_RS).context("failed to write lib.rs")?;
    std::fs::write(src_dir.join("user_code.rs"), code).context("failed to write user_code.rs")?;

    let wasm_path = compile_wasm(temp_dir.path()).await?;
    let output = execute_wasm(&wasm_path)?;

    Ok(output)
}

async fn compile_wasm(project_dir: &std::path::Path) -> Result<std::path::PathBuf> {
    let output = tokio::time::timeout(
        COMPILE_TIMEOUT,
        Command::new("cargo")
            .args(["build", "--release", "--target", WASM_TARGET])
            .current_dir(project_dir)
            .output(),
    )
    .await
    .context("compilation timed out")?
    .context("failed to execute cargo build")?;

    if output.status.success() {
        let wasm_path = project_dir
            .join("target")
            .join(WASM_TARGET)
            .join("release")
            .join("generated_wasm_run.wasm");

        if wasm_path.exists() {
            return Ok(wasm_path);
        }

        bail!(
            "compilation succeeded but wasm file not found at {}",
            wasm_path.display()
        );
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let trimmed = truncate_str(&stderr, STDERR_LIMIT);

    if stderr.contains("can't find crate for `std`") || stderr.contains(WASM_TARGET) {
        bail!(
            "Compilation failed: {WASM_TARGET} target is not installed. \
             Run `rustup target add {WASM_TARGET}`.\n\n{trimmed}"
        );
    }

    bail!("Compilation failed:\n{trimmed}");
}

fn execute_wasm(wasm_path: &std::path::Path) -> Result<String> {
    let mut config = wasmtime::Config::new();
    config.consume_fuel(true);

    let engine = wasmtime::Engine::new(&config).context("failed to create wasmtime engine")?;
    let module =
        wasmtime::Module::from_file(&engine, wasm_path).context("failed to load wasm module")?;
    let mut store = wasmtime::Store::new(&engine, ());
    store.set_fuel(INITIAL_FUEL).context("failed to set fuel")?;

    let instance = wasmtime::Instance::new(&mut store, &module, &[])
        .context("failed to instantiate wasm module")?;

    let run_fn = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .context("wasm module does not export `run` function")?;
    let result_ptr_fn = instance
        .get_typed_func::<(), u32>(&mut store, "result_ptr")
        .context("wasm module does not export `result_ptr` function")?;
    let result_len_fn = instance
        .get_typed_func::<(), u32>(&mut store, "result_len")
        .context("wasm module does not export `result_len` function")?;

    run_fn
        .call(&mut store, ())
        .context("wasm `run` function trapped")?;

    let ptr = result_ptr_fn
        .call(&mut store, ())
        .context("wasm `result_ptr` trapped")? as usize;
    let len = result_len_fn
        .call(&mut store, ())
        .context("wasm `result_len` trapped")? as usize;

    if len > OUTPUT_LIMIT_BYTES {
        bail!("wasm output too large: {len} bytes (limit: {OUTPUT_LIMIT_BYTES} bytes)");
    }

    let memory = instance
        .get_memory(&mut store, "memory")
        .context("wasm module does not export memory")?;

    let data = memory.data(&store);
    let end = ptr
        .checked_add(len)
        .context("wasm result pointer overflow")?;
    let bytes = data
        .get(ptr..end)
        .context("wasm result points outside exported memory")?;

    let output = std::str::from_utf8(bytes)
        .context("wasm result is not valid UTF-8")?
        .to_string();

    Ok(output)
}

fn truncate_str(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_string();
    }
    let mut truncated = s[..limit].to_string();
    truncated.push_str("\n... (truncated)");
    truncated
}
