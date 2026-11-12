use crate::ast::{BinaryOp, Span, UnaryOp};
use std::fmt::Display;

pub struct Identifier<'a> {
    pub name: &'a str,
    pub span: Span,
}

pub enum ExprKind<'a> {
    Unit,
    Bool(bool),
    Char(char),
    Int(i64),
    String(&'a str),
    Variable(&'a str),
    Application(Box<Expr<'a>>, Box<Expr<'a>>),
    Abstraction(Vec<Identifier<'a>>, Box<Expr<'a>>),
    LetIn(bool, Identifier<'a>, Box<Expr<'a>>, Box<Expr<'a>>),
    IfElse(Box<Expr<'a>>, Box<Expr<'a>>, Box<Expr<'a>>),
    Unary(UnaryOp, Box<Expr<'a>>),
    Binary(BinaryOp, Box<Expr<'a>>, Box<Expr<'a>>),
    Sequence(Box<Expr<'a>>, Box<Expr<'a>>),
}

impl Display for ExprKind<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExprKind::Unit => write!(f, "()"),
            ExprKind::Bool(b) => write!(f, "{b}"),
            ExprKind::Char(c) => write!(f, "'{c}'"),
            ExprKind::Int(n) => write!(f, "{n}"),
            ExprKind::String(s) => write!(f, "\"{s}\""),
            ExprKind::Variable(x) => write!(f, "{x}"),
            ExprKind::Application(left, right) => write!(f, "({left} {right})"),
            ExprKind::Abstraction(xs, body) => {
                if xs.is_empty() {
                    write!(f, "(fun => {body})")
                } else {
                    write!(f, "(fun ")?;
                    for (i, x) in xs.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", x.name)?;
                    }
                    write!(f, " => {body})")
                }
            }
            ExprKind::LetIn(is_rec, x, expr, body) => write!(
                f,
                "(let {}{} = {expr}; {body})",
                if *is_rec { "rec " } else { "" },
                x.name,
            ),
            ExprKind::IfElse(condition, consequence, alternative) => {
                write!(f, "(if {condition} then {consequence} else {alternative})")
            }
            ExprKind::Unary(op, expr) => write!(f, "({op} {expr})"),
            ExprKind::Binary(op, left, right) => write!(f, "({left} {op} {right})"),
            ExprKind::Sequence(left, right) => write!(f, "({left}; {right})"),
        }
    }
}

pub struct Expr<'a> {
    pub kind: ExprKind<'a>,
    pub span: Span,
}

impl<'a> Expr<'a> {
    pub fn new(kind: ExprKind<'a>, start: usize, end: usize) -> Self {
        Self {
            kind,
            span: (start..end).into(),
        }
    }
}

impl Display for Expr<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.kind.fmt(f)
    }
}
