use miette::Diagnostic;
use miette::SourceSpan;
use thiserror::Error;

use crate::ast;
use crate::lexer;
use crate::surface;

#[derive(Debug, Error, Diagnostic)]
pub enum ElaborationError {
    #[error("variable {x} is bound multiple times")]
    #[diagnostic(code(elaborator::multiple_bindings))]
    MultipleBindings {
        x: String,
        #[label("this variable has already appeared")]
        span: SourceSpan,
    },
    #[error("cannot destructure in recursive let")]
    #[diagnostic(code(elaborator::destructured_let_rec))]
    DestructuredLetRec {
        #[label("cannot destructure here")]
        span: SourceSpan,
    },
    #[error("this pattern cannot contain literal values")]
    #[diagnostic(code(elaborator::pattern_contains_literals))]
    PatternContainsLiterals {
        #[label("cannot include literals here")]
        span: SourceSpan,
    },
}

pub fn surface_to_core(program: surface::Expr) -> Result<ast::Expr, ElaborationError> {
    surface_to_core_inner(&mut Vec::new(), program)
}

fn surface_to_core_inner<'a>(
    scope: &mut Vec<&'a str>,
    expr: surface::Expr<'a>,
) -> Result<ast::Expr, ElaborationError> {
    let kind = match expr.kind {
        surface::ExprKind::Literal(lit) => ast::ExprKind::Literal(surface_to_core_literal(&lit)),
        surface::ExprKind::Variable(x) => match scope.iter().rev().position(|&n| n == x) {
            Some(index) => ast::ExprKind::BoundVariable(i64::try_from(index).unwrap()),
            None => ast::ExprKind::FreeVariable(x.to_string()),
        },
        surface::ExprKind::Application(f, x) => ast::ExprKind::Application(
            Box::new(surface_to_core_inner(scope, *f)?),
            Box::new(surface_to_core_inner(scope, *x)?),
        ),
        surface::ExprKind::Abstraction(xs, body) => {
            let binders: Vec<(&str, Option<ast::Span>)> = if xs.is_empty() {
                vec![("", None)]
            } else {
                xs.iter()
                    .map(|x| (if x.name == "_" { "" } else { x.name }, Some(x.span)))
                    .collect()
            };
            for (name, _) in &binders {
                scope.push(name);
            }
            let mut inner = surface_to_core_inner(scope, *body)?;
            for (_, span) in binders[1..].iter().rev() {
                scope.pop();
                let start = span.map_or(inner.span.start, |s| s.start);
                let end = inner.span.end;
                inner = ast::Expr {
                    kind: ast::ExprKind::Abstraction(*span, Box::new(inner)),
                    span: (start..end).into(),
                }
            }
            scope.pop();
            ast::ExprKind::Abstraction(binders[0].1, Box::new(inner))
        }
        surface::ExprKind::LetIn(is_rec, x, expr, body) => {
            if is_rec {
                let (_, pattern) = push_scope(true, true, 0, scope, &x)?;
                let e = surface_to_core_inner(scope, *expr)?;
                let b = surface_to_core_inner(scope, *body)?;
                scope.pop();
                ast::ExprKind::LetIn(is_rec, pattern, Box::new(e), Box::new(b))
            } else {
                let e = surface_to_core_inner(scope, *expr)?;
                let start = scope.len();
                let (pop_count, pattern) = push_scope(false, true, start, scope, &x)?;
                let b = surface_to_core_inner(scope, *body)?;
                for _ in 0..pop_count {
                    scope.pop();
                }
                ast::ExprKind::LetIn(is_rec, pattern, Box::new(e), Box::new(b))
            }
        }
        surface::ExprKind::IfElse(condition, consequence, alternative) => ast::ExprKind::IfElse(
            Box::new(surface_to_core_inner(scope, *condition)?),
            Box::new(surface_to_core_inner(scope, *consequence)?),
            Box::new(surface_to_core_inner(scope, *alternative)?),
        ),
        surface::ExprKind::Unary(op, expr) => {
            ast::ExprKind::Unary(op, Box::new(surface_to_core_inner(scope, *expr)?))
        }
        surface::ExprKind::Binary(surface::BinaryOp::Neq, left, right) => ast::ExprKind::Unary(
            ast::UnaryOp::Not,
            Box::new(ast::Expr {
                kind: ast::ExprKind::Binary(
                    ast::BinaryOp::Eq,
                    Box::new(surface_to_core_inner(scope, *left)?),
                    Box::new(surface_to_core_inner(scope, *right)?),
                ),
                span: expr.span,
            }),
        ),
        surface::ExprKind::Binary(surface::BinaryOp::ForwardPipe, left, right) => {
            ast::ExprKind::Application(
                Box::new(surface_to_core_inner(scope, *right)?),
                Box::new(surface_to_core_inner(scope, *left)?),
            )
        }
        surface::ExprKind::Binary(surface::BinaryOp::BackwardPipe, left, right) => {
            ast::ExprKind::Application(
                Box::new(surface_to_core_inner(scope, *left)?),
                Box::new(surface_to_core_inner(scope, *right)?),
            )
        }
        surface::ExprKind::Binary(op, left, right) => ast::ExprKind::Binary(
            surface_to_core_binop(op),
            Box::new(surface_to_core_inner(scope, *left)?),
            Box::new(surface_to_core_inner(scope, *right)?),
        ),
        surface::ExprKind::Tuple(elements) => ast::ExprKind::Tuple(
            elements
                .into_iter()
                .map(|e| surface_to_core_inner(scope, e))
                .collect::<Result<_, _>>()?,
        ),
        surface::ExprKind::Sequence(left, right) => {
            let left_elabed = surface_to_core_inner(scope, *left)?;
            scope.push("");
            let right_span = right.span;
            let right_elabed = surface_to_core_inner(scope, *right)?;
            scope.pop();
            ast::ExprKind::Application(
                Box::new(ast::Expr {
                    kind: ast::ExprKind::Abstraction(None, Box::new(right_elabed)),
                    span: right_span,
                }),
                Box::new(left_elabed),
            )
        }
        surface::ExprKind::Match(scrutinee, arms) => {
            if scrutinee.is_none() {
                scope.push("");
            }
            let mut elaborated_cases = Vec::new();
            for (p, e) in arms {
                let (pop_count, pattern) = push_scope(false, false, scope.len(), scope, &p)?;
                let expr = surface_to_core_inner(scope, e)?;
                for _ in 0..pop_count {
                    scope.pop();
                }
                elaborated_cases.push((pattern, expr));
            }
            if let Some(scrutinee) = scrutinee {
                let expr = surface_to_core_inner(scope, *scrutinee)?;
                ast::ExprKind::Match(Box::new(expr), elaborated_cases)
            } else {
                scope.pop();
                ast::ExprKind::Abstraction(
                    None,
                    Box::new(ast::Expr {
                        kind: ast::ExprKind::Match(
                            Box::new(ast::Expr {
                                kind: ast::ExprKind::BoundVariable(0),
                                span: expr.span,
                            }),
                            elaborated_cases,
                        ),
                        span: expr.span,
                    }),
                )
            }
        }
    };
    Ok(ast::Expr {
        kind,
        span: expr.span,
    })
}

fn push_scope<'a>(
    is_recursive: bool,
    is_let: bool,
    start: usize,
    scope: &mut Vec<&'a str>,
    pattern: &surface::Pattern<'a>,
) -> Result<(usize, ast::Pattern), ElaborationError> {
    let (count, kind) = match &pattern.kind {
        surface::PatternKind::Literal(lit) => {
            if is_let {
                return Err(ElaborationError::PatternContainsLiterals {
                    span: pattern.span.into(),
                });
            }
            (0, ast::PatternKind::Literal(surface_to_core_literal(lit)))
        }
        surface::PatternKind::Wild => {
            scope.push("");
            (1, ast::PatternKind::Wild)
        }
        surface::PatternKind::Variable(n) => {
            if !is_recursive && scope[start..].contains(n) {
                return Err(ElaborationError::MultipleBindings {
                    x: n.to_string(),
                    span: pattern.span.into(),
                });
            }
            scope.push(n);
            (1, ast::PatternKind::Variable)
        }
        surface::PatternKind::Tuple(patterns) => {
            if is_recursive {
                return Err(ElaborationError::DestructuredLetRec {
                    span: pattern.span.into(),
                });
            }
            let mut count = 0;
            let mut pats = Vec::new();
            for p in patterns {
                let (c, pat) = push_scope(false, is_let, start, scope, p)?;
                count += c;
                pats.push(pat);
            }
            (count, ast::PatternKind::Tuple(pats))
        }
    };
    Ok((
        count,
        ast::Pattern {
            kind,
            span: pattern.span,
        },
    ))
}

fn surface_to_core_literal(lit: &surface::Literal) -> ast::Literal {
    match lit {
        surface::Literal::Unit => ast::Literal::Unit,
        surface::Literal::Char(c) => ast::Literal::Char(*c),
        surface::Literal::Bool(b) => ast::Literal::Bool(*b),
        surface::Literal::Int(n) => ast::Literal::Int(*n),
        surface::Literal::String(s) => ast::Literal::String(lexer::unescape_str(s)),
    }
}

fn surface_to_core_binop(op: surface::BinaryOp) -> ast::BinaryOp {
    match op {
        surface::BinaryOp::Add => ast::BinaryOp::Add,
        surface::BinaryOp::Sub => ast::BinaryOp::Sub,
        surface::BinaryOp::Mul => ast::BinaryOp::Mul,
        surface::BinaryOp::Div => ast::BinaryOp::Div,
        surface::BinaryOp::Lt => ast::BinaryOp::Lt,
        surface::BinaryOp::Gt => ast::BinaryOp::Gt,
        surface::BinaryOp::Lte => ast::BinaryOp::Lte,
        surface::BinaryOp::Gte => ast::BinaryOp::Gte,
        surface::BinaryOp::Eq => ast::BinaryOp::Eq,
        surface::BinaryOp::Concat => ast::BinaryOp::Concat,
        surface::BinaryOp::BitAnd => ast::BinaryOp::BitAnd,
        surface::BinaryOp::BitOr => ast::BinaryOp::BitOr,
        surface::BinaryOp::And => ast::BinaryOp::And,
        surface::BinaryOp::Or => ast::BinaryOp::Or,
        surface::BinaryOp::Neq
        | surface::BinaryOp::ForwardPipe
        | surface::BinaryOp::BackwardPipe => {
            unreachable!("!= handled separately")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn elab(source: &str) -> ast::Expr {
        surface_to_core(Parser::parse(source).unwrap()).unwrap()
    }

    #[test]
    fn elaborates_expressions() {
        assert_eq!(elab("()").to_string(), "()");
        assert_eq!(elab("true").to_string(), "true");
        assert_eq!(elab("false").to_string(), "false");
        assert_eq!(elab("'c'").to_string(), "'c'");
        assert_eq!(elab("1").to_string(), "1");
        assert_eq!(elab("\"hello\"").to_string(), "\"hello\"");
        assert_eq!(elab("fun x => x").to_string(), "(λ.#0)");
        assert_eq!(elab("fun x => fun y => x").to_string(), "(λ.(λ.#1))");
        assert_eq!(elab("fun x, y => x").to_string(), "(λ.(λ.#1))");
        assert_eq!(elab("fun x => fun x => x").to_string(), "(λ.(λ.#0))");
        assert_eq!(elab("fun x, x => x").to_string(), "(λ.(λ.#0))");
        assert_eq!(elab("(fun _ => b) a").to_string(), "((λ.b) a)");
        assert_eq!(elab("fun _ => _").to_string(), "(λ._)");
        assert_eq!(elab("fun x => fun _ => x").to_string(), "(λ.(λ.#1))");
        assert_eq!(elab("let _ = 1; _").to_string(), "(let 1 in _)");
        assert_eq!(elab("a; b").to_string(), "((λ.b) a)");
        assert_eq!(elab("a != b").to_string(), "(! (== a b))");
        assert_eq!(elab("x |> f").to_string(), "(f x)");
        assert_eq!(elab("n + 1 |> f").to_string(), "(f (+ n 1))");
        assert_eq!(elab("a || b |> f").to_string(), "(f (|| a b))");
        assert_eq!(elab("x |> y; f").to_string(), "((λ.f) (y x))");
        assert_eq!(elab("x |> { y; f }").to_string(), "(((λ.f) y) x)");
        assert_eq!(
            elab("a || b |> fun x => if x then 1 else 2").to_string(),
            "((λ.(if #0 then 1 else 2)) (|| a b))"
        );
        assert_eq!(
            elab("let (a, b, c) = (1, 2, 3); a + b + c").to_string(),
            "(let (1, 2, 3) in (+ (+ #2 #1) #0))"
        );
        assert_eq!(
            elab("let (a, _, b, _, c) = (1, 2, 3, 4, 5); a + b + c").to_string(),
            "(let (1, 2, 3, 4, 5) in (+ (+ #4 #2) #0))"
        );
    }

    #[test]
    fn detects_bad_patterns() {
        assert!(matches!(
            surface_to_core(Parser::parse("let rec (x, y) = (1, 2); x + y").unwrap()),
            Err(ElaborationError::DestructuredLetRec { .. })
        ));
        assert!(matches!(
            surface_to_core(Parser::parse("let (x, y, x) = (1, 2, 3); x").unwrap()),
            Err(ElaborationError::MultipleBindings { .. })
        ));
        assert!(matches!(
            surface_to_core(Parser::parse("let 1 = f x; ()").unwrap()),
            Err(ElaborationError::PatternContainsLiterals { .. })
        ));
    }
}
