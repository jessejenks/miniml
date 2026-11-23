use std::collections::{HashMap, HashSet};

use miette::Diagnostic;
use miette::SourceSpan;
use thiserror::Error;

use crate::ast;
use crate::lexer;
use crate::surface;
use crate::typecheck::{TypeExpr, TypeId};

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
    #[error("{name} is declared multiple times")]
    #[diagnostic(code(elaborator::duplicate_declaration))]
    DuplicateDeclaration {
        name: String,
        #[label("this name has already been declared")]
        span: SourceSpan,
    },
    #[error("unknown type {name}")]
    #[diagnostic(code(elaborator::unknown_type))]
    UnknownType {
        name: String,
        #[label("this type is not defined")]
        span: SourceSpan,
    },
    #[error("unknown constructor {name}")]
    #[diagnostic(code(elaborator::unknown_constructor))]
    UnknownConstructor {
        name: String,
        #[label("this constructor is not defined")]
        span: SourceSpan,
    },
    #[error("too many constructors {name}")]
    #[diagnostic(code(elaborator::too_many_constructors))]
    TooManyConstructors {
        name: String,
        #[label("max 247 constructors allowed per type")]
        span: SourceSpan,
    },
    #[error("{name} expects {expected} type arguments, but got {actual}")]
    #[diagnostic(code(elaborator::wrong_type_arity))]
    WrongTypeArity {
        name: String,
        expected: usize,
        actual: usize,
        #[label("wrong number of type arguments")]
        span: SourceSpan,
    },
}

pub fn elaborate(
    program: surface::Expr,
    decls: &[surface::TypeDecl],
) -> Result<(ast::Expr, Vec<ast::TypeDecl>), ElaborationError> {
    let (elaborated_decls, constructors) = elaborate_type_decls(decls)?;
    Ok((
        surface_to_core_inner(&mut Vec::new(), program, &constructors)?,
        elaborated_decls,
    ))
}

const BUILTIN_TYPES: [(&str, TypeExpr); 5] = [
    ("Unit", TypeExpr::Unit),
    ("Bool", TypeExpr::Bool),
    ("Char", TypeExpr::Char),
    ("Int", TypeExpr::Int),
    ("String", TypeExpr::String),
];

enum TypeName {
    Builtin(TypeExpr),
    Declared(TypeId, usize),
}

type Constructors<'a> = HashMap<&'a str, (TypeId, HashMap<&'a str, (usize, usize)>)>;

pub fn elaborate_type_decls<'a>(
    decls: &'a [surface::TypeDecl],
) -> Result<(Vec<ast::TypeDecl>, Constructors<'a>), ElaborationError> {
    let mut types: HashMap<&str, TypeName> = BUILTIN_TYPES
        .into_iter()
        .map(|(name, tp)| (name, TypeName::Builtin(tp)))
        .collect();
    let mut constructors: Constructors = HashMap::new();

    for (i, decl) in decls.iter().enumerate() {
        check_unique(&decl.type_parameters)?;
        let id = TypeId(i);
        let entry = TypeName::Declared(id, decl.type_parameters.len());
        if types.insert(decl.name.name, entry).is_some() {
            return Err(duplicate_decl_error(&decl.name));
        }
        if decl.constructors.len() > 247 {
            return Err(ElaborationError::TooManyConstructors {
                name: decl.name.name.to_string(),
                span: decl.name.span.into(),
            });
        }
        let mut ctors = HashMap::new();
        for (i, (c, fields)) in decl.constructors.iter().enumerate() {
            if ctors.insert(c.name, (i, fields.len())).is_some() {
                return Err(duplicate_decl_error(c));
            }
        }
        constructors.insert(decl.name.name, (id, ctors));
    }

    let elaborated_decls = decls
        .iter()
        .map(|decl| {
            let constructors = decl
                .constructors
                .iter()
                .map(|(c, fields)| {
                    let fields = fields
                        .iter()
                        .map(|t| elaborate_type(&types, &decl.type_parameters, t))
                        .collect::<Result<_, _>>()?;
                    Ok((c.name.to_string(), fields))
                })
                .collect::<Result<_, _>>()?;
            Ok(ast::TypeDecl {
                name: decl.name.name.to_string(),
                arity: decl.type_parameters.len(),
                constructors,
            })
        })
        .collect::<Result<_, _>>()?;

    Ok((elaborated_decls, constructors))
}

fn check_unique<'a>(
    idents: impl IntoIterator<Item = &'a surface::Identifier<'a>>,
) -> Result<(), ElaborationError> {
    let mut seen = HashSet::new();
    for ident in idents {
        if !seen.insert(ident.name) {
            return Err(duplicate_decl_error(ident));
        }
    }
    Ok(())
}

fn duplicate_decl_error(ident: &surface::Identifier) -> ElaborationError {
    ElaborationError::DuplicateDeclaration {
        name: ident.name.to_string(),
        span: ident.span.into(),
    }
}

fn unknown_type_error(path: &surface::Path) -> ElaborationError {
    ElaborationError::UnknownType {
        name: path.to_string(),
        span: path.span.into(),
    }
}

fn unknown_constructor_error(path: &surface::Path) -> ElaborationError {
    ElaborationError::UnknownConstructor {
        name: path.to_string(),
        span: path.span.into(),
    }
}

fn elaborate_type(
    types: &HashMap<&str, TypeName>,
    params: &[surface::Identifier],
    tp: &surface::TypeExpr,
) -> Result<TypeExpr, ElaborationError> {
    match &tp.kind {
        surface::TypeExprKind::Arrow(src, tgt) => Ok(TypeExpr::arrow(
            elaborate_type(types, params, src)?,
            elaborate_type(types, params, tgt)?,
        )),
        surface::TypeExprKind::Product(elts) => Ok(TypeExpr::Product(
            elts.iter()
                .map(|t| elaborate_type(types, params, t))
                .collect::<Result<_, _>>()?,
        )),
        surface::TypeExprKind::Named(path, args) => {
            let name = path.name.name;
            let unknown = || ElaborationError::UnknownType {
                name: path.to_string(),
                span: tp.span.into(),
            };
            if !path.qualifiers.is_empty() {
                return Err(unknown());
            }
            let param = params.iter().position(|p| p.name == name);
            let (resolved, arity) = match (param, types.get(name)) {
                (Some(i), _) => (TypeExpr::Var(i64::try_from(i).unwrap(), false), 0),
                (None, Some(TypeName::Builtin(builtin))) => (builtin.clone(), 0),
                (None, Some(&TypeName::Declared(id, arity))) => {
                    let args = args
                        .iter()
                        .map(|t| elaborate_type(types, params, t))
                        .collect::<Result<_, _>>()?;
                    (TypeExpr::Named(id, args), arity)
                }
                (None, None) => return Err(unknown()),
            };
            if args.len() != arity {
                return Err(ElaborationError::WrongTypeArity {
                    name: path.to_string(),
                    expected: arity,
                    actual: args.len(),
                    span: tp.span.into(),
                });
            }
            Ok(resolved)
        }
    }
}

fn surface_to_core_inner<'a>(
    scope: &mut Vec<&'a str>,
    expr: surface::Expr<'a>,
    constructors: &Constructors<'a>,
) -> Result<ast::Expr, ElaborationError> {
    let kind = match expr.kind {
        surface::ExprKind::Literal(lit) => ast::ExprKind::Literal(surface_to_core_literal(&lit)),
        surface::ExprKind::Variable(x) => match scope.iter().rev().position(|&n| n == x) {
            Some(index) => ast::ExprKind::BoundVariable(i64::try_from(index).unwrap()),
            None => ast::ExprKind::FreeVariable(x.to_string()),
        },
        surface::ExprKind::Path(p) => {
            let [type_name] = p.qualifiers.as_slice() else {
                return Err(unknown_type_error(&p));
            };
            let Some((id, ctors)) = constructors.get(type_name.name) else {
                return Err(unknown_type_error(&p));
            };
            let Some(&(index, field_count)) = ctors.get(p.name.name) else {
                return Err(unknown_constructor_error(&p));
            };
            // eta-expand
            let args = (0..field_count)
                .rev()
                .map(|i| ast::Expr {
                    kind: ast::ExprKind::BoundVariable(i64::try_from(i).unwrap()),
                    span: expr.span,
                })
                .collect();
            let mut kind = ast::ExprKind::Constructor(*id, index, args);
            for _ in 0..field_count {
                kind = ast::ExprKind::Abstraction(
                    None,
                    Box::new(ast::Expr {
                        kind,
                        span: expr.span,
                    }),
                );
            }
            kind
        }
        surface::ExprKind::Application(f, x) => ast::ExprKind::Application(
            Box::new(surface_to_core_inner(scope, *f, constructors)?),
            Box::new(surface_to_core_inner(scope, *x, constructors)?),
        ),
        surface::ExprKind::Abstraction(xs, body) => {
            let binders: Vec<(&str, Option<ast::Span>)> = if xs.is_empty() {
                vec![("", None)]
            } else {
                xs.iter().map(|x| (x.name, Some(x.span))).collect()
            };
            for (name, _) in &binders {
                scope.push(name);
            }
            let mut inner = surface_to_core_inner(scope, *body, constructors)?;
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
                let e = surface_to_core_inner(scope, *expr, constructors)?;
                let b = surface_to_core_inner(scope, *body, constructors)?;
                scope.pop();
                ast::ExprKind::LetIn(is_rec, pattern, Box::new(e), Box::new(b))
            } else {
                let e = surface_to_core_inner(scope, *expr, constructors)?;
                let start = scope.len();
                let (pop_count, pattern) = push_scope(false, true, start, scope, &x)?;
                let b = surface_to_core_inner(scope, *body, constructors)?;
                for _ in 0..pop_count {
                    scope.pop();
                }
                ast::ExprKind::LetIn(is_rec, pattern, Box::new(e), Box::new(b))
            }
        }
        surface::ExprKind::IfElse(condition, consequence, alternative) => ast::ExprKind::IfElse(
            Box::new(surface_to_core_inner(scope, *condition, constructors)?),
            Box::new(surface_to_core_inner(scope, *consequence, constructors)?),
            Box::new(surface_to_core_inner(scope, *alternative, constructors)?),
        ),
        surface::ExprKind::Unary(op, expr) => ast::ExprKind::Unary(
            op,
            Box::new(surface_to_core_inner(scope, *expr, constructors)?),
        ),
        surface::ExprKind::Binary(surface::BinaryOp::Neq, left, right) => ast::ExprKind::Unary(
            ast::UnaryOp::Not,
            Box::new(ast::Expr {
                kind: ast::ExprKind::Binary(
                    ast::BinaryOp::Eq,
                    Box::new(surface_to_core_inner(scope, *left, constructors)?),
                    Box::new(surface_to_core_inner(scope, *right, constructors)?),
                ),
                span: expr.span,
            }),
        ),
        surface::ExprKind::Binary(surface::BinaryOp::ForwardPipe, left, right) => {
            ast::ExprKind::Application(
                Box::new(surface_to_core_inner(scope, *right, constructors)?),
                Box::new(surface_to_core_inner(scope, *left, constructors)?),
            )
        }
        surface::ExprKind::Binary(surface::BinaryOp::BackwardPipe, left, right) => {
            ast::ExprKind::Application(
                Box::new(surface_to_core_inner(scope, *left, constructors)?),
                Box::new(surface_to_core_inner(scope, *right, constructors)?),
            )
        }
        surface::ExprKind::Binary(op, left, right) => ast::ExprKind::Binary(
            surface_to_core_binop(op),
            Box::new(surface_to_core_inner(scope, *left, constructors)?),
            Box::new(surface_to_core_inner(scope, *right, constructors)?),
        ),
        surface::ExprKind::Tuple(elements) => ast::ExprKind::Tuple(
            elements
                .into_iter()
                .map(|e| surface_to_core_inner(scope, e, constructors))
                .collect::<Result<_, _>>()?,
        ),
        surface::ExprKind::Sequence(left, right) => {
            let left_elabed = surface_to_core_inner(scope, *left, constructors)?;
            scope.push("");
            let right_span = right.span;
            let right_elabed = surface_to_core_inner(scope, *right, constructors)?;
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
                let expr = surface_to_core_inner(scope, e, constructors)?;
                for _ in 0..pop_count {
                    scope.pop();
                }
                elaborated_cases.push((pattern, expr));
            }
            if let Some(scrutinee) = scrutinee {
                let expr = surface_to_core_inner(scope, *scrutinee, constructors)?;
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

    fn elab_decls(source: &str) -> Result<Vec<ast::TypeDecl>, ElaborationError> {
        Ok(elaborate_type_decls(&Parser::parse(source).unwrap().1)?.0)
    }

    #[test]
    fn elaborates_type_declarations() {
        let decls = elab_decls(
            "type List a = Nil | Cons a (List a);
             type Pair a b = Pair (a, b) (List a -> b) Int;",
        )
        .unwrap();
        let list = |arg| TypeExpr::Named(TypeId(0), vec![arg].into());
        let var = |i| TypeExpr::Var(i, false);
        assert_eq!(
            decls[0],
            ast::TypeDecl {
                name: "List".into(),
                arity: 1,
                constructors: vec![
                    ("Nil".into(), vec![]),
                    ("Cons".into(), vec![var(0), list(var(0))]),
                ],
            }
        );
        assert_eq!(
            decls[1].constructors[0].1,
            vec![
                TypeExpr::Product(vec![var(0), var(1)].into()),
                TypeExpr::arrow(list(var(0)), var(1)),
                TypeExpr::Int,
            ]
        );

        // later declarations are visible to earlier ones
        let decls = elab_decls("type A = A B; type B = B;").unwrap();
        assert_eq!(
            decls[0].constructors[0].1,
            vec![TypeExpr::Named(TypeId(1), vec![].into())]
        );
    }

    #[test]
    fn detects_bad_type_declarations() {
        for source in [
            "type T = A; type T = B;",
            "type Int = I;",
            "type T a a = T;",
            "type T = A | A;",
        ] {
            assert!(
                matches!(
                    elab_decls(source),
                    Err(ElaborationError::DuplicateDeclaration { .. })
                ),
                "{source}"
            );
        }
        for source in ["type T = T a;", "type T = T Foo;", "type T = T Other::T;"] {
            assert!(
                matches!(
                    elab_decls(source),
                    Err(ElaborationError::UnknownType { .. })
                ),
                "{source}"
            );
        }
        for source in [
            "type T a = T T;",
            "type T = T (Int Int);",
            "type T a = T (a Int);",
        ] {
            assert!(
                matches!(
                    elab_decls(source),
                    Err(ElaborationError::WrongTypeArity { .. })
                ),
                "{source}"
            );
        }
    }

    fn elab(source: &str) -> ast::Expr {
        elaborate(Parser::parse(source).unwrap().0, &[]).unwrap().0
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
        assert_eq!(elab("fun _ => 1").to_string(), "(λ.1)");
        assert_eq!(elab("fun x => fun _ => x").to_string(), "(λ.(λ.#1))");
        assert_eq!(elab("let _ = 1; 2").to_string(), "(let 1 in 2)");
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

    fn elab_with_types(source: &str) -> Result<ast::Expr, ElaborationError> {
        let types = "type Option a = None | Some a; type Pair = Pair Int Int;";
        let source = format!("{types} {source}");
        let (program, decls) = Parser::parse(&source).unwrap();
        Ok(elaborate(program, &decls)?.0)
    }

    #[test]
    fn elaborates_constructors() {
        let elab = |source| elab_with_types(source).unwrap().to_string();
        assert_eq!(elab("Option::None"), "T_0::[0]");
        assert_eq!(elab("Option::Some"), "(λ.T_0::[1](#0))");
        assert_eq!(elab("Option::Some 1"), "((λ.T_0::[1](#0)) 1)");
        assert_eq!(elab("Pair::Pair"), "(λ.(λ.T_1::[0](#1, #0)))");
        assert_eq!(elab("Pair::Pair 1 2"), "(((λ.(λ.T_1::[0](#1, #0))) 1) 2)");
        assert_eq!(elab("fun x => Option::Some x"), "(λ.((λ.T_0::[1](#0)) #0))");

        for source in ["Foo::Bar", "Int::Zero", "Option::Option::None"] {
            assert!(
                matches!(
                    elab_with_types(source),
                    Err(ElaborationError::UnknownType { .. })
                ),
                "{source}"
            );
        }
        assert!(matches!(
            elab_with_types("Option::Nothing"),
            Err(ElaborationError::UnknownConstructor { .. })
        ));
    }

    #[test]
    fn detects_bad_patterns() {
        assert!(matches!(
            elaborate(
                Parser::parse("let rec (x, y) = (1, 2); x + y").unwrap().0,
                &[]
            ),
            Err(ElaborationError::DestructuredLetRec { .. })
        ));
        assert!(matches!(
            elaborate(
                Parser::parse("let (x, y, x) = (1, 2, 3); x").unwrap().0,
                &[]
            ),
            Err(ElaborationError::MultipleBindings { .. })
        ));
        assert!(matches!(
            elaborate(Parser::parse("let 1 = f x; ()").unwrap().0, &[]),
            Err(ElaborationError::PatternContainsLiterals { .. })
        ));
    }
}
