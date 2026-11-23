use crate::{ast, typecheck};
use std::fmt::Display;

#[derive(PartialEq, Clone)]
pub enum Pattern {
    Literal(ast::Literal),
    Wild,
    Variable,
    Tuple(Vec<Pattern>),
}

impl Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pattern::Literal(lit) => lit.fmt(f),
            Pattern::Wild => write!(f, "_"),
            Pattern::Variable => write!(f, "v"),
            Pattern::Tuple(patterns) => {
                write!(f, "(")?;
                for (i, p) in patterns.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ")")
            }
        }
    }
}

#[derive(PartialEq, Clone)]
pub enum Expr {
    Literal(ast::Literal),
    BoundVariable(i64),
    FreeVariable(String),
    Application(Box<Expr>, Box<Expr>),
    Abstraction(Box<Expr>),
    LetIn(bool, Pattern, Box<Expr>, Box<Expr>),
    IfElse(Box<Expr>, Box<Expr>, Box<Expr>),
    Unary(ast::UnaryOp, Box<Expr>),
    Binary(ast::BinaryOp, Box<Expr>, Box<Expr>),
    Tuple(Vec<Expr>),
    Match(Box<Expr>, Vec<(Pattern, Expr)>),
    Constructor(typecheck::TypeId, usize, Vec<Expr>),
}

impl Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Expr::Literal(lit) => lit.fmt(f),
            Expr::FreeVariable(x) => write!(f, "{x}"),
            Expr::BoundVariable(i) => write!(f, "#{i}"),
            Expr::Application(left, right) => write!(f, "({left} {right})"),
            Expr::Abstraction(body) => write!(f, "(λ.{body})"),
            Expr::LetIn(is_rec, _, expr, body) => write!(
                f,
                "(let {}{expr} in {body})",
                if *is_rec { "rec " } else { "" },
            ),
            Expr::IfElse(condition, consequence, alternative) => {
                write!(f, "(if {condition} then {consequence} else {alternative})")
            }
            Expr::Unary(op, expr) => write!(f, "({op} {expr})"),
            Expr::Binary(op, left, right) => write!(f, "({op} {left} {right})"),
            Expr::Tuple(elements) => {
                write!(f, "(")?;
                for (i, elt) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{elt}")?;
                }
                write!(f, ")")
            }
            Expr::Match(scrutinee, arms) => {
                write!(f, "(match {scrutinee}")?;
                write!(f, " {{ ")?;
                for (i, (pattern, expr)) in arms.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{pattern} => {expr}")?;
                }
                write!(f, " }})")
            }
            Expr::Constructor(id, index, args) => {
                write!(f, "T_{}::[{index}]", id.0)?;
                if !args.is_empty() {
                    write!(f, "(")?;
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{arg}")?;
                    }
                    write!(f, ")")?;
                }
                Ok(())
            }
        }
    }
}

fn to_spanless(expr: &ast::Expr) -> Expr {
    match &expr.kind {
        ast::ExprKind::Literal(lit) => Expr::Literal(lit.clone()),
        ast::ExprKind::BoundVariable(i) => Expr::BoundVariable(*i),
        ast::ExprKind::FreeVariable(x) => Expr::FreeVariable(x.clone()),
        ast::ExprKind::Application(left, right) => {
            Expr::Application(Box::new(to_spanless(left)), Box::new(to_spanless(right)))
        }
        ast::ExprKind::Abstraction(_, body) => Expr::Abstraction(Box::new(to_spanless(body))),
        ast::ExprKind::LetIn(is_rec, pattern, expr, body) => Expr::LetIn(
            *is_rec,
            to_spanless_pattern(pattern),
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
        ast::ExprKind::Tuple(elements) => Expr::Tuple(elements.iter().map(to_spanless).collect()),
        ast::ExprKind::Constructor(id, index, args) => {
            Expr::Constructor(*id, *index, args.iter().map(to_spanless).collect())
        }
        ast::ExprKind::Match(scrutinee, arms) => Expr::Match(
            Box::new(to_spanless(scrutinee)),
            arms.iter()
                .map(|(p, e)| (to_spanless_pattern(p), to_spanless(e)))
                .collect(),
        ),
    }
}

fn to_spanless_pattern(pattern: &ast::Pattern) -> Pattern {
    match &pattern.kind {
        ast::PatternKind::Literal(lit) => Pattern::Literal(lit.clone()),
        ast::PatternKind::Wild => Pattern::Wild,
        ast::PatternKind::Variable => Pattern::Variable,
        ast::PatternKind::Tuple(patterns) => {
            Pattern::Tuple(patterns.iter().map(to_spanless_pattern).collect())
        }
    }
}

fn leaf_count(pattern: &Pattern) -> i64 {
    match pattern {
        Pattern::Literal(_) => 0,
        Pattern::Wild | Pattern::Variable => 1,
        Pattern::Tuple(patterns) => patterns.iter().fold(0, |acc, curr| acc + leaf_count(curr)),
    }
}

fn get_leaves(pattern: &Pattern, value: &Expr, out: &mut Vec<Expr>) -> bool {
    match (pattern, value) {
        (Pattern::Wild | Pattern::Variable, v) => {
            out.push(v.clone());
            true
        }
        (Pattern::Literal(lit), Expr::Literal(v)) => lit == v,
        (Pattern::Tuple(patterns), Expr::Tuple(elements)) if patterns.len() == elements.len() => {
            patterns
                .iter()
                .zip(elements)
                .all(|(p, v)| get_leaves(p, v, out))
        }
        _ => false,
    }
}

fn shift(i: i64, c: i64, expr: Expr) -> Expr {
    match expr {
        Expr::Literal(_) | Expr::FreeVariable(_) => expr,
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
        Expr::LetIn(is_rec, pattern, e, b) => {
            let count = leaf_count(&pattern);
            Expr::LetIn(
                is_rec,
                pattern,
                Box::new(shift(i, if is_rec { c + 1 } else { c }, *e)),
                Box::new(shift(i, c + count, *b)),
            )
        }
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
        Expr::Tuple(elements) => {
            Expr::Tuple(elements.into_iter().map(|e| shift(i, c, e)).collect())
        }
        Expr::Match(scrutinee, arms) => Expr::Match(
            Box::new(shift(i, c, *scrutinee)),
            arms.into_iter()
                .map(|(p, e)| {
                    let count = leaf_count(&p);
                    (p, shift(i, c + count, e))
                })
                .collect(),
        ),
        Expr::Constructor(id, index, args) => Expr::Constructor(
            id,
            index,
            args.into_iter().map(|e| shift(i, c, e)).collect(),
        ),
    }
}

fn substitute(t: Expr, n: i64, expr: Expr) -> Expr {
    match t {
        Expr::Literal(_) | Expr::FreeVariable(_) => t,
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
        Expr::LetIn(is_rec, pattern, e, b) => {
            let count = leaf_count(&pattern);
            let shifted_expr = shift(count, 0, expr.clone());
            Expr::LetIn(
                is_rec,
                pattern,
                if is_rec {
                    Box::new(substitute(*e, n + 1, shifted_expr.clone()))
                } else {
                    Box::new(substitute(*e, n, expr))
                },
                Box::new(substitute(*b, n + count, shifted_expr)),
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
        Expr::Tuple(elements) => Expr::Tuple(
            elements
                .into_iter()
                .map(|e| substitute(e, n, expr.clone()))
                .collect(),
        ),
        Expr::Match(scrutinee, arms) => {
            let new_arms = arms
                .into_iter()
                .map(|(p, e)| {
                    let count = leaf_count(&p);
                    (p, substitute(e, n + count, shift(count, 0, expr.clone())))
                })
                .collect();
            Expr::Match(Box::new(substitute(*scrutinee, n, expr)), new_arms)
        }
        Expr::Constructor(id, index, args) => Expr::Constructor(
            id,
            index,
            args.into_iter()
                .map(|e| substitute(e, n, expr.clone()))
                .collect(),
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
                Expr::FreeVariable(name) => match (name.as_str(), right_val) {
                    ("print", Expr::Literal(ast::Literal::String(s))) => {
                        println!("{s}");
                        Expr::Literal(ast::Literal::Unit)
                    }
                    ("len", Expr::Literal(ast::Literal::String(s))) => {
                        Expr::Literal(ast::Literal::Int(s.len().try_into().unwrap()))
                    }
                    ("unitToStr", Expr::Literal(ast::Literal::Unit)) => {
                        Expr::Literal(ast::Literal::String("()".to_string()))
                    }
                    ("boolToStr", Expr::Literal(ast::Literal::Bool(b))) => {
                        Expr::Literal(ast::Literal::String(format!("{b}")))
                    }
                    ("charToStr", Expr::Literal(ast::Literal::Char(c))) => {
                        Expr::Literal(ast::Literal::String(format!("{c}")))
                    }
                    ("intToStr", Expr::Literal(ast::Literal::Int(n))) => {
                        Expr::Literal(ast::Literal::String(format!("{n}")))
                    }
                    (_, r) => stuck_application(Expr::FreeVariable(name), r),
                },
                l => stuck_application(l, right_val),
            }
        }
        Expr::LetIn(is_rec, pattern, expr, body) => {
            if is_rec {
                // fix
                let unfolded = beta(
                    *expr.clone(),
                    Expr::LetIn(
                        true,
                        pattern.clone(),
                        expr,
                        Box::new(Expr::BoundVariable(0)),
                    ),
                );
                if matches!(*body, Expr::BoundVariable(0)) {
                    cbv(unfolded)
                } else {
                    cbv(Expr::LetIn(false, pattern, Box::new(unfolded), body))
                }
            } else {
                // zeta
                let expr_value = cbv(*expr);
                let mut leaves = Vec::new();
                if get_leaves(&pattern, &expr_value, &mut leaves) {
                    cbv(leaves.into_iter().rev().fold(*body, beta))
                } else {
                    Expr::LetIn(false, pattern, Box::new(expr_value), body)
                }
            }
        }
        Expr::IfElse(condition, consequence, alternative) => {
            let cond = cbv(*condition);
            if let Expr::Literal(ast::Literal::Bool(b)) = cond {
                // iota
                cbv(if b { *consequence } else { *alternative })
            } else {
                Expr::IfElse(Box::new(cond), consequence, alternative)
            }
        }
        Expr::Unary(op, expr) => {
            let child = cbv(*expr);
            match (op, child) {
                (ast::UnaryOp::Negate, Expr::Literal(ast::Literal::Int(n))) => {
                    Expr::Literal(ast::Literal::Int(-n))
                }
                (ast::UnaryOp::BitNot, Expr::Literal(ast::Literal::Int(n))) => {
                    Expr::Literal(ast::Literal::Int(-1 ^ n))
                }
                (ast::UnaryOp::Not, Expr::Literal(ast::Literal::Bool(b))) => {
                    Expr::Literal(ast::Literal::Bool(!b))
                }
                (o, c) => Expr::Unary(o, Box::new(c)),
            }
        }
        Expr::Binary(op, left, right) => {
            let l_child = cbv(*left);
            // short-circuiting
            match (op, &l_child) {
                (ast::BinaryOp::And, Expr::Literal(ast::Literal::Bool(false))) => {
                    return Expr::Literal(ast::Literal::Bool(false));
                }
                (ast::BinaryOp::Or, Expr::Literal(ast::Literal::Bool(true))) => {
                    return Expr::Literal(ast::Literal::Bool(true));
                }
                _ => {}
            }
            let r_child = cbv(*right);
            match (op, l_child, r_child) {
                (
                    ast::BinaryOp::Add,
                    Expr::Literal(ast::Literal::Int(a)),
                    Expr::Literal(ast::Literal::Int(b)),
                ) => Expr::Literal(ast::Literal::Int(a + b)),
                (
                    ast::BinaryOp::Sub,
                    Expr::Literal(ast::Literal::Int(a)),
                    Expr::Literal(ast::Literal::Int(b)),
                ) => Expr::Literal(ast::Literal::Int(a - b)),
                (
                    ast::BinaryOp::Mul,
                    Expr::Literal(ast::Literal::Int(a)),
                    Expr::Literal(ast::Literal::Int(b)),
                ) => Expr::Literal(ast::Literal::Int(a * b)),
                (
                    ast::BinaryOp::Div,
                    Expr::Literal(ast::Literal::Int(a)),
                    Expr::Literal(ast::Literal::Int(b)),
                ) if b != 0 => Expr::Literal(ast::Literal::Int(a / b)),
                (
                    ast::BinaryOp::Lt,
                    Expr::Literal(ast::Literal::Int(a)),
                    Expr::Literal(ast::Literal::Int(b)),
                ) => Expr::Literal(ast::Literal::Bool(a < b)),
                (
                    ast::BinaryOp::Gt,
                    Expr::Literal(ast::Literal::Int(a)),
                    Expr::Literal(ast::Literal::Int(b)),
                ) => Expr::Literal(ast::Literal::Bool(a > b)),
                (
                    ast::BinaryOp::Lte,
                    Expr::Literal(ast::Literal::Int(a)),
                    Expr::Literal(ast::Literal::Int(b)),
                ) => Expr::Literal(ast::Literal::Bool(a <= b)),
                (
                    ast::BinaryOp::Gte,
                    Expr::Literal(ast::Literal::Int(a)),
                    Expr::Literal(ast::Literal::Int(b)),
                ) => Expr::Literal(ast::Literal::Bool(a >= b)),
                (ast::BinaryOp::Eq, a, b) => {
                    if let Some(are_equal) = values_equal(&a, &b) {
                        Expr::Literal(ast::Literal::Bool(are_equal))
                    } else {
                        Expr::Binary(ast::BinaryOp::Eq, Box::new(a), Box::new(b))
                    }
                }
                (
                    ast::BinaryOp::Concat,
                    Expr::Literal(ast::Literal::String(a)),
                    Expr::Literal(ast::Literal::String(b)),
                ) => Expr::Literal(ast::Literal::String(a + &b)),
                (
                    ast::BinaryOp::BitAnd,
                    Expr::Literal(ast::Literal::Int(a)),
                    Expr::Literal(ast::Literal::Int(b)),
                ) => Expr::Literal(ast::Literal::Int(a & b)),
                (
                    ast::BinaryOp::BitOr,
                    Expr::Literal(ast::Literal::Int(a)),
                    Expr::Literal(ast::Literal::Int(b)),
                ) => Expr::Literal(ast::Literal::Int(a | b)),
                (
                    ast::BinaryOp::And,
                    Expr::Literal(ast::Literal::Bool(a)),
                    Expr::Literal(ast::Literal::Bool(b)),
                ) => Expr::Literal(ast::Literal::Bool(a && b)),
                (
                    ast::BinaryOp::Or,
                    Expr::Literal(ast::Literal::Bool(a)),
                    Expr::Literal(ast::Literal::Bool(b)),
                ) => Expr::Literal(ast::Literal::Bool(a || b)),
                (op, l, r) => Expr::Binary(op, Box::new(l), Box::new(r)),
            }
        }
        Expr::Tuple(elements) => Expr::Tuple(elements.into_iter().map(cbv).collect()),
        Expr::Constructor(id, index, args) => {
            Expr::Constructor(id, index, args.into_iter().map(cbv).collect())
        }
        Expr::Match(scrutinee, arms) => {
            let v = cbv(*scrutinee);
            for (p, e) in &arms {
                let mut leaves = Vec::new();
                if get_leaves(p, &v, &mut leaves) {
                    return cbv(leaves.into_iter().rev().fold(e.clone(), beta));
                }
            }
            Expr::Match(Box::new(v), arms)
        }
        x => x,
    }
}

fn values_equal(a: &Expr, b: &Expr) -> Option<bool> {
    match (a, b) {
        (Expr::Literal(ast::Literal::Unit), Expr::Literal(ast::Literal::Unit)) => Some(true),
        (Expr::Literal(ast::Literal::Bool(a)), Expr::Literal(ast::Literal::Bool(b))) => {
            Some(a == b)
        }
        (Expr::Literal(ast::Literal::Char(a)), Expr::Literal(ast::Literal::Char(b))) => {
            Some(a == b)
        }
        (Expr::Literal(ast::Literal::Int(a)), Expr::Literal(ast::Literal::Int(b))) => Some(a == b),
        (Expr::Literal(ast::Literal::String(a)), Expr::Literal(ast::Literal::String(b))) => {
            Some(a == b)
        }
        (Expr::Tuple(a), Expr::Tuple(b)) if a.len() == b.len() => all_values_equal(a, b),
        (Expr::Constructor(a_id, a_index, a_args), Expr::Constructor(b_id, b_index, b_args))
            if a_id == b_id =>
        {
            if a_index != b_index {
                return Some(false);
            }
            all_values_equal(a_args, b_args)
        }
        _ => None,
    }
}

fn all_values_equal(a: &[Expr], b: &[Expr]) -> Option<bool> {
    for (a, b) in a.iter().zip(b) {
        match values_equal(a, b) {
            None => return None,
            Some(false) => return Some(false),
            _ => {}
        }
    }
    Some(true)
}

fn stuck_application(left: Expr, right: Expr) -> Expr {
    Expr::Application(Box::new(left), Box::new(right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::front_end;

    fn interp(source: &str) -> Expr {
        interpret(&front_end(source).0)
    }

    #[test]
    fn interprets_constructors() {
        let interp = |source| {
            interp(&format!(
                "type Option a = None | Some a;
                 type List a = Nil | Cons a (List a);
                 type Pair = Pair Int Int;
                 {source}"
            ))
            .to_string()
        };
        assert_eq!(interp("Option::None"), "T_0::[0]");
        assert_eq!(interp("Option::Some (1 + 2)"), "T_0::[1](3)");
        assert_eq!(
            interp("List::Cons 1 (List::Cons 2 List::Nil)"),
            "T_1::[1](1, T_1::[1](2, T_1::[0]))"
        );
        // partial application
        assert_eq!(interp("Pair::Pair 1"), "(λ.T_2::[0](1, #0))");
        assert_eq!(
            interp("let p = Pair::Pair 1; (p 2, p 3)"),
            "(T_2::[0](1, 2), T_2::[0](1, 3))"
        );
        assert_eq!(
            interp("let x = 5; let f = fun y => Option::Some (x, y); f 6"),
            "T_0::[1]((5, 6))"
        );
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
        assert_eq!(interp("(1 + 2, 3)").to_string(), "(3, 3)");
        // tuples and destructuring
        assert_eq!(interp("let _ = 1; 2").to_string(), "2");
        assert_eq!(interp("let x = 1; (x, x)").to_string(), "(1, 1)");
        assert_eq!(interp("let (x, y) = (1, 2); 1 + 2").to_string(), "3");
        assert_eq!(
            interp("let (x, (y, z)) = (1, (2, 3)); x + y + z").to_string(),
            "6"
        );
        assert_eq!(
            // pattern doesn't match, so stuck
            interp("let (x, (y, z)) = (1, ()); x + y + z").to_string(),
            "(let (1, ()) in (+ (+ #2 #1) #0))"
        );
        // pattern matching
        assert_eq!(interp("match (1, 2) { (x, y) => x + y }").to_string(), "3");
        assert_eq!(
            interp("match (\"a\", 3) { (\"b\", _) => 0, (\"a\", n) => n * 2 }").to_string(),
            "6"
        );
        assert_eq!(
            interp("let y = 10; match (1, (2, true)) { (1, (_, false)) => 0, (x, (2, true)) => x + y, _ => 99 }").to_string(),
            "11",
        );
        assert_eq!(
            // stuck, non-exhaustive match
            interp("match 1 { 2 => 3 }").to_string(),
            "(match 1 { 2 => 3 })"
        );
        // builtins
        assert_eq!(interp("print ()").to_string(), "(print ())");
        assert_eq!(interp("print 3").to_string(), "(print 3)");
        assert_eq!(interp("print \"3\"").to_string(), "()");
        assert_eq!(interp("print \"3\"; 4").to_string(), "4");
        assert_eq!(
            interp("let p = print; p \"one\"; p \"two\"; \"three\"").to_string(),
            "\"three\""
        );
        assert_eq!(
            interp("(fun f => { f \"debug\"; 7 }) print").to_string(),
            "7"
        );
        assert_eq!(interp("len 'a'").to_string(), "(len 'a')");
        assert_eq!(interp("len \"a\"").to_string(), "1");
        assert_eq!(interp("len \"abcde\"").to_string(), "5");
        assert_eq!(interp("unitToStr ()").to_string(), "\"()\"");
        assert_eq!(interp("boolToStr true").to_string(), "\"true\"");
        assert_eq!(interp("charToStr 'a'").to_string(), "\"a\"");
        assert_eq!(interp("intToStr 123").to_string(), "\"123\"");
        // tuple/constructor equality
        assert_eq!(interp("(1, 2, 3) == (1, 2, 3)").to_string(), "true");
        assert_eq!(interp("(1, 2, 3) == (1, 2, 4)").to_string(), "false");
        assert_eq!(
            interp("(1, 2, 3) == (1, 2)").to_string(),
            "(== (1, 2, 3) (1, 2))"
        );
        assert_eq!(
            interp("(1, 2, 3) == (1, 2, 3, 4)").to_string(),
            "(== (1, 2, 3) (1, 2, 3, 4))"
        );
        assert_eq!(
            interp("(1, 2, 3) == (1, '2', ())").to_string(),
            "(== (1, 2, 3) (1, '2', ()))"
        );
        assert_eq!(
            interp(
                "
            type Triple a b c
                = Triple a b c;
            Triple::Triple 1 2 3 == Triple::Triple 1 2 3"
            )
            .to_string(),
            "true"
        );
        assert_eq!(
            interp(
                "
            type Triple a b c
                = Triple a b c;
            Triple::Triple 1 2 3 == Triple::Triple 1 2 4"
            )
            .to_string(),
            "false"
        );
        assert_eq!(
            interp(
                "
            type Option t
                = None
                | Some t;
            Option::Some 1 == Option::None"
            )
            .to_string(),
            "false"
        );
        assert_eq!(
            interp(
                "
            type Option t
                = None
                | Some t;
            type Either a b
                = Left a
                | Right b;
            Option::Some 'a' == Either::Right 'a'"
            )
            .to_string(),
            "(== T_0::[1]('a') T_1::[1]('a'))"
        );
        assert_eq!(
            interp(
                "
            type List t
                = Nil
                | Cons t (List t);
            List::Cons 1 List::Nil == List::Nil"
            )
            .to_string(),
            "false"
        );
    }
}
