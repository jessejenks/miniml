use crate::ast;
use std::fmt::Display;

#[derive(Debug, PartialEq, Clone)]
pub enum Expr {
    Unit,
    Bool(bool),
    Char(char),
    Int(i64),
    String(String),
    BoundVariable(i64),
    FreeVariable(String),
    Application(Box<Expr>, Box<Expr>),
    Abstraction(Box<Expr>),
    LetIn(bool, Box<Expr>, Box<Expr>),
    IfElse(Box<Expr>, Box<Expr>, Box<Expr>),
    Unary(ast::UnaryOp, Box<Expr>),
    Binary(ast::BinaryOp, Box<Expr>, Box<Expr>),
}

impl Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Expr::Unit => write!(f, "()"),
            Expr::Bool(b) => write!(f, "{b}"),
            Expr::Char(c) => write!(f, "'{c}'"),
            Expr::Int(n) => write!(f, "{n}"),
            Expr::String(s) => write!(f, "\"{s}\""),
            Expr::FreeVariable(x) => write!(f, "{x}"),
            Expr::BoundVariable(i) => write!(f, "#{i}"),
            Expr::Application(left, right) => write!(f, "({left} {right})"),
            Expr::Abstraction(body) => write!(f, "(λ.{body})"),
            Expr::LetIn(is_rec, expr, body) => write!(
                f,
                "(let {}{expr} in {body})",
                if *is_rec { "rec " } else { "" },
            ),
            Expr::IfElse(condition, consequence, alternative) => {
                write!(f, "(if {condition} then {consequence} else {alternative})")
            }
            Expr::Unary(op, expr) => write!(f, "({op} {expr})"),
            Expr::Binary(op, left, right) => write!(f, "({op} {left} {right})"),
        }
    }
}

fn to_spanless(expr: &ast::Expr) -> Expr {
    match &expr.kind {
        ast::ExprKind::Unit => Expr::Unit,
        ast::ExprKind::Bool(b) => Expr::Bool(*b),
        ast::ExprKind::Char(c) => Expr::Char(*c),
        ast::ExprKind::Int(n) => Expr::Int(*n),
        ast::ExprKind::String(s) => Expr::String(s.clone()),
        ast::ExprKind::BoundVariable(i) => Expr::BoundVariable(*i),
        ast::ExprKind::FreeVariable(x) => Expr::FreeVariable(x.clone()),
        ast::ExprKind::Application(left, right) => {
            Expr::Application(Box::new(to_spanless(left)), Box::new(to_spanless(right)))
        }
        ast::ExprKind::Abstraction(_, body) => Expr::Abstraction(Box::new(to_spanless(body))),
        ast::ExprKind::LetIn(is_rec, _, expr, body) => Expr::LetIn(
            *is_rec,
            Box::new(to_spanless(expr)),
            Box::new(to_spanless(body)),
        ),
        ast::ExprKind::IfElse(condition, consequence, alternative) => Expr::IfElse(
            Box::new(to_spanless(condition)),
            Box::new(to_spanless(consequence)),
            Box::new(to_spanless(alternative)),
        ),
        ast::ExprKind::Unary(op, expr) => Expr::Unary(*op, Box::new(to_spanless(expr))),
        ast::ExprKind::Binary(op, left, right) => Expr::Binary(
            *op,
            Box::new(to_spanless(left)),
            Box::new(to_spanless(right)),
        ),
    }
}

fn shift(i: i64, c: i64, expr: Expr) -> Expr {
    match expr {
        Expr::Unit
        | Expr::Bool(_)
        | Expr::Char(_)
        | Expr::Int(_)
        | Expr::String(_)
        | Expr::FreeVariable(_) => expr,
        Expr::BoundVariable(index) => {
            if index < c {
                expr
            } else {
                Expr::BoundVariable(index + i)
            }
        }
        Expr::Application(left, right) => {
            Expr::Application(Box::new(shift(i, c, *left)), Box::new(shift(i, c, *right)))
        }
        Expr::Abstraction(body) => Expr::Abstraction(Box::new(shift(i, c + 1, *body))),
        Expr::LetIn(is_rec, e, b) => Expr::LetIn(
            is_rec,
            Box::new(shift(i, if is_rec { c + 1 } else { c }, *e)),
            Box::new(shift(i, c + 1, *b)),
        ),
        Expr::IfElse(condition, consequence, alternative) => Expr::IfElse(
            Box::new(shift(i, c, *condition)),
            Box::new(shift(i, c, *consequence)),
            Box::new(shift(i, c, *alternative)),
        ),
        Expr::Unary(op, expr) => Expr::Unary(op, Box::new(shift(i, c, *expr))),
        Expr::Binary(op, left, right) => Expr::Binary(
            op,
            Box::new(shift(i, c, *left)),
            Box::new(shift(i, c, *right)),
        ),
    }
}

fn substitute(t: Expr, n: i64, expr: Expr) -> Expr {
    match t {
        Expr::Unit
        | Expr::Bool(_)
        | Expr::Char(_)
        | Expr::Int(_)
        | Expr::String(_)
        | Expr::FreeVariable(_) => t,
        Expr::BoundVariable(index) => {
            if index == n {
                expr
            } else {
                t
            }
        }
        Expr::Application(left, right) => Expr::Application(
            Box::new(substitute(*left, n, expr.clone())),
            Box::new(substitute(*right, n, expr)),
        ),
        Expr::Abstraction(body) => {
            Expr::Abstraction(Box::new(substitute(*body, n + 1, shift(1, 0, expr))))
        }
        Expr::LetIn(is_rec, e, b) => {
            let shifted_expr = shift(1, 0, expr.clone());
            Expr::LetIn(
                is_rec,
                if is_rec {
                    Box::new(substitute(*e, n + 1, shifted_expr.clone()))
                } else {
                    Box::new(substitute(*e, n, expr))
                },
                Box::new(substitute(*b, n + 1, shifted_expr)),
            )
        }
        Expr::IfElse(condition, consequence, alternative) => Expr::IfElse(
            Box::new(substitute(*condition, n, expr.clone())),
            Box::new(substitute(*consequence, n, expr.clone())),
            Box::new(substitute(*alternative, n, expr)),
        ),
        Expr::Unary(op, e) => Expr::Unary(op, Box::new(substitute(*e, n, expr))),
        Expr::Binary(op, left, right) => Expr::Binary(
            op,
            Box::new(substitute(*left, n, expr.clone())),
            Box::new(substitute(*right, n, expr)),
        ),
    }
}

fn beta(body: Expr, right: Expr) -> Expr {
    shift(-1, 0, substitute(body, 0, shift(1, 0, right)))
}

// call by value
pub fn interpret(expr: &ast::Expr) -> Expr {
    let spanless = to_spanless(expr);
    cbv(spanless)
}

fn cbv(expr: Expr) -> Expr {
    match expr {
        Expr::Application(left, right) => {
            // nu
            let left_val = cbv(*left);
            // mu
            let right_val = cbv(*right);
            match left_val {
                Expr::Abstraction(body) => cbv(beta(*body, right_val)),
                Expr::FreeVariable(n) if n == "print" => {
                    println!("{right_val}");
                    Expr::Unit
                }
                l => Expr::Application(Box::new(l), Box::new(right_val)),
            }
        }
        Expr::LetIn(is_rec, expr, body) => {
            if is_rec {
                // fix
                let unfolded = beta(
                    *expr.clone(),
                    Expr::LetIn(true, expr, Box::new(Expr::BoundVariable(0))),
                );
                if matches!(*body, Expr::BoundVariable(0)) {
                    cbv(unfolded)
                } else {
                    cbv(Expr::LetIn(false, Box::new(unfolded), body))
                }
            } else {
                // zeta
                cbv(beta(*body, cbv(*expr)))
            }
        }
        Expr::IfElse(condition, consequence, alternative) => {
            let cond = cbv(*condition);
            if let Expr::Bool(b) = cond {
                // iota
                cbv(if b { *consequence } else { *alternative })
            } else {
                Expr::IfElse(Box::new(cond), consequence, alternative)
            }
        }
        Expr::Unary(op, expr) => {
            let child = cbv(*expr);
            match (op, child) {
                (ast::UnaryOp::Negate, Expr::Int(n)) => Expr::Int(-n),
                (ast::UnaryOp::BitNot, Expr::Int(n)) => Expr::Int(-1 ^ n),
                (ast::UnaryOp::Not, Expr::Bool(b)) => Expr::Bool(!b),
                (o, c) => Expr::Unary(o, Box::new(c)),
            }
        }
        Expr::Binary(op, left, right) => {
            let l_child = cbv(*left);
            // short-circuiting
            match (op, &l_child) {
                (ast::BinaryOp::And, Expr::Bool(false)) => {
                    return Expr::Bool(false);
                }
                (ast::BinaryOp::Or, Expr::Bool(true)) => {
                    return Expr::Bool(true);
                }
                _ => {}
            }
            let r_child = cbv(*right);
            match (op, l_child, r_child) {
                (ast::BinaryOp::Add, Expr::Int(a), Expr::Int(b)) => Expr::Int(a + b),
                (ast::BinaryOp::Sub, Expr::Int(a), Expr::Int(b)) => Expr::Int(a - b),
                (ast::BinaryOp::Mul, Expr::Int(a), Expr::Int(b)) => Expr::Int(a * b),
                (ast::BinaryOp::Div, Expr::Int(a), Expr::Int(b)) if b != 0 => Expr::Int(a / b),
                (ast::BinaryOp::Lt, Expr::Int(a), Expr::Int(b)) => Expr::Bool(a < b),
                (ast::BinaryOp::Gt, Expr::Int(a), Expr::Int(b)) => Expr::Bool(a > b),
                (ast::BinaryOp::Lte, Expr::Int(a), Expr::Int(b)) => Expr::Bool(a <= b),
                (ast::BinaryOp::Gte, Expr::Int(a), Expr::Int(b)) => Expr::Bool(a >= b),
                (ast::BinaryOp::Eq, Expr::Unit, Expr::Unit) => Expr::Bool(true),
                (ast::BinaryOp::Eq, Expr::Bool(a), Expr::Bool(b)) => Expr::Bool(a == b),
                (ast::BinaryOp::Eq, Expr::Char(a), Expr::Char(b)) => Expr::Bool(a == b),
                (ast::BinaryOp::Eq, Expr::Int(a), Expr::Int(b)) => Expr::Bool(a == b),
                (ast::BinaryOp::Eq, Expr::String(a), Expr::String(b)) => Expr::Bool(a == b),
                (ast::BinaryOp::Concat, Expr::String(a), Expr::String(b)) => Expr::String(a + &b),
                (ast::BinaryOp::BitAnd, Expr::Int(a), Expr::Int(b)) => Expr::Int(a & b),
                (ast::BinaryOp::BitOr, Expr::Int(a), Expr::Int(b)) => Expr::Int(a | b),
                (ast::BinaryOp::And, Expr::Bool(a), Expr::Bool(b)) => Expr::Bool(a && b),
                (ast::BinaryOp::Or, Expr::Bool(a), Expr::Bool(b)) => Expr::Bool(a || b),
                (op, l, r) => Expr::Binary(op, Box::new(l), Box::new(r)),
            }
        }
        x => x,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elaborate::surface_to_core;
    use crate::parser::Parser;

    fn interp(source: &str) -> Expr {
        interpret(&surface_to_core(Parser::parse(source).unwrap()))
    }

    #[test]
    fn interprets_expressions() {
        assert_eq!(interp("()").to_string(), "()");
        assert_eq!(interp("true").to_string(), "true");
        assert_eq!(interp("false").to_string(), "false");
        assert_eq!(interp("1").to_string(), "1");
        assert_eq!(interp("1 == 1").to_string(), "true");
        assert_eq!(interp("'a' == 'a'").to_string(), "true");
        assert_eq!(interp("'a' == 'b'").to_string(), "false");
        assert_eq!(interp(r#""abc" == "abc""#).to_string(), "true");
        assert_eq!(interp(r#""abc" == "bcd""#).to_string(), "false");
        assert_eq!(interp(r#""a" ++ "b""#).to_string(), r#""ab""#);
        assert_eq!(interp("fun x => x").to_string(), "(λ.#0)");
        assert_eq!(interp("fun x => fun x => x").to_string(), "(λ.(λ.#0))");
        assert_eq!(interp("(fun _ => b) a").to_string(), "b");
        assert_eq!(interp("a; b").to_string(), "b");
        assert_eq!(interp("1 + 2 * 3 - 4").to_string(), "3");
        assert_eq!(
            interp("let I = fun x => x; let K = fun x => fun _ => x; I K").to_string(),
            "(λ.(λ.#1))"
        );
        assert_eq!(interp("if true then 1 else 2").to_string(), "1");
        assert_eq!(interp("if false then 1 else 2").to_string(), "2");
        assert_eq!(
            interp(
                "
            # λ.(λ.#1)
            let K = fun x => fun _ => x;
            # λ.λ.λ.#2 #0 (#1 #0)
            let S = fun x => fun y => fun z => x z (y z);
               S                     K        K
            # (λ.λ.λ.#2 #0 (#1 #0)) (λ.λ.#1) (λ.λ.#1)
            # (λ.λ.(λ.λ.#1) #0 (#1 #0))      (λ.λ.#1)
            # (λ.(λ.λ.#1) #0 ((λ.λ.#1) #0))
            "
            )
            .to_string(),
            "(λ.(((λ.(λ.#1)) #0) ((λ.(λ.#1)) #0)))"
        );
        assert_eq!(
            interp(
                "
            let rec f = fun n => if n < 2 then 1 else n * f (n - 1); f
            # f = let rec λ.if (< #0 2) then 1 else (* #0 (#1 (- #0 1)))
            # |
            # +--------------------------------+ fix
            #                                  |
            # λ.if (< #0 2) then 1 else (* #0 (f (- #0 1)))
            "
            )
            .to_string(),
            "(λ.(if (< #0 2) then 1 else (* #0 ((let rec (λ.(if (< #0 2) then 1 else (* #0 (#1 (- #0 1))))) in #0) (- #0 1)))))"
        );
        assert_eq!(interp("print 3; 4").to_string(), "4");
        assert_eq!(interp("let p = print; p 1; p 2; 5").to_string(), "5");
        assert_eq!(interp("(fun f => { f 6; 7 }) print").to_string(), "7");
    }
}
