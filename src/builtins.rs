use std::rc::Rc;
use std::time::Duration;
use std::fs;
use std::cell::RefCell;
use std::io::Write;
use std::path::Path;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::time;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::event::{self, Event};
use crate::env::{Env, BuiltinFn};
use crate::value::Value;
use crate::eval::BoxFuture;
use libloading::Library;
use lazy_static::lazy_static;

macro_rules! builtin {
    ($name:ident, $f:expr) => {
        pub fn $name() -> BuiltinFn {
            Rc::new($f)
        }
    };
}

// -----------------------------------------------------------------------------
// Existing builtins (sleep, array, push, pop, exit, length, slice, input, ...)
// -----------------------------------------------------------------------------

builtin!(sleep_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("sleep expects 1 argument".to_string());
        }
        let ms = match &args[0] {
            Value::Number(n) => *n as u64,
            _ => return Err("sleep argument must be number".to_string()),
        };
        time::sleep(Duration::from_millis(ms)).await;
        Ok(Value::Null)
    })
});

builtin!(array_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        Ok(Value::Array(Rc::new(RefCell::new(args))))
    })
});

builtin!(push_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 2 {
            return Err("push expects 2 arguments".to_string());
        }
        let (arr_val, val) = (args[0].clone(), args[1].clone());
        match arr_val {
            Value::Array(arr_rc) => {
                let mut arr = arr_rc.borrow_mut();
                arr.push(val);
                Ok(Value::Number(arr.len() as f64))
            }
            _ => Err("push: first argument must be array".to_string()),
        }
    })
});

builtin!(pop_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("pop expects 1 argument".to_string());
        }
        match args[0].clone() {
            Value::Array(arr_rc) => {
                let mut arr = arr_rc.borrow_mut();
                arr.pop().ok_or_else(|| "pop from empty array".to_string())
            }
            _ => Err("pop: argument must be array".to_string()),
        }
    })
});

builtin!(exit_fn, |_args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        println!("Press any key to exit...");
        if let Err(e) = enable_raw_mode() {
            return Err(format!("exit: failed to enable raw mode: {}", e));
        }
        let result = event::read();
        if let Err(e) = disable_raw_mode() {
            return Err(format!("exit: failed to disable raw mode: {}", e));
        }
        match result {
            Ok(Event::Key(_)) => {
                std::process::exit(0);
            }
            Ok(_) => {
                std::process::exit(0);
            }
            Err(e) => {
                return Err(format!("exit: failed to read event: {}", e));
            }
        }
    })
});

builtin!(length_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("length expects 1 argument".to_string());
        }
        match &args[0] {
            Value::Array(arr_rc) => {
                let arr = arr_rc.borrow();
                Ok(Value::Number(arr.len() as f64))
            }
            Value::String(s) => Ok(Value::Number(s.len() as f64)),
            _ => Err("length: argument must be array or string".to_string()),
        }
    })
});

builtin!(slice_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 3 {
            return Err("slice expects 3 arguments".to_string());
        }
        match (&args[0], &args[1], &args[2]) {
            (Value::Array(arr_rc), Value::Number(start), Value::Number(end)) => {
                let arr = arr_rc.borrow();
                let s = *start as usize;
                let e = *end as usize;
                if s > e {
                    return Err("slice: start index must be <= end index".to_string());
                }
                if e > arr.len() {
                    return Err("slice: end index out of bounds".to_string());
                }
                let sliced = arr[s..e].to_vec();
                Ok(Value::Array(Rc::new(RefCell::new(sliced))))
            }
            _ => Err("slice: first argument must be array, start and end must be numbers".to_string()),
        }
    })
});

builtin!(input_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        let prompt = if args.is_empty() {
            String::new()
        } else {
            match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("input prompt must be string".to_string()),
            }
        };
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let mut stdout = tokio::io::stdout();
        if let Err(e) = stdout.write_all(prompt.as_bytes()).await {
            return Err(format!("input: failed to write prompt: {}", e));
        }
        if let Err(e) = stdout.flush().await {
            return Err(format!("input: failed to flush stdout: {}", e));
        }
        let mut reader = BufReader::new(tokio::io::stdin());
        let mut line = String::new();
        if let Err(e) = reader.read_line(&mut line).await {
            return Err(format!("input: failed to read line: {}", e));
        }
        Ok(Value::String(line.trim_end().to_string()))
    })
});

builtin!(write_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 2 {
            return Err("write expects 2 arguments".to_string());
        }
        let filename = match &args[0] {
            Value::String(s) => s,
            _ => return Err("write: first argument must be string".to_string()),
        };
        let content = match &args[1] {
            Value::String(s) => s.as_str(),
            Value::Number(n) => return Ok(Value::Boolean(fs::write(filename, n.to_string()).is_ok())),
            Value::Boolean(b) => return Ok(Value::Boolean(fs::write(filename, b.to_string()).is_ok())),
            _ => return Err("write: second argument must be string, number or boolean".to_string()),
        };
        Ok(Value::Boolean(fs::write(filename, content).is_ok()))
    })
});

builtin!(append_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 2 {
            return Err("append expects 2 arguments".to_string());
        }
        let filename = match &args[0] {
            Value::String(s) => s,
            _ => return Err("append: first argument must be string".to_string()),
        };
        let content = match &args[1] {
            Value::String(s) => s.as_str(),
            Value::Number(n) => {
                return Ok(Value::Boolean(
                    fs::OpenOptions::new().append(true).create(true).open(filename)
                        .and_then(|mut f| write!(f, "{}", n)).is_ok()
                ));
            }
            Value::Boolean(b) => {
                return Ok(Value::Boolean(
                    fs::OpenOptions::new().append(true).create(true).open(filename)
                        .and_then(|mut f| write!(f, "{}", b)).is_ok()
                ));
            }
            _ => return Err("append: second argument must be string, number or boolean".to_string()),
        };
        Ok(Value::Boolean(
            fs::OpenOptions::new().append(true).create(true).open(filename)
                .and_then(|mut f| write!(f, "{}", content)).is_ok()
        ))
    })
});

builtin!(read_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("read expects 1 argument".to_string());
        }
        let filename = match &args[0] {
            Value::String(s) => s,
            _ => return Err("read: argument must be string".to_string()),
        };
        match fs::read_to_string(filename) {
            Ok(content) => Ok(Value::String(content)),
            Err(e) => Err(format!("read: {}", e)),
        }
    })
});

builtin!(upper_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("upper expects 1 argument".to_string());
        }
        match &args[0] {
            Value::String(s) => Ok(Value::String(s.to_uppercase())),
            _ => Err("upper: argument must be string".to_string()),
        }
    })
});

builtin!(lower_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("lower expects 1 argument".to_string());
        }
        match &args[0] {
            Value::String(s) => Ok(Value::String(s.to_lowercase())),
            _ => Err("lower: argument must be string".to_string()),
        }
    })
});

builtin!(split_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 2 {
            return Err("split expects 2 arguments".to_string());
        }
        match (&args[0], &args[1]) {
            (Value::String(s), Value::String(sep)) => {
                let parts: Vec<Value> = s.split(sep).map(|x| Value::String(x.to_string())).collect();
                Ok(Value::Array(Rc::new(RefCell::new(parts))))
            }
            _ => Err("split: arguments must be strings".to_string()),
        }
    })
});

builtin!(join_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 2 {
            return Err("join expects 2 arguments".to_string());
        }
        match (&args[0], &args[1]) {
            (Value::Array(arr_rc), Value::String(sep)) => {
                let arr = arr_rc.borrow();
                let strings: Result<Vec<String>, _> = arr.iter().map(|v| {
                    if let Value::String(s) = v {
                        Ok(s.clone())
                    } else {
                        Err("join: array elements must be strings")
                    }
                }).collect();
                let strings = strings.map_err(|e| e.to_string())?;
                Ok(Value::String(strings.join(sep)))
            }
            _ => Err("join: first argument must be array of strings, second must be string".to_string()),
        }
    })
});

builtin!(replace_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 3 {
            return Err("replace expects 3 arguments".to_string());
        }
        match (&args[0], &args[1], &args[2]) {
            (Value::String(s), Value::String(from), Value::String(to)) => {
                Ok(Value::String(s.replace(from, to)))
            }
            _ => Err("replace: all arguments must be strings".to_string()),
        }
    })
});

builtin!(contains_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 2 {
            return Err("contains expects 2 arguments".to_string());
        }
        match (&args[0], &args[1]) {
            (Value::String(s), Value::String(sub)) => Ok(Value::Boolean(s.contains(sub))),
            _ => Err("contains: arguments must be strings".to_string()),
        }
    })
});

builtin!(get_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 2 {
            return Err("get expects 2 arguments".to_string());
        }
        match (&args[0], &args[1]) {
            (Value::Array(arr_rc), Value::Number(i)) => {
                let arr = arr_rc.borrow();
                let idx = *i as usize;
                if idx < arr.len() {
                    Ok(arr[idx].clone())
                } else {
                    Err("get: index out of bounds".to_string())
                }
            }
            _ => Err("get: first argument must be array, second must be number".to_string()),
        }
    })
});

builtin!(set_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 3 {
            return Err("set expects 3 arguments".to_string());
        }
        match (&args[0], &args[1], &args[2]) {
            (Value::Array(arr_rc), Value::Number(i), val) => {
                let mut arr = arr_rc.borrow_mut();
                let idx = *i as usize;
                if idx < arr.len() {
                    arr[idx] = val.clone();
                    Ok(Value::Null)
                } else {
                    Err("set: index out of bounds".to_string())
                }
            }
            _ => Err("set: first argument must be array, second must be number".to_string()),
        }
    })
});

builtin!(file_exists_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("file_exists expects 1 argument".to_string());
        }
        let path = match &args[0] {
            Value::String(s) => s,
            _ => return Err("file_exists argument must be string".to_string()),
        };
        Ok(Value::Boolean(Path::new(path).exists()))
    })
});

builtin!(mem_read_fn, |args: Vec<Value>, env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("mem_read expects 1 argument".to_string());
        }
        let addr = match &args[0] {
            Value::Number(n) => *n as usize,
            _ => return Err("mem_read argument must be number".to_string()),
        };
        match env.mem_read(addr) {
            Ok(byte) => Ok(Value::Number(byte as f64)),
            Err(e) => Err(e),
        }
    })
});

builtin!(mem_write_fn, |args: Vec<Value>, env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 2 {
            return Err("mem_write expects 2 arguments".to_string());
        }
        let addr = match &args[0] {
            Value::Number(n) => *n as usize,
            _ => return Err("mem_write first argument must be number".to_string()),
        };
        let value = match &args[1] {
            Value::Number(n) => *n as u8,
            _ => return Err("mem_write second argument must be number".to_string()),
        };
        match env.mem_write(addr, value) {
            Ok(()) => Ok(Value::Null),
            Err(e) => Err(e),
        }
    })
});

builtin!(get_reg_fn, |args: Vec<Value>, env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("get_reg expects 1 argument".to_string());
        }
        let name = match &args[0] {
            Value::String(s) => s,
            _ => return Err("get_reg argument must be string".to_string()),
        };
        match env.get_reg(name) {
            Some(val) => Ok(Value::Number(val as f64)),
            None => Err(format!("Register '{}' not defined", name)),
        }
    })
});

builtin!(set_reg_fn, |args: Vec<Value>, env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 2 {
            return Err("set_reg expects 2 arguments".to_string());
        }
        let name = match &args[0] {
            Value::String(s) => s.clone(),
            _ => return Err("set_reg first argument must be string".to_string()),
        };
        let value = match &args[1] {
            Value::Number(n) => *n as i64,
            _ => return Err("set_reg second argument must be number".to_string()),
        };
        env.set_reg(name, value);
        Ok(Value::Null)
    })
});

// -----------------------------------------------------------------------------
// DLL-related builtins (with 64‑bit support)
// -----------------------------------------------------------------------------

builtin!(dll_load_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("dll_load expects 1 argument".to_string());
        }
        let path = match &args[0] {
            Value::String(s) => s,
            _ => return Err("dll_load argument must be string".to_string()),
        };
        unsafe {
            match Library::new(path) {
                Ok(lib) => Ok(Value::Dll(Rc::new(lib))),
                Err(e) => Err(format!("Failed to load DLL: {}", e)),
            }
        }
    })
});

builtin!(dll_call_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() < 2 {
            return Err("dll_call expects at least 2 arguments".to_string());
        }
        let lib = match &args[0] {
            Value::Dll(lib) => lib,
            _ => return Err("dll_call first argument must be a DLL handle".to_string()),
        };
        let func_name = match &args[1] {
            Value::String(s) => s,
            _ => return Err("dll_call second argument must be string (function name)".to_string()),
        };

        // Convert arguments to C‑compatible types (64‑bit aware)
        let mut c_args = Vec::new();
        let mut string_holders = Vec::new();

        for arg in args.iter().skip(2) {
            match arg {
                Value::Number(n) => c_args.push(*n as i64),   // use i64 for 64‑bit compatibility
                Value::String(s) => {
                    let mut bytes = s.as_bytes().to_vec();
                    bytes.push(0);
                    let ptr = bytes.as_ptr() as i64;
                    string_holders.push(bytes);
                    c_args.push(ptr);
                }
                Value::Boolean(b) => c_args.push(if *b { 1 } else { 0 }),
                _ => return Err(format!("Unsupported argument type for DLL call: {}", arg.type_name())),
            }
        }

        unsafe {
            // Dispatch based on argument count – we support up to 12 arguments.
            // The return type is i64 (to hold pointers or 64‑bit integers).
            match c_args.len() {
                0 => {
                    let func: libloading::Symbol<unsafe extern "C" fn() -> i64> = lib.get(func_name.as_bytes())
                        .map_err(|e| format!("Failed to get function '{}': {}", func_name, e))?;
                    Ok(Value::Number(func() as f64))
                }
                1 => {
                    let func: libloading::Symbol<unsafe extern "C" fn(i64) -> i64> = lib.get(func_name.as_bytes())
                        .map_err(|e| format!("Failed to get function '{}': {}", func_name, e))?;
                    Ok(Value::Number(func(c_args[0]) as f64))
                }
                2 => {
                    let func: libloading::Symbol<unsafe extern "C" fn(i64, i64) -> i64> = lib.get(func_name.as_bytes())
                        .map_err(|e| format!("Failed to get function '{}': {}", func_name, e))?;
                    Ok(Value::Number(func(c_args[0], c_args[1]) as f64))
                }
                3 => {
                    let func: libloading::Symbol<unsafe extern "C" fn(i64, i64, i64) -> i64> = lib.get(func_name.as_bytes())
                        .map_err(|e| format!("Failed to get function '{}': {}", func_name, e))?;
                    Ok(Value::Number(func(c_args[0], c_args[1], c_args[2]) as f64))
                }
                4 => {
                    let func: libloading::Symbol<unsafe extern "C" fn(i64, i64, i64, i64) -> i64> = lib.get(func_name.as_bytes())
                        .map_err(|e| format!("Failed to get function '{}': {}", func_name, e))?;
                    Ok(Value::Number(func(c_args[0], c_args[1], c_args[2], c_args[3]) as f64))
                }
                5 => {
                    let func: libloading::Symbol<unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64> = lib.get(func_name.as_bytes())
                        .map_err(|e| format!("Failed to get function '{}': {}", func_name, e))?;
                    Ok(Value::Number(func(c_args[0], c_args[1], c_args[2], c_args[3], c_args[4]) as f64))
                }
                6 => {
                    let func: libloading::Symbol<unsafe extern "C" fn(i64, i64, i64, i64, i64, i64) -> i64> = lib.get(func_name.as_bytes())
                        .map_err(|e| format!("Failed to get function '{}': {}", func_name, e))?;
                    Ok(Value::Number(func(c_args[0], c_args[1], c_args[2], c_args[3], c_args[4], c_args[5]) as f64))
                }
                7 => {
                    let func: libloading::Symbol<unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64) -> i64> = lib.get(func_name.as_bytes())
                        .map_err(|e| format!("Failed to get function '{}': {}", func_name, e))?;
                    Ok(Value::Number(func(c_args[0], c_args[1], c_args[2], c_args[3], c_args[4], c_args[5], c_args[6]) as f64))
                }
                8 => {
                    let func: libloading::Symbol<unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64) -> i64> = lib.get(func_name.as_bytes())
                        .map_err(|e| format!("Failed to get function '{}': {}", func_name, e))?;
                    Ok(Value::Number(func(c_args[0], c_args[1], c_args[2], c_args[3], c_args[4], c_args[5], c_args[6], c_args[7]) as f64))
                }
                9 => {
                    let func: libloading::Symbol<unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64> = lib.get(func_name.as_bytes())
                        .map_err(|e| format!("Failed to get function '{}': {}", func_name, e))?;
                    Ok(Value::Number(func(c_args[0], c_args[1], c_args[2], c_args[3], c_args[4], c_args[5], c_args[6], c_args[7], c_args[8]) as f64))
                }
                10 => {
                    let func: libloading::Symbol<unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64> = lib.get(func_name.as_bytes())
                        .map_err(|e| format!("Failed to get function '{}': {}", func_name, e))?;
                    Ok(Value::Number(func(c_args[0], c_args[1], c_args[2], c_args[3], c_args[4], c_args[5], c_args[6], c_args[7], c_args[8], c_args[9]) as f64))
                }
                11 => {
                    let func: libloading::Symbol<unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64> = lib.get(func_name.as_bytes())
                        .map_err(|e| format!("Failed to get function '{}': {}", func_name, e))?;
                    Ok(Value::Number(func(c_args[0], c_args[1], c_args[2], c_args[3], c_args[4], c_args[5], c_args[6], c_args[7], c_args[8], c_args[9], c_args[10]) as f64))
                }
                12 => {
                    let func: libloading::Symbol<unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64> = lib.get(func_name.as_bytes())
                        .map_err(|e| format!("Failed to get function '{}': {}", func_name, e))?;
                    Ok(Value::Number(func(c_args[0], c_args[1], c_args[2], c_args[3], c_args[4], c_args[5], c_args[6], c_args[7], c_args[8], c_args[9], c_args[10], c_args[11]) as f64))
                }
                _ => Err("Too many arguments for DLL call".to_string()),
            }
        }
    })
});

builtin!(dll_free_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("dll_free expects 1 argument".to_string());
        }
        match &args[0] {
            Value::Dll(_) => Ok(Value::Null),
            _ => Err("dll_free argument must be a DLL handle".to_string()),
        }
    })
});

// -----------------------------------------------------------------------------
// Memory management builtins (malloc, free, poke, peek, peek32)
// -----------------------------------------------------------------------------

lazy_static! {
    static ref HEAP: Mutex<HashMap<usize, Vec<u8>>> = Mutex::new(HashMap::new());
    static ref NEXT_PTR: Mutex<usize> = Mutex::new(1);
}

builtin!(malloc_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("malloc expects 1 argument (size)".to_string());
        }
        let size = match &args[0] {
            Value::Number(n) => *n as usize,
            _ => return Err("malloc argument must be number".to_string()),
        };
        let mut heap = HEAP.lock().unwrap();
        let mut next = NEXT_PTR.lock().unwrap();
        let ptr = *next;
        *next += 1;
        heap.insert(ptr, vec![0; size]);
        Ok(Value::Number(ptr as f64))
    })
});

builtin!(free_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("free expects 1 argument (ptr)".to_string());
        }
        let ptr = match &args[0] {
            Value::Number(n) => *n as usize,
            _ => return Err("free argument must be number".to_string()),
        };
        let mut heap = HEAP.lock().unwrap();
        heap.remove(&ptr);
        Ok(Value::Null)
    })
});

builtin!(poke_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 3 {
            return Err("poke expects 3 arguments: ptr, offset, value".to_string());
        }
        let ptr = match &args[0] {
            Value::Number(n) => *n as usize,
            _ => return Err("poke first argument must be number".to_string()),
        };
        let offset = match &args[1] {
            Value::Number(n) => *n as usize,
            _ => return Err("poke second argument must be number".to_string()),
        };
        let value = match &args[2] {
            Value::Number(n) => *n as u8,
            _ => return Err("poke third argument must be number (byte)".to_string()),
        };
        let mut heap = HEAP.lock().unwrap();
        let block = heap.get_mut(&ptr).ok_or("Invalid pointer")?;
        if offset >= block.len() {
            return Err("Offset out of bounds".to_string());
        }
        block[offset] = value;
        Ok(Value::Null)
    })
});

builtin!(peek_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 2 {
            return Err("peek expects 2 arguments: ptr, offset".to_string());
        }
        let ptr = match &args[0] {
            Value::Number(n) => *n as usize,
            _ => return Err("peek first argument must be number".to_string()),
        };
        let offset = match &args[1] {
            Value::Number(n) => *n as usize,
            _ => return Err("peek second argument must be number".to_string()),
        };
        let heap = HEAP.lock().unwrap();
        let block = heap.get(&ptr).ok_or("Invalid pointer")?;
        if offset >= block.len() {
            return Err("Offset out of bounds".to_string());
        }
        Ok(Value::Number(block[offset] as f64))
    })
});

builtin!(peek32_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 2 {
            return Err("peek32 expects 2 arguments: ptr, offset".to_string());
        }
        let ptr = match &args[0] {
            Value::Number(n) => *n as usize,
            _ => return Err("peek32 first argument must be number".to_string()),
        };
        let offset = match &args[1] {
            Value::Number(n) => *n as usize,
            _ => return Err("peek32 second argument must be number".to_string()),
        };
        let heap = HEAP.lock().unwrap();
        let block = heap.get(&ptr).ok_or("Invalid pointer")?;
        if offset + 3 >= block.len() {
            return Err("Offset out of bounds for 4-byte read".to_string());
        }
        let val = (block[offset] as u32) |
                 ((block[offset+1] as u32) << 8) |
                 ((block[offset+2] as u32) << 16) |
                 ((block[offset+3] as u32) << 24);
        Ok(Value::Number(val as f64))
    })
});

// -----------------------------------------------------------------------------
// Window class registration (experimental, not fully implemented)
// -----------------------------------------------------------------------------

// This is a stub – full implementation requires async callback handling.
builtin!(register_window_class_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 2 {
            return Err("register_window_class expects 2 arguments: class_name, callback_function_name".to_string());
        }
        // Just a placeholder – real implementation would register a window class
        // with a thunk that calls back into Forge.
        Err("Window class registration is not yet implemented in this build".to_string())
    })
});

// -----------------------------------------------------------------------------
// Type conversion and introspection
// -----------------------------------------------------------------------------

builtin!(tonumber_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("tonumber expects 1 argument".to_string());
        }
        match &args[0] {
            Value::String(s) => {
                match s.parse::<f64>() {
                    Ok(n) => Ok(Value::Number(n)),
                    Err(_) => Ok(Value::Number(0.0)),
                }
            }
            Value::Number(n) => Ok(Value::Number(*n)),
            Value::Boolean(b) => Ok(Value::Number(if *b { 1.0 } else { 0.0 })),
            Value::Null => Ok(Value::Number(0.0)),
            Value::Array(_) => Ok(Value::Number(0.0)),
            Value::Class { .. } | Value::Instance { .. } | Value::Method(_, _) | Value::Dll(_) => Ok(Value::Number(0.0)),
        }
    })
});

builtin!(type_fn, |args: Vec<Value>, _env: &mut Env| -> BoxFuture<'_, Result<Value, String>> {
    Box::pin(async move {
        if args.len() != 1 {
            return Err("type expects 1 argument".to_string());
        }
        let type_str = match &args[0] {
            Value::Null => "null",
            Value::Boolean(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Class { .. } => "class",
            Value::Instance { .. } => "instance",
            Value::Method(_, _) => "method",
            Value::Dll(_) => "dll",
        };
        Ok(Value::String(type_str.to_string()))
    })
});

// -----------------------------------------------------------------------------
// Install all builtins into the environment
// -----------------------------------------------------------------------------

pub fn install(env: &mut Env) {
    env.add_builtin("sleep", sleep_fn());
    env.add_builtin("array", array_fn());
    env.add_builtin("push", push_fn());
    env.add_builtin("pop", pop_fn());
    env.add_builtin("exit", exit_fn());
    env.add_builtin("length", length_fn());
    env.add_builtin("slice", slice_fn());
    env.add_builtin("input", input_fn());
    env.add_builtin("write", write_fn());
    env.add_builtin("append", append_fn());
    env.add_builtin("read", read_fn());
    env.add_builtin("upper", upper_fn());
    env.add_builtin("lower", lower_fn());
    env.add_builtin("split", split_fn());
    env.add_builtin("join", join_fn());
    env.add_builtin("replace", replace_fn());
    env.add_builtin("contains", contains_fn());
    env.add_builtin("get", get_fn());
    env.add_builtin("set", set_fn());
    env.add_builtin("file_exists", file_exists_fn());
    env.add_builtin("mem_read", mem_read_fn());
    env.add_builtin("mem_write", mem_write_fn());
    env.add_builtin("get_reg", get_reg_fn());
    env.add_builtin("set_reg", set_reg_fn());
    env.add_builtin("tonumber", tonumber_fn());
    env.add_builtin("type", type_fn());
    env.add_builtin("dll_load", dll_load_fn());
    env.add_builtin("dll_call", dll_call_fn());
    env.add_builtin("dll_free", dll_free_fn());
    env.add_builtin("malloc", malloc_fn());
    env.add_builtin("free", free_fn());
    env.add_builtin("poke", poke_fn());
    env.add_builtin("peek", peek_fn());
    env.add_builtin("peek32", peek32_fn());
    env.add_builtin("register_window_class", register_window_class_fn());
}