use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use crate::env::UserFunction;

#[derive(Clone)]
pub enum Value {
    Number(f64),
    String(String),
    Boolean(bool),
    Array(Rc<RefCell<Vec<Value>>>),
    Null,
    Class {
        name: String,
        parent: Option<Rc<Value>>,
        fields: Rc<RefCell<HashMap<String, Value>>>,
        methods: HashMap<String, Rc<UserFunction>>,
    },
    Instance {
        class: Rc<Value>,
        fields: Rc<RefCell<HashMap<String, Value>>>,
    },
    Method(Rc<UserFunction>, Rc<Value>), // метод, связанный с экземпляром или классом
    Dll(Rc<libloading::Library>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Boolean(a), Value::Boolean(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
            (Value::Null, Value::Null) => true,
            (Value::Class { name, .. }, Value::Class { name: name2, .. }) => name == name2,
            (Value::Instance { class, fields }, Value::Instance { class: class2, fields: fields2 }) => {
                Rc::ptr_eq(class, class2) && Rc::ptr_eq(fields, fields2)
            }
            (Value::Method(f, o), Value::Method(f2, o2)) => Rc::ptr_eq(f, f2) && Rc::ptr_eq(o, o2),
            (Value::Dll(l), Value::Dll(l2)) => Rc::ptr_eq(l, l2),
            _ => false,
        }
    }
}

impl Value {
    pub fn as_bool(&self) -> bool {
        match self {
            Value::Boolean(b) => *b,
            Value::Number(n) => *n != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::Array(arr) => !arr.borrow().is_empty(),
            Value::Null => false,
            Value::Class { .. } => true,
            Value::Instance { .. } => true,
            Value::Method(..) => true,
            Value::Dll(..) => true,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Boolean(_) => "boolean",
            Value::Array(_) => "array",
            Value::Null => "null",
            Value::Class { .. } => "class",
            Value::Instance { .. } => "instance",
            Value::Method(..) => "method",
            Value::Dll(_) => "dll",
        }
    }

    pub fn get_attr(&self, attr: &str) -> Option<Value> {
        match self {
            Value::Instance { class, fields } => {
                if let Some(val) = fields.borrow().get(attr).cloned() {
                    return Some(val);
                }
                if let Value::Class { fields: class_fields, methods, .. } = &**class {
                    if let Some(val) = class_fields.borrow().get(attr).cloned() {
                        return Some(val);
                    }
                    if let Some(m) = methods.get(attr) {
                        return Some(Value::Method(Rc::clone(m), Rc::clone(class)));
                    }
                }
                None
            }
            Value::Class { fields, methods, .. } => {
                if let Some(val) = fields.borrow().get(attr).cloned() {
                    return Some(val);
                }
                if let Some(m) = methods.get(attr) {
                    return Some(Value::Method(Rc::clone(m), Rc::new(self.clone())));
                }
                None
            }
            _ => None,
        }
    }

    pub fn set_attr(&self, attr: String, value: Value) -> Result<(), String> {
        match self {
            Value::Instance { fields, .. } => {
                fields.borrow_mut().insert(attr, value);
                Ok(())
            }
            Value::Class { fields, .. } => {
                fields.borrow_mut().insert(attr, value);
                Ok(())
            }
            _ => Err("Cannot set attribute on this value".to_string()),
        }
    }

    pub async fn call_as_class(&self, args: Vec<Value>, env: &mut crate::env::Env) -> Result<Value, String> {
        match self {
            Value::Class { name, parent, fields, methods } => {
                let instance = Value::Instance {
                    class: Rc::new(self.clone()),
                    fields: Rc::new(RefCell::new(HashMap::new())),
                };
                if let Some(init) = methods.get("__init__") {
                    let mut call_args = vec![instance.clone()];
                    call_args.extend(args);
                    if call_args.len() != init.params.len() {
                        return Err(format!("Constructor __init__ expects {} arguments, got {}", init.params.len(), call_args.len()));
                    }
                    let mut local_env = env.child();
                    for (p, v) in init.params.iter().zip(call_args) {
                        local_env.set_var(p.clone(), v);
                    }
                    crate::eval::eval_block(&init.body, &mut local_env).await?;
                }
                Ok(instance)
            }
            _ => Err("Not a class".to_string()),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Number(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "{}", s),
            Value::Boolean(b) => write!(f, "{}", b),
            Value::Array(arr) => {
                let arr = arr.borrow();
                let elems: Vec<String> = arr.iter().map(|v| format!("{}", v)).collect();
                write!(f, "[{}]", elems.join(", "))
            }
            Value::Null => write!(f, "null"),
            Value::Class { name, .. } => write!(f, "<class {}>", name),
            Value::Instance { class, .. } => {
                if let Value::Class { name, .. } = &**class {
                    write!(f, "<instance of {}>", name)
                } else {
                    write!(f, "<instance>")
                }
            }
            Value::Method(_, _) => write!(f, "<method>"),
            Value::Dll(_) => write!(f, "<dll>"),
        }
    }
}