// main.rs (updated)
mod ast;
mod env;
mod eval;
mod parser;
mod builtins;
mod value;

use std::fs;
use std::path::Path;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    // Normal mode: read script from command line argument
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <file.forge>", args[0]);
        std::process::exit(1);
    }
    let filename = &args[1];
    if !filename.ends_with(".forge") {
        eprintln!("File must have .forge extension");
        std::process::exit(1);
    }
    if !Path::new(filename).exists() {
        eprintln!("File '{}' not found", filename);
        std::process::exit(1);
    }
    let content = fs::read_to_string(filename)
        .map_err(|e| format!("Failed to read file: {}", e))?;
    run_script(&content).await
}

/// Execute a Forge script given its source code.
async fn run_script(source: &str) -> Result<(), String> {
    let lines: Vec<String> = source.lines().map(|s| s.trim_end().to_string()).collect();
    let stmts = parser::parse(&lines)?;
    let mut env = env::Env::new();
    builtins::install(&mut env);
    eval::eval_block(&stmts, &mut env).await?;
    Ok(())
}