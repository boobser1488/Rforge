use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

fn main() {
    if let Err(e) = run() {
        eprintln!("{}Ошибка:{} {}", RED, RESET, e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("{}Использование:{} builder <script.forge>", YELLOW, RESET);
        std::process::exit(1);
    }

    let script_path = Path::new(&args[1]);
    if !script_path.exists() {
        return Err(format!("Файл '{}' не найден", script_path.display()));
    }
    if script_path.extension().and_then(|s| s.to_str()) != Some("forge") {
        return Err("Файл должен иметь расширение .forge".to_string());
    }

    println!("{}Читаем скрипт:{} {}", GREEN, RESET, script_path.display());
    let script_content = fs::read_to_string(script_path)
        .map_err(|e| format!("Не удалось прочитать скрипт: {}", e))?;

    let build_dir = PathBuf::from("forge_build_temp");
    if build_dir.exists() {
        fs::remove_dir_all(&build_dir)
            .map_err(|e| format!("Не удалось очистить старую папку сборки: {}", e))?;
    }
    fs::create_dir(&build_dir)
        .map_err(|e| format!("Не удалось создать папку сборки: {}", e))?;

    println!("{}Копируем исходники интерпретатора...{}", GREEN, RESET);
    copy_interpreter_sources(&build_dir)?;

    // Внедряем скрипт в main.rs
    let main_path = build_dir.join("src/main.rs");
    let main_code = fs::read_to_string(&main_path)
        .map_err(|e| format!("Не удалось прочитать main.rs: {}", e))?;

    let escaped = escape_rust_string(&script_content);

    // Создаём новый main, который сразу выполняет встроенный скрипт
    let new_main = format!(
        r#"
// --- Автоматически сгенерировано builder'ом ---
mod ast;
mod env;
mod eval;
mod parser;
mod builtins;
mod value;

const EMBEDDED_SCRIPT: &str = "{}";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {{
    run_script(EMBEDDED_SCRIPT).await
}}

async fn run_script(source: &str) -> Result<(), String> {{
    let lines: Vec<String> = source.lines().map(|s| s.trim_end().to_string()).collect();
    let stmts = parser::parse(&lines)?;
    let mut env = env::Env::new();
    builtins::install(&mut env);
    eval::eval_block(&stmts, &mut env).await?;
    Ok(())
}}
"#,
        escaped
    );

    fs::write(&main_path, new_main)
        .map_err(|e| format!("Не удалось записать изменённый main.rs: {}", e))?;

    println!("{}Компиляция с Cargo (релиз)...{}", GREEN, RESET);
    let status = Command::new("cargo")
        .current_dir(&build_dir)
        .arg("build")
        .arg("--release")
        .status()
        .map_err(|e| format!("Не удалось запустить cargo: {}", e))?;

    if !status.success() {
        return Err("Сборка Cargo не удалась".to_string());
    }

    let exe_name = script_path.file_stem().unwrap().to_str().unwrap();
    let target_exe = build_dir.join("target/release/forge_interpreter.exe");
    if !target_exe.exists() {
        return Err("Сборка завершена, но исполняемый файл не найден".to_string());
    }

    let dest_exe = PathBuf::from(format!("{}.exe", exe_name));
    fs::copy(&target_exe, &dest_exe)
        .map_err(|e| format!("Не удалось скопировать exe: {}", e))?;

    println!("{}Готово:{} {}", GREEN, RESET, dest_exe.display());
    println!("{}Папку сборки '{}' можно удалить вручную.{}", YELLOW, build_dir.display(), RESET);
    Ok(())
}

fn copy_interpreter_sources(dest: &Path) -> Result<(), String> {
    let src_dir = dest.join("src");
    fs::create_dir(&src_dir).map_err(|e| format!("Не удалось создать папку src: {}", e))?;

    let files = [
        "Cargo.toml",
        "src/ast.rs",
        "src/env.rs",
        "src/eval.rs",
        "src/parser.rs",
        "src/builtins.rs",
        "src/value.rs",
        "src/main.rs",
    ];

    for file in &files {
        let src_path = Path::new(file);
        if !src_path.exists() {
            return Err(format!("Не найден необходимый файл '{}' в текущей папке", file));
        }
        let dest_path = dest.join(file);
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::copy(src_path, &dest_path)
            .map_err(|e| format!("Не удалось скопировать {}: {}", file, e))?;
    }
    Ok(())
}

fn escape_rust_string(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() + 10);
    for c in s.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(c),
        }
    }
    escaped
}