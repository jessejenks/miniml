use crate::ast::{Span, UnaryOp};
use crate::lexer::escape_str;
use std::fmt::Display;

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
    Neq,
    Concat,
    BitAnd,
    BitOr,
    And,
    Or,
    ForwardPipe,
    BackwardPipe,
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
            BinaryOp::Neq => write!(f, "!="),
            BinaryOp::Concat => write!(f, "++"),
            BinaryOp::BitAnd => write!(f, "&"),
            BinaryOp::BitOr => write!(f, "|"),
            BinaryOp::And => write!(f, "&&"),
            BinaryOp::Or => write!(f, "||"),
            BinaryOp::ForwardPipe => write!(f, "|>"),
            BinaryOp::BackwardPipe => write!(f, "<|"),
        }
    }
}

pub enum Literal<'a> {
    Unit,
    Bool(bool),
    Char(char),
    Int(i64),
    String(&'a str),
}

impl Display for Literal<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unit => write!(f, "()"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Char(c) => write!(f, "'{}'", escape_str(&c.to_string(), '\'')),
            Self::Int(n) => write!(f, "{n}"),
            Self::String(s) => write!(f, "\"{s}\""),
        }
    }
}

pub enum PatternKind<'a> {
    Literal(Literal<'a>),
    Wild,
    Variable(&'a str),
    Tuple(Vec<Pattern<'a>>),
}

impl Display for PatternKind<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Literal(lit) => lit.fmt(f),
            Self::Wild => write!(f, "_"),
            Self::Variable(x) => write!(f, "{x}"),
            Self::Tuple(patterns) => {
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

pub struct Pattern<'a> {
    pub kind: PatternKind<'a>,
    pub span: Span,
}

impl Display for Pattern<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.kind.fmt(f)
    }
}

pub struct Identifier<'a> {
    pub name: &'a str,
    pub span: Span,
}

pub enum ExprKind<'a> {
    Literal(Literal<'a>),
    Variable(&'a str),
    Application(Box<Expr<'a>>, Box<Expr<'a>>),
    Abstraction(Vec<Identifier<'a>>, Box<Expr<'a>>),
    LetIn(bool, Pattern<'a>, Box<Expr<'a>>, Box<Expr<'a>>),
    IfElse(Box<Expr<'a>>, Box<Expr<'a>>, Box<Expr<'a>>),
    Unary(UnaryOp, Box<Expr<'a>>),
    Binary(BinaryOp, Box<Expr<'a>>, Box<Expr<'a>>),
    Sequence(Box<Expr<'a>>, Box<Expr<'a>>),
    Tuple(Vec<Expr<'a>>),
    Match(Option<Box<Expr<'a>>>, Vec<(Pattern<'a>, Expr<'a>)>),
}

impl Display for ExprKind<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExprKind::Literal(lit) => lit.fmt(f),
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
                x,
            ),
            ExprKind::IfElse(condition, consequence, alternative) => {
                write!(f, "(if {condition} then {consequence} else {alternative})")
            }
            ExprKind::Unary(op, expr) => write!(f, "({op} {expr})"),
            ExprKind::Binary(op, left, right) => write!(f, "({left} {op} {right})"),
            ExprKind::Sequence(left, right) => write!(f, "({left}; {right})"),
            ExprKind::Tuple(elements) => {
                write!(f, "(")?;
                for (i, elt) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{elt}")?;
                }
                write!(f, ")")
            }
            ExprKind::Match(scrutinee, cases) => {
                write!(f, "(match")?;
                if let Some(expr) = scrutinee {
                    write!(f, " {expr}")?;
                }
                write!(f, " {{ ")?;
                for (i, (pattern, expr)) in cases.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{pattern} => {expr}")?;
                }
                write!(f, " }})")
            }
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
