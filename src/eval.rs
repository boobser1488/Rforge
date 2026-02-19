use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;
use crate::ast::*;
use crate::env::{Env, UserFunction, BuiltinFn};
use crate::value::Value;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

pub async fn eval_block(stmts: &[Stmt], env: &mut Env) -> Result<Option<Value>, String> {
    for stmt in stmts {
        if let Some(val) = eval_stmt(stmt, env).await? {
            return Ok(Some(val));
        }
    }
    Ok(None)
}

fn eval_stmt<'a>(stmt: &'a Stmt, env: &'a mut Env) -> BoxFuture<'a, Result<Option<Value>, String>> {
    Box::pin(async move {
        match stmt {
            Stmt::Expr(expr) => {
                eval_expr(expr, env).await?;
                Ok(None)
            }
            Stmt::Assign { name, value } => {
                let val = eval_expr(value, env).await?;
                env.set_var(name.clone(), val);
                Ok(None)
            }
            Stmt::If { condition, then_branch, elif_branches, else_branch } => {
                if eval_expr(condition, env).await?.as_bool() {
                    return eval_block(then_branch, env).await;
                }
                for (cond, branch) in elif_branches {
                    if eval_expr(cond, env).await?.as_bool() {
                        return eval_block(branch, env).await;
                    }
                }
                if let Some(branch) = else_branch {
                    return eval_block(branch, env).await;
                }
                Ok(None)
            }
            Stmt::While { condition, body } => {
                while eval_expr(condition, env).await?.as_bool() {
                    if let Some(val) = eval_block(body, env).await? {
                        return Ok(Some(val));
                    }
                }
                Ok(None)
            }
            Stmt::For { var, start, end, body } => {
                let start_val = eval_expr(start, env).await?;
                let end_val = eval_expr(end, env).await?;
                let start_num = match start_val {
                    Value::Number(n) => n as i64,
                    _ => return Err("start value must be number".to_string()),
                };
                let end_num = match end_val {
                    Value::Number(n) => n as i64,
                    _ => return Err("end value must be number".to_string()),
                };
                for i in start_num..=end_num {
                    env.set_var(var.clone(), Value::Number(i as f64));
                    if let Some(val) = eval_block(body, env).await? {
                        return Ok(Some(val));
                    }
                }
                Ok(None)
            }
            Stmt::ForIn { var, array, body } => {
                let arr_val = eval_expr(array, env).await?;
                match arr_val {
                    Value::Array(arr_rc) => {
                        let arr = arr_rc.borrow().clone();
                        for item in arr {
                            env.set_var(var.clone(), item);
                            if let Some(val) = eval_block(body, env).await? {
                                return Ok(Some(val));
                            }
                        }
                        Ok(None)
                    }
                    _ => Err("for-in: right side must be array".to_string()),
                }
            }
            Stmt::Return(expr) => {
                let val = eval_expr(expr, env).await?;
                Ok(Some(val))
            }
            Stmt::FunctionDef { name, params, body, is_async } => {
                let func = UserFunction {
                    name: name.clone(),
                    params: params.clone(),
                    body: body.clone(),
                    is_async: *is_async,
                };
                env.define_func(name.clone(), func);
                Ok(None)
            }
            Stmt::Print(exprs) => {
                let mut first = true;
                for expr in exprs {
                    if !first {
                        print!(" ");
                    }
                    first = false;
                    let val = eval_expr(expr, env).await?;
                    print!("{}", val);
                }
                println!();
                Ok(None)
            }
            Stmt::LoadFrom { folder, target } => {
                use std::fs;
                use std::path::Path;
                let folder_path = Path::new(folder);
                if !folder_path.exists() || !folder_path.is_dir() {
                    return Err(format!("Module folder '{}' not found", folder));
                }
                let files = match target {
                    LoadTarget::All => {
                        let entries = fs::read_dir(folder_path)
                            .map_err(|e| format!("Failed to read folder: {}", e))?;
                        let mut list = Vec::new();
                        for entry in entries {
                            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
                            let path = entry.path();
                            if path.extension().and_then(|s| s.to_str()) == Some("forge") {
                                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                    list.push(name.to_string());
                                }
                            }
                        }
                        list
                    }
                    LoadTarget::File(name) => vec![name.clone()],
                };
                for file in files {
                    let full_path = folder_path.join(&file);
                    if !full_path.exists() {
                        return Err(format!("File '{}' not found", full_path.display()));
                    }
                    let content = fs::read_to_string(&full_path)
                        .map_err(|e| format!("Failed to read file '{}': {}", full_path.display(), e))?;
                    let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
                    let stmts = crate::parser::parse(&lines)?;
                    eval_block(&stmts, env).await?;
                }
                Ok(None)
            }
            Stmt::TryCatch { try_body, catch_body } => {
                let env_snapshot = env.clone();
                match eval_block(try_body, env).await {
                    Ok(Some(val)) => Ok(Some(val)),
                    Ok(None) => Ok(None),
                    Err(_) => {
                        *env = env_snapshot;
                        eval_block(catch_body, env).await
                    }
                }
            }
            Stmt::ClassDef { name, parent, fields, methods } => {
                let mut field_map = HashMap::new();
                for (fname, fexpr) in fields {
                    let val = eval_expr(fexpr, env).await?;
                    field_map.insert(fname.clone(), val);
                }
                let parent_val = if let Some(p) = parent {
                    match env.get_class(p) {
                        Some(v) => Some(Rc::new(v)),
                        None => return Err(format!("Parent class '{}' not found", p)),
                    }
                } else {
                    None
                };
                let class_value = Value::Class {
                    name: name.clone(),
                    parent: parent_val,
                    fields: Rc::new(RefCell::new(field_map)),
                    methods: methods.iter().map(|m| (m.name.clone(), Rc::new(m.clone()))).collect(),
                };
                env.define_class(name.clone(), class_value);
                Ok(None)
            }
            Stmt::ImportDll { path, name, alias } => {
                let lib = env.get_dll(path)?;
                let lib_clone = Rc::clone(&lib);
                let func_name = name.clone();
                let wrapper: BuiltinFn = Rc::new(move |args: Vec<Value>, _env: &mut Env| -> BoxFuture<Result<Value, String>> {
                    let lib = Rc::clone(&lib_clone);
                    let func_name = func_name.clone();
                    Box::pin(async move {
                        if !args.is_empty() {
                            return Err("DLL function called with arguments (not supported in this simple version)".to_string());
                        }
                        unsafe {
                            let func: libloading::Symbol<unsafe extern "C" fn() -> i32> = match lib.get(func_name.as_bytes()) {
                                Ok(f) => f,
                                Err(e) => return Err(format!("Failed to get function '{}': {}", func_name, e)),
                            };
                            let result = func();
                            Ok(Value::Number(result as f64))
                        }
                    })
                });
                env.add_builtin(&alias, wrapper);
                Ok(None)
            }
        }
    })
}

pub fn eval_expr<'a>(expr: &'a Expr, env: &'a mut Env) -> BoxFuture<'a, Result<Value, String>> {
    Box::pin(async move {
        match expr {
            Expr::Number(n) => Ok(Value::Number(*n)),
            Expr::String(s) => Ok(Value::String(s.clone())),
            Expr::Boolean(b) => Ok(Value::Boolean(*b)),
            Expr::Null => Ok(Value::Null),
            Expr::Variable(name) => {
                env.get_var(name).ok_or_else(|| format!("Variable '{}' not defined", name))
            }
            Expr::BinaryOp { left, op, right } => {
                let left_val = eval_expr(left, env).await?;
                let right_val = eval_expr(right, env).await?;
                match op {
                    BinaryOpKind::Add => add(&left_val, &right_val).await,
                    BinaryOpKind::Sub => sub(&left_val, &right_val).await,
                    BinaryOpKind::Mul => mul(&left_val, &right_val).await,
                    BinaryOpKind::Div => div(&left_val, &right_val).await,
                    BinaryOpKind::Mod => modulo(&left_val, &right_val).await,
                    BinaryOpKind::Eq => Ok(Value::Boolean(left_val == right_val)),
                    BinaryOpKind::Ne => Ok(Value::Boolean(left_val != right_val)),
                    BinaryOpKind::Lt => cmp(&left_val, &right_val, |a, b| a < b).await,
                    BinaryOpKind::Le => cmp(&left_val, &right_val, |a, b| a <= b).await,
                    BinaryOpKind::Gt => cmp(&left_val, &right_val, |a, b| a > b).await,
                    BinaryOpKind::Ge => cmp(&left_val, &right_val, |a, b| a >= b).await,
                    BinaryOpKind::And => Ok(Value::Boolean(left_val.as_bool() && right_val.as_bool())),
                    BinaryOpKind::Or => Ok(Value::Boolean(left_val.as_bool() || right_val.as_bool())),
                }
            }
            Expr::UnaryOp { op, expr } => {
                let val = eval_expr(expr, env).await?;
                match op {
                    UnaryOpKind::Not => Ok(Value::Boolean(!val.as_bool())),
                    UnaryOpKind::Neg => match val {
                        Value::Number(n) => Ok(Value::Number(-n)),
                        _ => Err("Unary minus applied to non-number".to_string()),
                    },
                }
            }
            Expr::Call { name, args } => {
                let mut arg_vals = Vec::new();
                for arg in args {
                    arg_vals.push(eval_expr(arg, env).await?);
                }
                if let Some(class_val) = env.get_class(name) {
                    return class_val.call_as_class(arg_vals, env).await;
                }
                if let Some(builtin) = env.get_builtin(name) {
                    return builtin(arg_vals, env).await;
                }
                if let Some(func) = env.get_func(name) {
                    if arg_vals.len() != func.params.len() {
                        return Err(format!("Function '{}' expects {} arguments, got {}", name, func.params.len(), arg_vals.len()));
                    }
                    let mut local_env = env.child();
                    for (p, v) in func.params.iter().zip(arg_vals) {
                        local_env.set_var(p.clone(), v);
                    }
                    let result = eval_block(&func.body, &mut local_env).await?;
                    Ok(result.unwrap_or(Value::Null))
                } else {
                    Err(format!("Unknown function or class '{}'", name))
                }
            }
            Expr::Index { array, index } => {
                let arr_val = eval_expr(array, env).await?;
                let idx_val = eval_expr(index, env).await?;
                match (arr_val, idx_val) {
                    (Value::Array(arr_rc), Value::Number(n)) => {
                        let arr = arr_rc.borrow();
                        let i = n as usize;
                        if i < arr.len() {
                            Ok(arr[i].clone())
                        } else {
                            Err("Index out of bounds".to_string())
                        }
                    }
                    (Value::String(s), Value::Number(n)) => {
                        let i = n as usize;
                        if i < s.len() {
                            Ok(Value::String(s.chars().nth(i).unwrap().to_string()))
                        } else {
                            Err("String index out of bounds".to_string())
                        }
                    }
                    _ => Err("Invalid index access".to_string()),
                }
            }
            Expr::GetAttr { object, attr } => {
                let obj_val = eval_expr(object, env).await?;
                obj_val.get_attr(attr).ok_or_else(|| format!("Attribute '{}' not found", attr))
            }
            Expr::SetAttr { object, attr, value } => {
                let obj_val = eval_expr(object, env).await?;
                let val = eval_expr(value, env).await?;
                obj_val.set_attr(attr.clone(), val)?;
                Ok(Value::Null)
            }
            Expr::CallMethod { object, method, args } => {
                let obj_val = eval_expr(object, env).await?;
                let mut arg_vals = Vec::new();
                for arg in args {
                    arg_vals.push(eval_expr(arg, env).await?);
                }
                let method_val = obj_val.get_attr(method).ok_or_else(|| format!("Method '{}' not found", method))?;
                match method_val {
                    Value::Method(func, class_or_self) => {
                        let mut call_args = vec![obj_val.clone()];
                        call_args.extend(arg_vals);
                        if call_args.len() != func.params.len() {
                            return Err(format!("Method '{}' expects {} arguments, got {}", method, func.params.len(), call_args.len()));
                        }
                        let mut local_env = env.child();
                        for (p, v) in func.params.iter().zip(call_args) {
                            local_env.set_var(p.clone(), v);
                        }
                        let result = eval_block(&func.body, &mut local_env).await?;
                        Ok(result.unwrap_or(Value::Null))
                    }
                    _ => Err("Not a method".to_string()),
                }
            }
            Expr::Super { args } => {
                Err("super not implemented yet".to_string())
            }
        }
    })
}

async fn add(a: &Value, b: &Value) -> Result<Value, String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Ok(Value::Number(x + y)),
        (Value::String(x), Value::String(y)) => Ok(Value::String(format!("{}{}", x, y))),
        (Value::String(x), y) => Ok(Value::String(format!("{}{}", x, y))),
        (x, Value::String(y)) => Ok(Value::String(format!("{}{}", x, y))),
        (Value::Array(x_rc), Value::Array(y_rc)) => {
            let x = x_rc.borrow();
            let y = y_rc.borrow();
            let mut new_vec = x.clone();
            new_vec.extend(y.clone());
            Ok(Value::Array(Rc::new(RefCell::new(new_vec))))
        }
        _ => Err("Invalid operands for +".to_string()),
    }
}

async fn sub(a: &Value, b: &Value) -> Result<Value, String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Ok(Value::Number(x - y)),
        _ => Err("Invalid operands for -".to_string()),
    }
}

async fn mul(a: &Value, b: &Value) -> Result<Value, String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Ok(Value::Number(x * y)),
        _ => Err("Invalid operands for *".to_string()),
    }
}

async fn div(a: &Value, b: &Value) -> Result<Value, String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if *y == 0.0 {
                Err("Division by zero".to_string())
            } else {
                Ok(Value::Number(x / y))
            }
        }
        _ => Err("Invalid operands for /".to_string()),
    }
}

async fn modulo(a: &Value, b: &Value) -> Result<Value, String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Ok(Value::Number(x % y)),
        _ => Err("Invalid operands for %".to_string()),
    }
}

async fn cmp<F>(a: &Value, b: &Value, f: F) -> Result<Value, String>
where
    F: FnOnce(f64, f64) -> bool,
{
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Ok(Value::Boolean(f(*x, *y))),
        (Value::String(x), Value::String(y)) => {
            Ok(Value::Boolean(f(x.len() as f64, y.len() as f64)))
        }
        _ => Err("Comparison not supported for these types".to_string()),
    }
}