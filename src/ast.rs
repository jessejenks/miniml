use std::{fmt::Display, ops::Range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn to(self, other: &Span) -> Span {
        Span {
            start: self.start,
            end: other.end,
        }
    }
}

impl From<Range<usize>> for Span {
    fn from(value: Range<usize>) -> Self {
        Self {
            start: value.start,
            end: value.end,
        }
    }
}

impl From<Span> for miette::SourceSpan {
    fn from(span: Span) -> Self {
        (span.start..span.end).into()
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum UnaryOp {
    Negate,
    BitNot,
    Not,
}

impl Display for UnaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnaryOp::Negate => write!(f, "-"),
            UnaryOp::BitNot => write!(f, "~"),
            UnaryOp::Not => write!(f, "!"),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Lt,
    Gt,
    Lte,
    Gte,
    Eq,
    BitAnd,
    BitOr,
    And,
    Or,
}

impl Display for BinaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BinaryOp::Add => write!(f, "+"),
            BinaryOp::Sub => write!(f, "-"),
            BinaryOp::Mul => write!(f, "*"),
            BinaryOp::Div => write!(f, "/"),
            BinaryOp::Lt => write!(f, "<"),
            BinaryOp::Gt => write!(f, ">"),
            BinaryOp::Lte => write!(f, "<="),
            BinaryOp::Gte => write!(f, ">="),
            BinaryOp::Eq => write!(f, "=="),
            BinaryOp::BitAnd => write!(f, "&"),
            BinaryOp::BitOr => write!(f, "|"),
            BinaryOp::And => write!(f, "&&"),
            BinaryOp::Or => write!(f, "||"),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum ExprKind {
    Unit,
    Bool(bool),
    Int(i64),
    BoundVariable(i64),
    FreeVariable(String),
    Application(Box<Expr>, Box<Expr>),
    Abstraction(Option<Span>, Box<Expr>),
    LetIn(bool, Option<Span>, Box<Expr>, Box<Expr>),
    IfElse(Box<Expr>, Box<Expr>, Box<Expr>),
    Unary(UnaryOp, Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
}

impl Display for ExprKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExprKind::Unit => write!(f, "()"),
            ExprKind::Bool(b) => write!(f, "{b}"),
            ExprKind::Int(n) => write!(f, "{n}"),
            ExprKind::FreeVariable(x) => write!(f, "{x}"),
            ExprKind::BoundVariable(i) => write!(f, "#{i}"),
            ExprKind::Application(left, right) => write!(f, "({left} {right})"),
            ExprKind::Abstraction(_, body) => write!(f, "(λ.{body})"),
            ExprKind::LetIn(is_rec, _, expr, body) => write!(
                f,
                "(let {}{expr} in {body})",
                if *is_rec { "rec " } else { "" },
            ),
            ExprKind::IfElse(condition, consequence, alternative) => {
                write!(f, "(if {condition} then {consequence} else {alternative})")
            }
            ExprKind::Unary(op, expr) => write!(f, "({op} {expr})"),
            ExprKind::Binary(op, left, right) => write!(f, "({op} {left} {right})"),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.kind.fmt(f)
    }
}
