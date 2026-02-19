use regex::Regex;
use crate::ast::*;
use lazy_static::lazy_static;
use std::iter::Peekable;
use std::vec::IntoIter;

lazy_static! {
    static ref RE_FUNCTION: Regex = Regex::new(r"^(?:(async)\s+)?function\s+(\w+)\s*\(([^)]*)\):$").unwrap();
    static ref RE_IF: Regex = Regex::new(r"^if\s+(.+):$").unwrap();
    static ref RE_ELIF: Regex = Regex::new(r"^elif\s+(.+):$").unwrap();
    static ref RE_ELSE: Regex = Regex::new(r"^else:$").unwrap();
    static ref RE_WHILE: Regex = Regex::new(r"^while\s+(.+):$").unwrap();
    static ref RE_FOR: Regex = Regex::new(r"^for\s+(\w+)\s*=\s*(.+),\s*(.+)\s*do$").unwrap();
    static ref RE_FOR_IN: Regex = Regex::new(r"^for\s+(\w+)\s+in\s+(.+):$").unwrap();
    static ref RE_TRY: Regex = Regex::new(r"^try:$").unwrap();
    static ref RE_CATCH: Regex = Regex::new(r"^catch:$").unwrap();
    static ref RE_RETURN: Regex = Regex::new(r"^return\s+(.+)$").unwrap();
    static ref RE_PRINT: Regex = Regex::new(r"^print\((.*)\)$").unwrap();
    static ref RE_ASSIGN: Regex = Regex::new(r"^(\w+)\s*=\s*(.+)$").unwrap();
    static ref RE_CALL: Regex = Regex::new(r"^(\w+)\((.*)\)$").unwrap();
    static ref RE_LOAD: Regex = Regex::new(r"^load\s+from\s+(\w+)\s+(.+)$").unwrap();
    static ref RE_CLASS: Regex = Regex::new(r"^class\s+(\w+)(?:\s*\(\s*(\w*)\s*\))?:$").unwrap();
    static ref RE_IMPORT_DLL: Regex = Regex::new(r#"^from\s+dll\s+"([^"]+)"\s+import\s+(\w+)(?:\s+as\s+(\w+))?$"#).unwrap();
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    String(String),
    Ident(String),
    Keyword(String),
    Operator(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    EOF,
}

pub fn parse(lines: &[String]) -> Result<Vec<Stmt>, String> {
    let mut stmts = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        if line.trim().is_empty() || is_comment(line) {
            i += 1;
            continue;
        }
        let indent = count_indent(line);
        let (block, next_i) = parse_block(lines, indent, i)?;
        stmts.extend(block);
        i = next_i;
    }
    Ok(stmts)
}

fn parse_block(lines: &[String], min_indent: usize, start: usize) -> Result<(Vec<Stmt>, usize), String> {
    let mut stmts = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let line = &lines[i];
        if line.trim().is_empty() || is_comment(line) {
            i += 1;
            continue;
        }
        let indent = count_indent(line);
        if indent < min_indent {
            break;
        }
        if indent > min_indent {
            if stmts.is_empty() {
                return Err(format!("Unexpected indentation at line {}", i + 1));
            }
            let last_stmt = stmts.last_mut().unwrap();
            match last_stmt {
                Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForIn { body, .. } | Stmt::FunctionDef { body, .. } => {
                    let (nested, next_i) = parse_block(lines, indent, i)?;
                    *body = nested;
                    i = next_i;
                    continue;
                }
                Stmt::If { then_branch, .. } => {
                    let (nested, next_i) = parse_block(lines, indent, i)?;
                    *then_branch = nested;
                    i = next_i;
                    continue;
                }
                Stmt::TryCatch { try_body, .. } => {
                    let (nested, next_i) = parse_block(lines, indent, i)?;
                    *try_body = nested;
                    i = next_i;
                    continue;
                }
                Stmt::ClassDef { fields, methods, .. } => {
                    let (nested, next_i) = parse_block(lines, indent, i)?;
                    for stmt in nested {
                        match stmt {
                            Stmt::Assign { name, value } => fields.push((name, value)),
                            Stmt::FunctionDef { name, params, body, is_async } => {
                                methods.push(crate::env::UserFunction {
                                    name,
                                    params,
                                    body,
                                    is_async,
                                });
                            }
                            _ => return Err(format!("Invalid statement inside class at line {}", i + 1)),
                        }
                    }
                    i = next_i;
                    continue;
                }
                _ => return Err(format!("Line {} cannot have a block", i + 1)),
            }
        }
        let trimmed = line.trim();
        let stmt = parse_stmt(trimmed, i + 1)?;

        // Обработка if-elif-else
        if let Stmt::If { condition, .. } = stmt {
            let mut current_if = Stmt::If {
                condition,
                then_branch: Vec::new(),
                elif_branches: Vec::new(),
                else_branch: None,
            };
            i += 1;

            if i >= lines.len() {
                return Err(format!("Expected block after if at line {}", i));
            }
            let then_indent = count_indent(&lines[i]);
            if then_indent <= min_indent {
                return Err(format!("Expected indented block after if at line {}", i + 1));
            }
            let (then_body, next_i) = parse_block(lines, then_indent, i)?;
            if let Stmt::If { ref mut then_branch, .. } = current_if {
                *then_branch = then_body;
            }
            i = next_i;

            while i < lines.len() {
                let next_line = &lines[i];
                if next_line.trim().is_empty() || is_comment(next_line) {
                    i += 1;
                    continue;
                }
                let next_indent = count_indent(next_line);
                if next_indent != min_indent {
                    break;
                }
                let next_trimmed = next_line.trim();
                if let Some(caps) = RE_ELIF.captures(next_trimmed) {
                    let cond = parse_expr(&caps[1])?;
                    i += 1;
                    if i >= lines.len() {
                        return Err(format!("Expected block after elif at line {}", i));
                    }
                    let elif_indent = count_indent(&lines[i]);
                    if elif_indent <= min_indent {
                        return Err(format!("Expected indented block after elif at line {}", i + 1));
                    }
                    let (elif_body, next_i) = parse_block(lines, elif_indent, i)?;
                    i = next_i;
                    if let Stmt::If { ref mut elif_branches, .. } = current_if {
                        elif_branches.push((cond, elif_body));
                    }
                    continue;
                } else if RE_ELSE.is_match(next_trimmed) {
                    i += 1;
                    if i >= lines.len() {
                        return Err(format!("Expected block after else at line {}", i));
                    }
                    let else_indent = count_indent(&lines[i]);
                    if else_indent <= min_indent {
                        return Err(format!("Expected indented block after else at line {}", i + 1));
                    }
                    let (else_body, next_i) = parse_block(lines, else_indent, i)?;
                    i = next_i;
                    if let Stmt::If { ref mut else_branch, .. } = current_if {
                        *else_branch = Some(else_body);
                    }
                    break;
                } else {
                    break;
                }
            }
            stmts.push(current_if);
            continue;
        }
        // Обработка try-catch
        else if let Stmt::TryCatch { try_body: _try_body, catch_body: _catch_body } = stmt {
            let mut current_try = Stmt::TryCatch { try_body: Vec::new(), catch_body: Vec::new() };
            i += 1;

            if i >= lines.len() {
                return Err(format!("Expected block after try at line {}", i));
            }
            let try_indent = count_indent(&lines[i]);
            if try_indent <= min_indent {
                return Err(format!("Expected indented block after try at line {}", i + 1));
            }
            let (try_body, next_i) = parse_block(lines, try_indent, i)?;
            if let Stmt::TryCatch { try_body: ref mut target, .. } = current_try {
                *target = try_body;
            }
            i = next_i;

            while i < lines.len() {
                let next_line = &lines[i];
                if next_line.trim().is_empty() || is_comment(next_line) {
                    i += 1;
                    continue;
                }
                break;
            }

            if i < lines.len() {
                let next_line = &lines[i];
                let next_indent = count_indent(next_line);
                if next_indent == min_indent && RE_CATCH.is_match(next_line.trim()) {
                    i += 1;
                    if i >= lines.len() {
                        return Err(format!("Expected block after catch at line {}", i));
                    }
                    let catch_indent = count_indent(&lines[i]);
                    if catch_indent <= min_indent {
                        return Err(format!("Expected indented block after catch at line {}", i + 1));
                    }
                    let (catch_body, next_i) = parse_block(lines, catch_indent, i)?;
                    i = next_i;
                    if let Stmt::TryCatch { catch_body: ref mut target, .. } = current_try {
                        *target = catch_body;
                    }
                } else {
                    return Err("Expected catch after try".to_string());
                }
            } else {
                return Err("Expected catch after try".to_string());
            }
            stmts.push(current_try);
            continue;
        } else {
            stmts.push(stmt);
            i += 1;
        }
    }
    Ok((stmts, i))
}

fn parse_stmt(line: &str, line_num: usize) -> Result<Stmt, String> {
    if let Some(caps) = RE_FUNCTION.captures(line) {
        let is_async = caps.get(1).is_some();
        let name = caps[2].to_string();
        let params: Vec<String> = caps[3]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        return Ok(Stmt::FunctionDef {
            name,
            params,
            body: vec![],
            is_async,
        });
    }
    if let Some(caps) = RE_IF.captures(line) {
        let cond = parse_expr(&caps[1])?;
        return Ok(Stmt::If {
            condition: cond,
            then_branch: vec![],
            elif_branches: vec![],
            else_branch: None,
        });
    }
    if let Some(caps) = RE_WHILE.captures(line) {
        let cond = parse_expr(&caps[1])?;
        return Ok(Stmt::While {
            condition: cond,
            body: vec![],
        });
    }
    if let Some(caps) = RE_FOR.captures(line) {
        let var = caps[1].to_string();
        let start = parse_expr(&caps[2])?;
        let end = parse_expr(&caps[3])?;
        return Ok(Stmt::For {
            var,
            start,
            end,
            body: vec![],
        });
    }
    if let Some(caps) = RE_FOR_IN.captures(line) {
        let var = caps[1].to_string();
        let array = parse_expr(&caps[2])?;
        return Ok(Stmt::ForIn {
            var,
            array,
            body: vec![],
        });
    }
    if RE_TRY.is_match(line) {
        return Ok(Stmt::TryCatch {
            try_body: vec![],
            catch_body: vec![],
        });
    }
    if let Some(caps) = RE_RETURN.captures(line) {
        let expr = parse_expr(&caps[1])?;
        return Ok(Stmt::Return(expr));
    }
    if let Some(caps) = RE_PRINT.captures(line) {
        let args_str = &caps[1];
        let args = parse_arguments(args_str)?;
        return Ok(Stmt::Print(args));
    }
    if let Some(caps) = RE_ASSIGN.captures(line) {
        let name = caps[1].to_string();
        let expr = parse_expr(&caps[2])?;
        return Ok(Stmt::Assign { name, value: expr });
    }
    if let Some(caps) = RE_CALL.captures(line) {
        let name = caps[1].to_string();
        let args_str = &caps[2];
        let args = parse_arguments(args_str)?;
        return Ok(Stmt::Expr(Expr::Call { name, args }));
    }
    if let Some(caps) = RE_LOAD.captures(line) {
        let folder = caps[1].to_string();
        let target_str = caps[2].to_string().trim().to_string();
        let target = if target_str == "all" {
            LoadTarget::All
        } else {
            LoadTarget::File(target_str)
        };
        return Ok(Stmt::LoadFrom { folder, target });
    }
    if let Some(caps) = RE_CLASS.captures(line) {
        let name = caps[1].to_string();
        let parent = caps.get(2).map(|m| m.as_str().to_string());
        return Ok(Stmt::ClassDef {
            name,
            parent,
            fields: vec![],
            methods: vec![],
        });
    }
    if let Some(caps) = RE_IMPORT_DLL.captures(line) {
        let path = caps[1].to_string();
        let name = caps[2].to_string();
        let alias = caps.get(3).map(|m| m.as_str().to_string()).unwrap_or(name.clone());
        return Ok(Stmt::ImportDll { path, name, alias });
    }
    Err(format!("Invalid syntax at line {}: {}", line_num, line))
}

fn parse_arguments(s: &str) -> Result<Vec<Expr>, String> {
    if s.trim().is_empty() {
        return Ok(vec![]);
    }
    let mut args = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    let mut in_string = false;
    let mut quote_char = '\0';
    let mut escaped = false;
    for ch in s.chars() {
        if in_string {
            if escaped {
                match ch {
                    'n' => current.push('\n'),
                    'r' => current.push('\r'),
                    't' => current.push('\t'),
                    '\\' => current.push('\\'),
                    '"' => current.push('"'),
                    '\'' => current.push('\''),
                    _ => current.push(ch),
                }
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else {
                current.push(ch);
                if ch == quote_char {
                    in_string = false;
                }
            }
        } else {
            match ch {
                '"' | '\'' => {
                    in_string = true;
                    quote_char = ch;
                    current.push(ch);
                }
                '(' => {
                    depth += 1;
                    current.push(ch);
                }
                ')' => {
                    depth -= 1;
                    current.push(ch);
                }
                ',' if depth == 0 => {
                    args.push(current.trim().to_string());
                    current.clear();
                }
                _ => current.push(ch),
            }
        }
    }
    if !current.is_empty() {
        args.push(current.trim().to_string());
    }
    args.into_iter().map(|a| parse_expr(&a)).collect()
}

// ---------- Парсер выражений ----------

fn parse_expr(input: &str) -> Result<Expr, String> {
    let tokens = tokenize(input)?;
    let mut iter = tokens.into_iter().peekable();
    let expr = parse_or(&mut iter)?;
    if iter.peek().is_some() && iter.peek().unwrap() != &Token::EOF {
        return Err(format!("Unexpected tokens at end of expression"));
    }
    Ok(expr)
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => continue,
            '(' => tokens.push(Token::LParen),
            ')' => tokens.push(Token::RParen),
            '[' => tokens.push(Token::LBracket),
            ']' => tokens.push(Token::RBracket),
            ',' => tokens.push(Token::Comma),
            '.' => tokens.push(Token::Dot),
            '+' | '-' | '*' | '/' | '%' | '=' | '!' | '<' | '>' => {
                let mut op = ch.to_string();
                if ch == '=' || ch == '!' || ch == '<' || ch == '>' {
                    if let Some(&next) = chars.peek() {
                        if next == '=' {
                            op.push(chars.next().unwrap());
                        }
                    }
                }
                tokens.push(Token::Operator(op));
            }
            '"' | '\'' => {
                let quote = ch;
                let mut s = String::new();
                let mut escaped = false;
                while let Some(next) = chars.next() {
                    if escaped {
                        match next {
                            'n' => s.push('\n'),
                            'r' => s.push('\r'),
                            't' => s.push('\t'),
                            '\\' => s.push('\\'),
                            '"' => s.push('"'),
                            '\'' => s.push('\''),
                            _ => s.push(next),
                        }
                        escaped = false;
                    } else if next == '\\' {
                        escaped = true;
                    } else if next == quote {
                        break;
                    } else {
                        s.push(next);
                    }
                }
                tokens.push(Token::String(s));
            }
            '0'..='9' => {
                let mut num = ch.to_string();
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() || next == '.' {
                        num.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                let n = num.parse::<f64>().map_err(|_| format!("Invalid number: {}", num))?;
                tokens.push(Token::Number(n));
            }
            _ if ch.is_alphabetic() || ch == '_' => {
                let mut ident = ch.to_string();
                while let Some(&next) = chars.peek() {
                    if next.is_alphanumeric() || next == '_' {
                        ident.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                match ident.as_str() {
                    "true" => tokens.push(Token::Keyword("true".to_string())),
                    "false" => tokens.push(Token::Keyword("false".to_string())),
                    "null" => tokens.push(Token::Keyword("null".to_string())),
                    "and" | "or" | "not" => tokens.push(Token::Keyword(ident)),
                    "super" => tokens.push(Token::Keyword("super".to_string())),
                    _ => tokens.push(Token::Ident(ident)),
                }
            }
            _ => return Err(format!("Unexpected character: {}", ch)),
        }
    }
    tokens.push(Token::EOF);
    Ok(tokens)
}

fn parse_or(iter: &mut Peekable<IntoIter<Token>>) -> Result<Expr, String> {
    let mut left = parse_and(iter)?;
    while let Some(Token::Keyword(kw)) = iter.peek() {
        if kw == "or" {
            iter.next();
            let right = parse_and(iter)?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinaryOpKind::Or,
                right: Box::new(right),
            };
        } else {
            break;
        }
    }
    Ok(left)
}

fn parse_and(iter: &mut Peekable<IntoIter<Token>>) -> Result<Expr, String> {
    let mut left = parse_comparison(iter)?;
    while let Some(Token::Keyword(kw)) = iter.peek() {
        if kw == "and" {
            iter.next();
            let right = parse_comparison(iter)?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinaryOpKind::And,
                right: Box::new(right),
            };
        } else {
            break;
        }
    }
    Ok(left)
}

fn parse_comparison(iter: &mut Peekable<IntoIter<Token>>) -> Result<Expr, String> {
    let left = parse_addition(iter)?;
    if let Some(Token::Operator(op)) = iter.peek() {
        let kind = match op.as_str() {
            "==" => Some(BinaryOpKind::Eq),
            "!=" => Some(BinaryOpKind::Ne),
            "<" => Some(BinaryOpKind::Lt),
            "<=" => Some(BinaryOpKind::Le),
            ">" => Some(BinaryOpKind::Gt),
            ">=" => Some(BinaryOpKind::Ge),
            _ => None,
        };
        if let Some(kind) = kind {
            iter.next();
            let right = parse_addition(iter)?;
            return Ok(Expr::BinaryOp {
                left: Box::new(left),
                op: kind,
                right: Box::new(right),
            });
        }
    }
    Ok(left)
}

fn parse_addition(iter: &mut Peekable<IntoIter<Token>>) -> Result<Expr, String> {
    let mut left = parse_multiplication(iter)?;
    while let Some(Token::Operator(op)) = iter.peek() {
        match op.as_str() {
            "+" => {
                iter.next();
                let right = parse_multiplication(iter)?;
                left = Expr::BinaryOp {
                    left: Box::new(left),
                    op: BinaryOpKind::Add,
                    right: Box::new(right),
                };
            }
            "-" => {
                iter.next();
                let right = parse_multiplication(iter)?;
                left = Expr::BinaryOp {
                    left: Box::new(left),
                    op: BinaryOpKind::Sub,
                    right: Box::new(right),
                };
            }
            _ => break,
        }
    }
    Ok(left)
}

fn parse_multiplication(iter: &mut Peekable<IntoIter<Token>>) -> Result<Expr, String> {
    let mut left = parse_unary(iter)?;
    while let Some(Token::Operator(op)) = iter.peek() {
        match op.as_str() {
            "*" => {
                iter.next();
                let right = parse_unary(iter)?;
                left = Expr::BinaryOp {
                    left: Box::new(left),
                    op: BinaryOpKind::Mul,
                    right: Box::new(right),
                };
            }
            "/" => {
                iter.next();
                let right = parse_unary(iter)?;
                left = Expr::BinaryOp {
                    left: Box::new(left),
                    op: BinaryOpKind::Div,
                    right: Box::new(right),
                };
            }
            "%" => {
                iter.next();
                let right = parse_unary(iter)?;
                left = Expr::BinaryOp {
                    left: Box::new(left),
                    op: BinaryOpKind::Mod,
                    right: Box::new(right),
                };
            }
            _ => break,
        }
    }
    Ok(left)
}

fn parse_unary(iter: &mut Peekable<IntoIter<Token>>) -> Result<Expr, String> {
    if let Some(Token::Operator(op)) = iter.peek() {
        if op == "-" {
            iter.next();
            let expr = parse_unary(iter)?;
            return Ok(Expr::UnaryOp {
                op: UnaryOpKind::Neg,
                expr: Box::new(expr),
            });
        }
    }
    if let Some(Token::Keyword(kw)) = iter.peek() {
        if kw == "not" {
            iter.next();
            let expr = parse_unary(iter)?;
            return Ok(Expr::UnaryOp {
                op: UnaryOpKind::Not,
                expr: Box::new(expr),
            });
        }
    }
    parse_postfix(iter)
}

fn parse_postfix(iter: &mut Peekable<IntoIter<Token>>) -> Result<Expr, String> {
    let mut left = parse_primary(iter)?;
    loop {
        match iter.peek() {
            Some(Token::LParen) => {
                iter.next();
                let mut args = Vec::new();
                if let Some(Token::RParen) = iter.peek() {
                    iter.next();
                } else {
                    loop {
                        let arg = parse_or(iter)?;
                        args.push(arg);
                        match iter.next() {
                            Some(Token::Comma) => continue,
                            Some(Token::RParen) => break,
                            _ => return Err("Expected ',' or ')' after argument".to_string()),
                        }
                    }
                }
                match left {
                    Expr::GetAttr { object, attr } => {
                        left = Expr::CallMethod {
                            object,
                            method: attr,
                            args,
                        };
                    }
                    Expr::Variable(name) => {
                        left = Expr::Call { name, args };
                    }
                    _ => return Err("Cannot call non-function or non-method".to_string()),
                }
            }
            Some(Token::LBracket) => {
                iter.next();
                let index = parse_or(iter)?;
                match iter.next() {
                    Some(Token::RBracket) => {}
                    _ => return Err("Expected ']' after index".to_string()),
                }
                left = Expr::Index {
                    array: Box::new(left),
                    index: Box::new(index),
                };
            }
            Some(Token::Dot) => {
                iter.next();
                match iter.next() {
                    Some(Token::Ident(attr)) => {
                        left = Expr::GetAttr {
                            object: Box::new(left),
                            attr,
                        };
                    }
                    _ => return Err("Expected attribute name after '.'".to_string()),
                }
            }
            _ => break,
        }
    }
    Ok(left)
}

fn parse_primary(iter: &mut Peekable<IntoIter<Token>>) -> Result<Expr, String> {
    match iter.next() {
        Some(Token::Number(n)) => Ok(Expr::Number(n)),
        Some(Token::String(s)) => Ok(Expr::String(s)),
        Some(Token::Keyword(kw)) => match kw.as_str() {
            "true" => Ok(Expr::Boolean(true)),
            "false" => Ok(Expr::Boolean(false)),
            "null" => Ok(Expr::Null),
            "super" => Ok(Expr::Super { args: vec![] }),
            _ => Err(format!("Unexpected keyword: {}", kw)),
        },
        Some(Token::Ident(name)) => Ok(Expr::Variable(name)),
        Some(Token::LParen) => {
            let expr = parse_or(iter)?;
            match iter.next() {
                Some(Token::RParen) => Ok(expr),
                _ => Err("Expected ')'".to_string()),
            }
        }
        Some(Token::EOF) => Err("Unexpected end of expression".to_string()),
        _ => Err("Unexpected token".to_string()),
    }
}

// ---------- Вспомогательные функции ----------
fn count_indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

fn is_comment(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//") || trimmed.starts_with('#')
}