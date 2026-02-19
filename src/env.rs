use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use crate::ast::Stmt;
use crate::eval::BoxFuture;
use crate::value::Value;
use libloading::Library;

#[derive(Debug, Clone)]
pub struct UserFunction {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    pub is_async: bool,
}

pub type BuiltinFn = Rc<dyn Fn(Vec<Value>, &mut Env) -> BoxFuture<'_, Result<Value, String>>>;

#[derive(Clone)]
pub struct Env {
    vars: HashMap<String, Value>,
    funcs: HashMap<String, UserFunction>,
    builtins: HashMap<String, BuiltinFn>,
    classes: HashMap<String, Value>,
    dll_cache: HashMap<String, Rc<Library>>,
    parent: Option<Rc<RefCell<Env>>>,
    memory: Vec<u8>,
    registers: HashMap<String, i64>,
}

impl Env {
    pub fn new() -> Self {
        Self {
            vars: HashMap::new(),
            funcs: HashMap::new(),
            builtins: HashMap::new(),
            classes: HashMap::new(),
            dll_cache: HashMap::new(),
            parent: None,
            memory: vec![0; 65536],
            registers: HashMap::new(),
        }
    }

    pub fn child(&self) -> Self {
        Self {
            vars: HashMap::new(),
            funcs: self.funcs.clone(),
            builtins: self.builtins.clone(),
            classes: self.classes.clone(),
            dll_cache: self.dll_cache.clone(),
            parent: Some(Rc::new(RefCell::new(self.clone()))),
            memory: self.memory.clone(),
            registers: self.registers.clone(),
        }
    }

    pub fn get_var(&self, name: &str) -> Option<Value> {
        if let Some(val) = self.vars.get(name).cloned() {
            return Some(val);
        }
        if let Some(parent) = &self.parent {
            parent.borrow().get_var(name)
        } else {
            None
        }
    }

    pub fn set_var(&mut self, name: String, value: Value) {
        self.vars.insert(name, value);
    }

    pub fn has_var(&self, name: &str) -> bool {
        if self.vars.contains_key(name) {
            return true;
        }
        if let Some(parent) = &self.parent {
            parent.borrow().has_var(name)
        } else {
            false
        }
    }

    pub fn define_func(&mut self, name: String, func: UserFunction) {
        self.funcs.insert(name, func);
    }

    pub fn get_func(&self, name: &str) -> Option<UserFunction> {
        if let Some(func) = self.funcs.get(name).cloned() {
            return Some(func);
        }
        if let Some(parent) = &self.parent {
            parent.borrow().get_func(name)
        } else {
            None
        }
    }

    pub fn get_builtin(&self, name: &str) -> Option<BuiltinFn> {
        if let Some(f) = self.builtins.get(name).cloned() {
            return Some(f);
        }
        if let Some(parent) = &self.parent {
            parent.borrow().get_builtin(name)
        } else {
            None
        }
    }

    pub fn add_builtin(&mut self, name: &str, f: BuiltinFn) {
        self.builtins.insert(name.to_string(), f);
    }

    pub fn define_class(&mut self, name: String, class_value: Value) {
        self.classes.insert(name, class_value);
    }

    pub fn get_class(&self, name: &str) -> Option<Value> {
        if let Some(val) = self.classes.get(name).cloned() {
            return Some(val);
        }
        if let Some(parent) = &self.parent {
            parent.borrow().get_class(name)
        } else {
            None
        }
    }

    pub fn get_dll(&mut self, path: &str) -> Result<Rc<Library>, String> {
        if let Some(lib) = self.dll_cache.get(path) {
            return Ok(Rc::clone(lib));
        }
        unsafe {
            match Library::new(path) {
                Ok(lib) => {
                    let lib_rc = Rc::new(lib);
                    self.dll_cache.insert(path.to_string(), Rc::clone(&lib_rc));
                    Ok(lib_rc)
                }
                Err(e) => Err(format!("Failed to load DLL '{}': {}", path, e)),
            }
        }
    }

    pub fn mem_read(&self, addr: usize) -> Result<u8, String> {
        self.memory.get(addr).copied().ok_or_else(|| "Memory access out of bounds".to_string())
    }

    pub fn mem_write(&mut self, addr: usize, value: u8) -> Result<(), String> {
        if addr < self.memory.len() {
            self.memory[addr] = value;
            Ok(())
        } else {
            Err("Memory access out of bounds".to_string())
        }
    }

    pub fn get_reg(&self, name: &str) -> Option<i64> {
        self.registers.get(name).copied()
    }

    pub fn set_reg(&mut self, name: String, value: i64) {
        self.registers.insert(name, value);
    }
}