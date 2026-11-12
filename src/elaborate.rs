use crate::ast;
use crate::surface;

pub fn surface_to_core(program: surface::Expr) -> ast::Expr {
    surface_to_core_inner(&mut Vec::new(), program)
}

fn surface_to_core_inner<'a>(scope: &mut Vec<&'a str>, expr: surface::Expr<'a>) -> ast::Expr {
    let kind = match expr.kind {
        surface::ExprKind::Char(_) | surface::ExprKind::String(_) => todo!(),
        surface::ExprKind::Unit => ast::ExprKind::Unit,
        surface::ExprKind::Bool(b) => ast::ExprKind::Bool(b),
        surface::ExprKind::Int(n) => ast::ExprKind::Int(n),
        surface::ExprKind::Variable(x) => match scope.iter().rev().position(|&n| n == x) {
            Some(index) => ast::ExprKind::BoundVariable(i64::try_from(index).unwrap()),
            None => ast::ExprKind::FreeVariable(x.to_string()),
        },
        surface::ExprKind::Application(f, x) => ast::ExprKind::Application(
            Box::new(surface_to_core_inner(scope, *f)),
            Box::new(surface_to_core_inner(scope, *x)),
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
            let mut inner = surface_to_core_inner(scope, *body);
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
            let (expr_elabed, body_elabed) = if is_rec {
                scope.push(if x.name == "_" { "" } else { x.name });
                let e = surface_to_core_inner(scope, *expr);
                let b = surface_to_core_inner(scope, *body);
                scope.pop();
                (e, b)
            } else {
                let e = surface_to_core_inner(scope, *expr);
                scope.push(if x.name == "_" { "" } else { x.name });
                let b = surface_to_core_inner(scope, *body);
                scope.pop();
                (e, b)
            };
            ast::ExprKind::LetIn(
                is_rec,
                Some(x.span),
                Box::new(expr_elabed),
                Box::new(body_elabed),
            )
        }
        surface::ExprKind::IfElse(condition, consequence, alternative) => ast::ExprKind::IfElse(
            Box::new(surface_to_core_inner(scope, *condition)),
            Box::new(surface_to_core_inner(scope, *consequence)),
            Box::new(surface_to_core_inner(scope, *alternative)),
        ),
        surface::ExprKind::Unary(op, expr) => {
            ast::ExprKind::Unary(op, Box::new(surface_to_core_inner(scope, *expr)))
        }
        surface::ExprKind::Binary(op, left, right) => ast::ExprKind::Binary(
            op,
            Box::new(surface_to_core_inner(scope, *left)),
            Box::new(surface_to_core_inner(scope, *right)),
        ),
        surface::ExprKind::Sequence(left, right) => {
            let left_elabed = surface_to_core_inner(scope, *left);
            scope.push("");
            let right_span = right.span;
            let right_elabed = surface_to_core_inner(scope, *right);
            scope.pop();
            ast::ExprKind::Application(
                Box::new(ast::Expr {
                    kind: ast::ExprKind::Abstraction(None, Box::new(right_elabed)),
                    span: right_span,
                }),
                Box::new(left_elabed),
            )
        }
    };
    ast::Expr {
        kind,
        span: expr.span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn elab(source: &str) -> ast::Expr {
        surface_to_core(Parser::parse(source).unwrap())
    }

    #[test]
    fn elaborates_expressions() {
        assert_eq!(elab("()").to_string(), "()");
        assert_eq!(elab("true").to_string(), "true");
        assert_eq!(elab("false").to_string(), "false");
        assert_eq!(elab("1").to_string(), "1");
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
    }
}
