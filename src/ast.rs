use std::fmt;

#[derive(Debug, Clone)]
pub enum Expr {
    Number(f64),
    String(String),
    Boolean(bool),
    Null,
    Variable(String),
    BinaryOp {
        left: Box<Expr>,
        op: BinaryOpKind,
        right: Box<Expr>,
    },
    UnaryOp {
        op: UnaryOpKind,
        expr: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
    Index {
        array: Box<Expr>,
        index: Box<Expr>,
    },
    GetAttr {
        object: Box<Expr>,
        attr: String,
    },
    SetAttr {
        object: Box<Expr>,
        attr: String,
        value: Box<Expr>,
    },
    CallMethod {
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    Super {
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum BinaryOpKind {
    Add, Sub, Mul, Div, Mod,
    Eq, Ne, Lt, Le, Gt, Ge,
    And, Or,
}

#[derive(Debug, Clone, Copy)]
pub enum UnaryOpKind {
    Not,
    Neg,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Expr(Expr),
    Assign {
        name: String,
        value: Expr,
    },
    If {
        condition: Expr,
        then_branch: Vec<Stmt>,
        elif_branches: Vec<(Expr, Vec<Stmt>)>,
        else_branch: Option<Vec<Stmt>>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    For {
        var: String,
        start: Expr,
        end: Expr,
        body: Vec<Stmt>,
    },
    ForIn {
        var: String,
        array: Expr,
        body: Vec<Stmt>,
    },
    Return(Expr),
    FunctionDef {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
        is_async: bool,
    },
    Print(Vec<Expr>),
    LoadFrom {
        folder: String,
        target: LoadTarget,
    },
    TryCatch {
        try_body: Vec<Stmt>,
        catch_body: Vec<Stmt>,
    },
    ClassDef {
        name: String,
        parent: Option<String>,
        fields: Vec<(String, Expr)>,      // статические поля
        methods: Vec<crate::env::UserFunction>,
    },
    ImportDll {
        path: String,
        name: String,      // оригинальное имя функции
        alias: String,     // имя в языке
    },
}

#[derive(Debug, Clone)]
pub enum LoadTarget {
    All,
    File(String),
}

impl fmt::Display for BinaryOpKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            BinaryOpKind::Add => write!(f, "+"),
            BinaryOpKind::Sub => write!(f, "-"),
            BinaryOpKind::Mul => write!(f, "*"),
            BinaryOpKind::Div => write!(f, "/"),
            BinaryOpKind::Mod => write!(f, "%"),
            BinaryOpKind::Eq => write!(f, "=="),
            BinaryOpKind::Ne => write!(f, "!="),
            BinaryOpKind::Lt => write!(f, "<"),
            BinaryOpKind::Le => write!(f, "<="),
            BinaryOpKind::Gt => write!(f, ">"),
            BinaryOpKind::Ge => write!(f, ">="),
            BinaryOpKind::And => write!(f, "and"),
            BinaryOpKind::Or => write!(f, "or"),
        }
    }
}

impl fmt::Display for UnaryOpKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            UnaryOpKind::Not => write!(f, "!"),
            UnaryOpKind::Neg => write!(f, "-"),
        }
    }
}