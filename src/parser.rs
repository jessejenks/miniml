use miette::{Diagnostic, SourceSpan};
use std::iter::Peekable;
use thiserror::Error;

use crate::ast::UnaryOp;
use crate::lexer::{Keyword, LexError, Lexer, Spanned, Token};
use crate::surface;
use crate::surface::BinaryOp;

#[derive(Debug, Error, Diagnostic)]
pub enum ParseError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Lex(#[from] LexError),

    #[error("unexpected token {tok}")]
    #[diagnostic(code(parser::unexpected_token))]
    UnexpectedToken {
        tok: String,
        #[label("this token was not expected")]
        span: SourceSpan,
    },

    #[error("unexpected token {actual}, expected {expected}")]
    #[diagnostic(code(parser::unexpected_token_expecting))]
    UnexpectedTokenExpecting {
        actual: String,
        expected: String,
        #[label("this token was not expected")]
        span: SourceSpan,
    },

    #[error("pad path")]
    #[diagnostic(code(parser::bad_path))]
    BadPath {
        #[label("qualified names cannot contain spaces")]
        span: SourceSpan,
    },

    #[error("unexpected end of file")]
    #[diagnostic(code(parser::unexpected_eof))]
    #[label("input ended here")]
    UnexpectedEOF { span: SourceSpan },

    #[error("expected end of file, but got {tok}")]
    #[diagnostic(code(parser::expected_eof))]
    ExpectedEOF {
        tok: String,
        #[label("expected end of file here")]
        span: SourceSpan,
    },

    #[error("cannot chain non-associative operator {first_op}")]
    #[diagnostic(code(parser::chain_non_associative_operator))]
    ChainNonAssociativeOp {
        first_op: String,
        #[label("this operator is non-associative")]
        first_span: SourceSpan,
        second_op: String,
        #[label("followed by this")]
        second_span: SourceSpan,
    },

    #[error("cannot mix {first_op} and {second_op} without parentheses")]
    #[diagnostic(code(parser::mixed_pipe_operators))]
    MixedPipeOperators {
        first_op: String,
        #[label("this pipe")]
        first_span: SourceSpan,
        second_op: String,
        #[label("and this pipe")]
        second_span: SourceSpan,
    },
}

const PREFIX_RBP: u8 = 13;
const APPLICATION_LBP: u8 = 15;
const APPLICATION_RBP: u8 = 16;

const TYPE_ARROW_LBP: u8 = 2;
const TYPE_ARROW_RBP: u8 = 1;
const TYPE_APPLICATION_LBP: u8 = 3;

fn token_to_binary_op(tok: &Token) -> Option<BinaryOp> {
    match tok {
        Token::Plus => Some(BinaryOp::Add),
        Token::PlusPlus => Some(BinaryOp::Concat),
        Token::Dash => Some(BinaryOp::Sub),
        Token::Star => Some(BinaryOp::Mul),
        Token::Slash => Some(BinaryOp::Div),
        Token::Lt => Some(BinaryOp::Lt),
        Token::Gt => Some(BinaryOp::Gt),
        Token::Lte => Some(BinaryOp::Lte),
        Token::Gte => Some(BinaryOp::Gte),
        Token::BangEqual => Some(BinaryOp::Neq),
        Token::EqualEqual => Some(BinaryOp::Eq),
        Token::Amp => Some(BinaryOp::BitAnd),
        Token::Pipe => Some(BinaryOp::BitOr),
        Token::AmpAmp => Some(BinaryOp::And),
        Token::PipePipe => Some(BinaryOp::Or),
        Token::PipeGT => Some(BinaryOp::ForwardPipe),
        Token::LtPipe => Some(BinaryOp::BackwardPipe),
        _ => None,
    }
}

pub struct Parser<'a> {
    source: &'a str,
    lexer: Peekable<Lexer<'a>>,
    prev_token: Option<Spanned<'a>>,
    decls: Vec<surface::TypeDecl<'a>>,
}

impl<'a> Parser<'a> {
    pub fn parse(
        source: &'a str,
    ) -> Result<(surface::Expr<'a>, Vec<surface::TypeDecl<'a>>), ParseError> {
        let mut p = Parser {
            source,
            lexer: Lexer::new(source).peekable(),
            prev_token: None,
            decls: Vec::new(),
        };
        let program = p.block_body(0, true)?;
        if let Some((start, tok, end)) = p.peek()? {
            return Err(ParseError::ExpectedEOF {
                tok: format!("{tok:?}"),
                span: (start..end).into(),
            });
        }
        Ok((program, p.decls))
    }

    fn next(&mut self) -> Result<Option<Spanned<'a>>, LexError> {
        let tok = self.lexer.next().transpose()?;
        self.prev_token = tok;
        Ok(tok)
    }

    fn peek(&mut self) -> Result<Option<Spanned<'a>>, LexError> {
        match self.lexer.peek() {
            Some(&Ok(tok)) => Ok(Some(tok)),
            Some(Err(_)) => self.next(),
            None => Ok(None),
        }
    }

    fn expect(&mut self, expected: Token) -> Result<Spanned<'a>, ParseError> {
        match self.next()? {
            Some(span) if span.1 == expected => Ok(span),
            Some((start, actual, end)) => Err(ParseError::UnexpectedTokenExpecting {
                actual: format!("{actual:?}"),
                expected: format!("{expected:?}"),
                span: (start..end).into(),
            }),
            None => Err(self.eof_error()),
        }
    }

    fn expect_identifier(&mut self) -> Result<surface::Identifier<'a>, ParseError> {
        match self.next()? {
            Some((start, Token::Identifier(name), end)) => Ok(surface::Identifier {
                name,
                span: (start..end).into(),
            }),
            Some((start, actual, end)) => Err(ParseError::UnexpectedTokenExpecting {
                actual: format!("{actual:?}"),
                expected: "identifier".into(),
                span: (start..end).into(),
            }),
            None => Err(self.eof_error()),
        }
    }

    fn chomp_if_next_is(&mut self, expected: Token) -> Result<bool, ParseError> {
        match self.peek()? {
            Some((_, actual, _)) if actual == expected => {
                self.next()?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn block_body(
        &mut self,
        start: usize,
        is_top_level: bool,
    ) -> Result<surface::Expr<'a>, ParseError> {
        if matches!(self.peek()?, None | Some((_, Token::RBrace, _))) {
            return Ok(surface::Expr::new(
                surface::ExprKind::Literal(surface::Literal::Unit),
                start,
                start,
            ));
        }
        match self.peek()? {
            Some((let_start, Token::Keyword(Keyword::Let), _)) => {
                self.next()?;
                let is_recursive = self.chomp_if_next_is(Token::Keyword(Keyword::Rec))?;
                let pattern = self.pattern()?;
                self.expect(Token::Equal)?;
                let expr = self.expression(0)?;
                let body_start = if let Some((_, Token::RBrace, brace_end)) = self.prev_token {
                    self.chomp_if_next_is(Token::Semi)?;
                    brace_end
                } else {
                    let (_, _, semi_end) = self.expect(Token::Semi)?;
                    semi_end
                };
                let body = self.block_body(body_start, is_top_level)?;
                let end = body.span.end;
                return Ok(surface::Expr::new(
                    surface::ExprKind::LetIn(is_recursive, pattern, Box::new(expr), Box::new(body)),
                    let_start,
                    end,
                ));
            }
            Some((_, Token::Keyword(Keyword::Type), _)) if is_top_level => {
                self.next()?;
                let name = self.expect_identifier()?;
                let mut type_parameters = Vec::new();
                while let Some((param_start, Token::Identifier(t), param_end)) = self.peek()? {
                    self.next()?;
                    type_parameters.push(surface::Identifier {
                        name: t,
                        span: (param_start..param_end).into(),
                    });
                }
                self.expect(Token::Equal)?;
                let mut constructors = vec![self.constructor()?];
                while self.chomp_if_next_is(Token::Pipe)? {
                    constructors.push(self.constructor()?);
                }
                let (_, _, end) = self.expect(Token::Semi)?;
                self.decls.push(surface::TypeDecl {
                    name,
                    type_parameters,
                    constructors,
                });
                return self.block_body(end, is_top_level);
            }
            _ => {}
        }
        let expr = self.expression(0)?;
        let ended_with_block = self.just_closed_block();
        let at_end = matches!(self.peek()?, None | Some((_, Token::RBrace, _)));
        if self.chomp_if_next_is(Token::Semi)? || (ended_with_block && !at_end) {
            let (_, _, rest_start) = self.prev_token.expect("just read `;` or `}`");
            let rest = self.block_body(rest_start, is_top_level)?;
            let seq_start = expr.span.start;
            let end = rest.span.end;
            Ok(surface::Expr::new(
                surface::ExprKind::Sequence(Box::new(expr), Box::new(rest)),
                seq_start,
                end,
            ))
        } else {
            Ok(expr)
        }
    }

    fn constructor(
        &mut self,
    ) -> Result<(surface::Identifier<'a>, Vec<surface::TypeExpr<'a>>), ParseError> {
        let constructor = self.expect_identifier()?;
        let mut type_args = Vec::new();
        while let Some((_, tok, _)) = self.peek()?
            && Self::starts_type_atom(&tok)
        {
            type_args.push(self.type_atom()?);
        }
        Ok((constructor, type_args))
    }

    fn type_expression(&mut self, min_bp: u8) -> Result<surface::TypeExpr<'a>, ParseError> {
        let mut left = self.type_atom()?;
        while let Some((_, tok, _)) = self.peek()? {
            if Self::starts_type_atom(&tok) {
                if TYPE_APPLICATION_LBP <= min_bp {
                    break;
                }
                let surface::TypeExprKind::Named(_, args) = &mut left.kind else {
                    break;
                };
                let arg = self.type_atom()?;
                left.span = (left.span.start..arg.span.end).into();
                args.push(arg);
                continue;
            }
            if tok == Token::NarrowArrow {
                if TYPE_ARROW_LBP <= min_bp {
                    break;
                }
                self.next()?;
                let right = self.type_expression(TYPE_ARROW_RBP)?;
                let span = (left.span.start..right.span.end).into();
                left = surface::TypeExpr {
                    kind: surface::TypeExprKind::Arrow(Box::new(left), Box::new(right)),
                    span,
                };
                continue;
            }
            break;
        }
        Ok(left)
    }

    fn starts_type_atom(tok: &Token<'a>) -> bool {
        matches!(tok, Token::LParen | Token::Identifier(_))
    }

    fn type_atom(&mut self) -> Result<surface::TypeExpr<'a>, ParseError> {
        match self.peek()? {
            Some((start, Token::LParen, _)) => {
                self.next()?;
                let t = self.type_expression(0)?;
                if self.chomp_if_next_is(Token::Comma)? {
                    let mut elts = vec![t, self.type_expression(0)?];
                    while self.chomp_if_next_is(Token::Comma)? {
                        elts.push(self.type_expression(0)?);
                    }
                    let (_, _, paren_end) = self.expect(Token::RParen)?;
                    return Ok(surface::TypeExpr {
                        kind: surface::TypeExprKind::Product(elts),
                        span: (start..paren_end).into(),
                    });
                }
                let (_, _, paren_end) = self.expect(Token::RParen)?;
                Ok(surface::TypeExpr {
                    kind: t.kind,
                    span: (start..paren_end).into(),
                })
            }
            Some((start, Token::Identifier(name), end)) => {
                self.next()?;
                let (path, new_end) = self.qualified_name(name, start, end)?;
                Ok(surface::TypeExpr {
                    kind: surface::TypeExprKind::Named(path, Vec::new()),
                    span: (start..new_end).into(),
                })
            }
            Some((start, tok, end)) => Err(ParseError::UnexpectedTokenExpecting {
                actual: format!("{tok:?}"),
                expected: "type expression".to_string(),
                span: (start..end).into(),
            }),
            None => Err(self.eof_error()),
        }
    }

    fn expression(&mut self, min_bp: u8) -> Result<surface::Expr<'a>, ParseError> {
        let (start, tok, end) = self.next()?.ok_or_else(|| self.eof_error())?;

        let mut left = self.atom(tok, start, end)?;

        while let Some((tok_start, tok, tok_end)) = self.peek()? {
            if Self::starts_atom(&tok) {
                if APPLICATION_LBP <= min_bp || self.just_closed_block() {
                    return Ok(left);
                }
                let right = self.expression(APPLICATION_RBP)?;
                let new_start = left.span.start;
                let new_end = right.span.end;
                left = surface::Expr::new(
                    surface::ExprKind::Application(Box::new(left), Box::new(right)),
                    new_start,
                    new_end,
                );
                continue;
            }

            let Some(op) = token_to_binary_op(&tok) else {
                break;
            };
            let (lbp, rbp, assoc) = Self::infix_binding_power(op);
            if lbp <= min_bp {
                break;
            }
            self.next()?;
            let right = self.expression(rbp)?;
            let new_start = left.span.start;
            let new_end = right.span.end;
            left = surface::Expr::new(
                surface::ExprKind::Binary(op, Box::new(left), Box::new(right)),
                new_start,
                new_end,
            );
            // when parsing a non-associative op, we need to see if the next op
            if !assoc
                && let Some((peek_start, peek, peek_end)) = self.peek()?
                && let Some(peek_op) = token_to_binary_op(&peek)
                && matches!(
                    Self::infix_binding_power(peek_op),
                    (l, _, _) if l == lbp
                )
            {
                return Err(ParseError::ChainNonAssociativeOp {
                    first_op: format!("{op}"),
                    first_span: (tok_start..tok_end).into(),
                    second_op: format!("{peek_op}"),
                    second_span: (peek_start..peek_end).into(),
                });
            }
            if op == BinaryOp::ForwardPipe
                && let Some((peek_start, peek, peek_end)) = self.peek()?
                && peek == Token::LtPipe
            {
                let peek_op = token_to_binary_op(&peek).expect("<| should map to op");
                return Err(ParseError::MixedPipeOperators {
                    first_op: format!("{op}"),
                    first_span: (tok_start..tok_end).into(),
                    second_op: format!("{peek_op}"),
                    second_span: (peek_start..peek_end).into(),
                });
            }
            if op == BinaryOp::BackwardPipe
                && let Some((peek_start, peek, peek_end)) = self.peek()?
                && peek == Token::PipeGT
            {
                let peek_op = token_to_binary_op(&peek).expect("|> should map to op");
                return Err(ParseError::MixedPipeOperators {
                    first_op: format!("{op}"),
                    first_span: (tok_start..tok_end).into(),
                    second_op: format!("{peek_op}"),
                    second_span: (peek_start..peek_end).into(),
                });
            }
        }
        Ok(left)
    }

    fn just_closed_block(&self) -> bool {
        matches!(self.prev_token, Some((_, Token::RBrace, _)))
    }

    // first set of `atom`, excluding negation so that `f - x` parses as (- f x) instead of `f (-x)`
    fn starts_atom(tok: &Token<'a>) -> bool {
        matches!(
            tok,
            Token::Unit
                | Token::Bool(_)
                | Token::Char(_)
                | Token::Int(_)
                | Token::String(_)
                | Token::Identifier(_)
                | Token::Keyword(Keyword::Fun | Keyword::Let | Keyword::If)
                | Token::Bang
                | Token::Tilde
                | Token::LParen
        )
    }

    fn atom(
        &mut self,
        tok: Token<'a>,
        start: usize,
        end: usize,
    ) -> Result<surface::Expr<'a>, ParseError> {
        match tok {
            Token::Unit => Ok(surface::Expr::new(
                surface::ExprKind::Literal(surface::Literal::Unit),
                start,
                end,
            )),
            Token::Bool(b) => Ok(surface::Expr::new(
                surface::ExprKind::Literal(surface::Literal::Bool(b)),
                start,
                end,
            )),
            Token::Char(c) => Ok(surface::Expr::new(
                surface::ExprKind::Literal(surface::Literal::Char(c)),
                start,
                end,
            )),
            Token::Int(n) => Ok(surface::Expr::new(
                surface::ExprKind::Literal(surface::Literal::Int(n)),
                start,
                end,
            )),
            Token::String(s) => Ok(surface::Expr::new(
                surface::ExprKind::Literal(surface::Literal::String(s)),
                start,
                end,
            )),
            Token::Identifier(name) => {
                let (path, new_end) = self.qualified_name(name, start, end)?;
                if path.qualifiers.is_empty() {
                    Ok(surface::Expr::new(
                        surface::ExprKind::Variable(name),
                        start,
                        end,
                    ))
                } else {
                    Ok(surface::Expr::new(
                        surface::ExprKind::Path(path),
                        start,
                        new_end,
                    ))
                }
            }
            Token::Keyword(Keyword::Fun) => self.abstraction(start),
            Token::Keyword(Keyword::Let) => self.let_in(start),
            Token::Keyword(Keyword::If) => self.if_else(start),
            Token::Keyword(Keyword::Match) => self.match_expression(start),
            Token::LParen => {
                let e = self.expression(0)?;
                if self.chomp_if_next_is(Token::Comma)? {
                    let mut elements = vec![e, self.expression(0)?];
                    while self.chomp_if_next_is(Token::Comma)? {
                        elements.push(self.expression(0)?);
                    }
                    let (_, _, paren_end) = self.expect(Token::RParen)?;
                    return Ok(surface::Expr::new(
                        surface::ExprKind::Tuple(elements),
                        start,
                        paren_end,
                    ));
                }
                let (_, _, paren_end) = self.expect(Token::RParen)?;
                Ok(surface::Expr::new(e.kind, start, paren_end))
            }
            Token::LBrace => {
                let body = self.block_body(end, false)?;
                let (_, _, block_end) = self.expect(Token::RBrace)?;
                Ok(surface::Expr::new(body.kind, start, block_end))
            }
            _ => {
                if let Some((op, rbp)) = Parser::prefix_binding_power(&tok) {
                    let e = self.expression(rbp)?;
                    let new_end = e.span.end;
                    Ok(surface::Expr::new(
                        surface::ExprKind::Unary(op, Box::new(e)),
                        start,
                        new_end,
                    ))
                } else {
                    Err(ParseError::UnexpectedToken {
                        tok: format!("{tok:?}"),
                        span: (start..end).into(),
                    })
                }
            }
        }
    }

    fn qualified_name(
        &mut self,
        name: &'a str,
        start: usize,
        end: usize,
    ) -> Result<(surface::Path<'a>, usize), ParseError> {
        let mut qualifiers = Vec::new();
        let mut last = surface::Identifier {
            name,
            span: (start..end).into(),
        };
        let mut new_end = end;
        while let Some((colon_start, Token::ColonColon, colon_end)) = self.peek()? {
            if colon_start != new_end {
                break;
            }
            self.next()?;
            let ident = self.expect_identifier()?;
            if ident.span.start != colon_end {
                return Err(ParseError::BadPath {
                    span: (colon_start..ident.span.end).into(),
                });
            }
            new_end = ident.span.end;
            qualifiers.push(std::mem::replace(&mut last, ident));
        }
        Ok((
            surface::Path {
                qualifiers,
                name: last,
                span: (start..new_end).into(),
            },
            new_end,
        ))
    }

    fn abstraction(&mut self, start: usize) -> Result<surface::Expr<'a>, ParseError> {
        // already read `fun`
        let mut variables = Vec::new();
        if !self.chomp_if_next_is(Token::WideArrow)? {
            loop {
                variables.push(self.expect_identifier()?);
                if !self.chomp_if_next_is(Token::Comma)? {
                    break;
                }
            }
            self.expect(Token::WideArrow)?;
        }
        let body = self.expression(0)?;
        let end = body.span.end;
        Ok(surface::Expr::new(
            surface::ExprKind::Abstraction(variables, Box::new(body)),
            start,
            end,
        ))
    }

    fn let_in(&mut self, start: usize) -> Result<surface::Expr<'a>, ParseError> {
        // already read `let`
        let is_recursive = self.chomp_if_next_is(Token::Keyword(Keyword::Rec))?;
        let pattern = self.pattern()?;
        self.expect(Token::Equal)?;
        let expr = self.expression(0)?;
        self.expect(Token::Semi)?;
        let body = self.expression(0)?;
        let end = body.span.end;
        Ok(surface::Expr::new(
            surface::ExprKind::LetIn(is_recursive, pattern, Box::new(expr), Box::new(body)),
            start,
            end,
        ))
    }

    fn pattern(&mut self) -> Result<surface::Pattern<'a>, ParseError> {
        match self.next()? {
            Some((start, Token::Unit, end)) => Ok(surface::Pattern {
                kind: surface::PatternKind::Literal(surface::Literal::Unit),
                span: (start..end).into(),
            }),
            Some((start, Token::Bool(b), end)) => Ok(surface::Pattern {
                kind: surface::PatternKind::Literal(surface::Literal::Bool(b)),
                span: (start..end).into(),
            }),
            Some((start, Token::Char(c), end)) => Ok(surface::Pattern {
                kind: surface::PatternKind::Literal(surface::Literal::Char(c)),
                span: (start..end).into(),
            }),
            Some((start, Token::Int(n), end)) => Ok(surface::Pattern {
                kind: surface::PatternKind::Literal(surface::Literal::Int(n)),
                span: (start..end).into(),
            }),
            Some((start, Token::String(s), end)) => Ok(surface::Pattern {
                kind: surface::PatternKind::Literal(surface::Literal::String(s)),
                span: (start..end).into(),
            }),
            Some((start, Token::Identifier(x), end)) => match x {
                "_" => Ok(surface::Pattern {
                    kind: surface::PatternKind::Wild,
                    span: (start..end).into(),
                }),
                x => Ok(surface::Pattern {
                    kind: surface::PatternKind::Variable(x),
                    span: (start..end).into(),
                }),
            },
            Some((start, Token::LParen, _)) => {
                let mut elements = vec![self.pattern()?];
                self.expect(Token::Comma)?;
                elements.push(self.pattern()?);
                while self.chomp_if_next_is(Token::Comma)? {
                    elements.push(self.pattern()?);
                }
                let (_, _, end) = self.expect(Token::RParen)?;
                Ok(surface::Pattern {
                    kind: surface::PatternKind::Tuple(elements),
                    span: (start..end).into(),
                })
            }
            Some((start, tok, end)) => Err(ParseError::UnexpectedTokenExpecting {
                actual: format!("{tok:?}"),
                expected: "pattern".to_string(),
                span: (start..end).into(),
            }),
            None => Err(self.eof_error()),
        }
    }

    fn if_else(&mut self, start: usize) -> Result<surface::Expr<'a>, ParseError> {
        // already read `if`
        let condition = self.expression(0)?;
        self.expect(Token::Keyword(Keyword::Then))?;
        let consequence = self.expression(0)?;
        self.expect(Token::Keyword(Keyword::Else))?;
        let alternative = self.expression(0)?;
        let end = alternative.span.end;
        Ok(surface::Expr::new(
            surface::ExprKind::IfElse(
                Box::new(condition),
                Box::new(consequence),
                Box::new(alternative),
            ),
            start,
            end,
        ))
    }

    fn match_expression(&mut self, start: usize) -> Result<surface::Expr<'a>, ParseError> {
        let scrutinee = if let Some((_, Token::LBrace, _)) = self.peek()? {
            None
        } else {
            Some(Box::new(self.expression(0)?))
        };
        let (arms, end) = self.match_body()?;
        Ok(surface::Expr::new(
            surface::ExprKind::Match(scrutinee, arms),
            start,
            end,
        ))
    }

    fn match_body(
        &mut self,
    ) -> Result<(Vec<(surface::Pattern<'a>, surface::Expr<'a>)>, usize), ParseError> {
        self.expect(Token::LBrace)?;
        let mut arms = Vec::new();
        loop {
            let pat = self.pattern()?;
            self.expect(Token::WideArrow)?;
            let expr = self.expression(0)?;
            arms.push((pat, expr));
            match self.peek()? {
                Some((_, Token::Comma, _)) => {
                    self.next()?;
                    if let Some((_, Token::RBrace, end)) = self.peek()? {
                        self.next()?;
                        return Ok((arms, end));
                    }
                    // continue
                }
                Some((_, Token::RBrace, end)) => {
                    self.next()?;
                    return Ok((arms, end));
                }
                Some((start, tok, end)) => {
                    return Err(ParseError::UnexpectedTokenExpecting {
                        actual: format!("{tok:?}"),
                        expected: "Comma or RBrace".to_string(),
                        span: (start..end).into(),
                    });
                }
                None => return Err(self.eof_error()),
            }
        }
    }

    fn prefix_binding_power(tok: &Token) -> Option<(UnaryOp, u8)> {
        match tok {
            Token::Dash => Some((UnaryOp::Negate, PREFIX_RBP)),
            Token::Tilde => Some((UnaryOp::BitNot, PREFIX_RBP)),
            Token::Bang => Some((UnaryOp::Not, PREFIX_RBP)),
            _ => None,
        }
    }

    fn infix_binding_power(op: BinaryOp) -> (u8, u8, bool) {
        match op {
            BinaryOp::ForwardPipe => (1, 2, true),
            BinaryOp::BackwardPipe => (2, 1, true),
            BinaryOp::Or => (3, 4, true),
            BinaryOp::And => (5, 6, true),
            BinaryOp::Lt
            | BinaryOp::Gt
            | BinaryOp::Lte
            | BinaryOp::Gte
            | BinaryOp::Eq
            | BinaryOp::Neq => (7, 8, false),
            BinaryOp::Concat => (10, 9, true),
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::BitOr => (11, 12, true),
            BinaryOp::Mul | BinaryOp::Div | BinaryOp::BitAnd => (13, 14, true),
        }
    }

    fn eof_error(&self) -> ParseError {
        let len = self.source.len();
        ParseError::UnexpectedEOF {
            span: (len..len).into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> surface::Expr<'_> {
        Parser::parse(source).unwrap().0
    }

    fn parse_decls(source: &str) -> (surface::Expr<'_>, Vec<surface::TypeDecl<'_>>) {
        Parser::parse(source).unwrap()
    }

    fn parse_err(source: &str) -> ParseError {
        match Parser::parse(source) {
            Ok((e, _)) => panic!("expected a parse error for {source:?}, but got {e}"),
            Err(e) => e,
        }
    }

    #[test]
    fn parses_literals() {
        assert!(matches!(
            parse("()").kind,
            surface::ExprKind::Literal(surface::Literal::Unit)
        ));
        assert!(matches!(
            parse("true").kind,
            surface::ExprKind::Literal(surface::Literal::Bool(true))
        ));
        assert!(matches!(
            parse("false").kind,
            surface::ExprKind::Literal(surface::Literal::Bool(false))
        ));
        assert!(matches!(
            parse("1").kind,
            surface::ExprKind::Literal(surface::Literal::Int(1))
        ));
        assert!(matches!(parse("x").kind, surface::ExprKind::Variable("x")));
        assert!(matches!(
            parse("x_y").kind,
            surface::ExprKind::Variable("x_y")
        ));
        assert!(matches!(
            parse("x'''").kind,
            surface::ExprKind::Variable("x'''")
        ));
        assert!(matches!(
            parse("'a'").kind,
            surface::ExprKind::Literal(surface::Literal::Char('a'))
        ));
        assert!(matches!(
            parse(r#""hi""#).kind,
            surface::ExprKind::Literal(surface::Literal::String("hi"))
        ));
    }

    #[test]
    fn parses_expressions() {
        assert_eq!(parse("(((x)))").to_string(), "x");
        assert_eq!(parse("{}").to_string(), "()");
        assert_eq!(parse("{ a; }").to_string(), "(a; ())");
        assert_eq!(parse("a; b;").to_string(), "(a; (b; ()))");
        assert_eq!(
            parse("a ++ b ++ c == d").to_string(),
            "((a ++ (b ++ c)) == d)"
        );
        assert_eq!(
            parse("a != b && c != d").to_string(),
            "((a != b) && (c != d))"
        );
        assert_eq!(parse("fun x => x").to_string(), "(fun x => x)");
        assert_eq!(parse("let x = x; x").to_string(), "(let x = x; x)");
        assert_eq!(parse("let rec x = x; x").to_string(), "(let rec x = x; x)");
        assert_eq!(
            parse("if x then x else x").to_string(),
            "(if x then x else x)"
        );
        assert_eq!(parse("f g").to_string(), "(f g)");
        assert_eq!(parse("f g h").to_string(), "((f g) h)");
        assert_eq!(parse("f (g h)").to_string(), "(f (g h))");
        assert_eq!(parse("x'y").to_string(), "(x' y)");
        assert_eq!(parse("f 1").to_string(), "(f 1)");
        assert_eq!(parse("f (1)").to_string(), "(f 1)");
        assert_eq!(parse("1 + 2").to_string(), "(1 + 2)");
        assert_eq!(parse("1 + { 2 }").to_string(), "(1 + 2)");
        assert_eq!(parse("{ 1 } + 2").to_string(), "(1 + 2)");
        assert_eq!(parse("1 + 2 + 3").to_string(), "((1 + 2) + 3)");
        assert_eq!(parse("1 + 2 * 3").to_string(), "(1 + (2 * 3))");
        assert_eq!(parse("1 * 2 * 3").to_string(), "((1 * 2) * 3)");
        assert_eq!(parse("1 + - 2").to_string(), "(1 + (- 2))");
        assert_eq!(
            parse("1 + - - - - 2").to_string(),
            "(1 + (- (- (- (- 2)))))"
        );
        assert_eq!(parse("1 + 2 * - 3").to_string(), "(1 + (2 * (- 3)))");
        assert_eq!(
            parse("1 + 2 * - 3 < 4").to_string(),
            "((1 + (2 * (- 3))) < 4)"
        );
        assert_eq!(parse("1 < 2 && 3 < 4").to_string(), "((1 < 2) && (3 < 4))");
        assert_eq!(parse("a || b && c").to_string(), "(a || (b && c))");
        assert_eq!(parse("x-y").to_string(), "(x - y)");
        assert_eq!(parse("fun x => f x g").to_string(), "(fun x => ((f x) g))");
        assert_eq!(
            parse("fun x => f (g x)").to_string(),
            "(fun x => (f (g x)))"
        );
        assert_eq!(
            parse("(fun x => x) (fun y => y + 1) 4").to_string(),
            "(((fun x => x) (fun y => (y + 1))) 4)"
        );
        assert_eq!(
            parse("fun x => { let y = x; y }").to_string(),
            "(fun x => (let y = x; y))"
        );
        assert_eq!(
            parse("let f = fun n => n + 1; f 2").to_string(),
            "(let f = (fun n => (n + 1)); (f 2))"
        );
        assert_eq!(parse("let x = 1; b; x").to_string(), "(let x = 1; (b; x))");
        assert_eq!(
            parse("let f = fun x => x; let g = fun y => y + 1; f g 4").to_string(),
            "(let f = (fun x => x); (let g = (fun y => (y + 1)); ((f g) 4)))"
        );
        assert_eq!(
            parse("let rec f = fun x => x; let g = fun y => y + 1; f g 4").to_string(),
            "(let rec f = (fun x => x); (let g = (fun y => (y + 1)); ((f g) 4)))"
        );
        assert_eq!(
            parse("let f = fun x => { x } f 3").to_string(),
            "(let f = (fun x => x); (f 3))"
        );
        assert_eq!(
            parse("let x = if condition then fun y => y + 1 else fun z => z + 2; x 3").to_string(),
            "(let x = (if condition then (fun y => (y + 1)) else (fun z => (z + 2))); (x 3))"
        );
        assert_eq!(
            parse("let _ = print \"hi\"; 1").to_string(),
            "(let _ = (print \"hi\"); 1)"
        );
        assert_eq!(parse("let x = 1; x").to_string(), "(let x = 1; x)");
        assert_eq!(
            parse("let (x, y) = (1, 2); x + y").to_string(),
            "(let (x, y) = (1, 2); (x + y))"
        );
        assert_eq!(
            parse("let (x, _, z) = (1, 2, 3); x + z").to_string(),
            "(let (x, _, z) = (1, 2, 3); (x + z))"
        );
        assert_eq!(
            parse("if x then 1 else 2").to_string(),
            "(if x then 1 else 2)"
        );
        assert_eq!(
            parse("if x then {1} else {2}").to_string(),
            "(if x then 1 else 2)"
        );
        assert_eq!(
            parse("if x then { let y = 1; y } else {2}").to_string(),
            "(if x then (let y = 1; y) else 2)"
        );
        assert_eq!(
            parse("if x then fun y => y + x else fun z => x * z").to_string(),
            "(if x then (fun y => (y + x)) else (fun z => (x * z)))"
        );
        assert_eq!(
            parse("if 1 + 2 * 3 < 4 then 1 else if 5 > 6 - 7 / 8 then 2 else 3").to_string(),
            "(if ((1 + (2 * 3)) < 4) then 1 else (if (5 > (6 - (7 / 8))) then 2 else 3))"
        );
        assert_eq!(
            parse("let rec f = fun n => if n == 0 then 1 else n * f (n - 1); f").to_string(),
            "(let rec f = (fun n => (if (n == 0) then 1 else (n * (f (n - 1))))); f)"
        );
        assert_eq!(parse("fun => 1").to_string(), "(fun => 1)");
        assert_eq!(
            parse("fun x, y, z => x + y + z").to_string(),
            "(fun x, y, z => ((x + y) + z))"
        );
        assert_eq!(parse("(1, 2)").to_string(), "(1, 2)");
        assert_eq!(parse("((), 'a')").to_string(), "((), 'a')");
        assert_eq!(parse("((1, 2), 3)").to_string(), "((1, 2), 3)");
        assert_eq!(parse("(1, (2, 3))").to_string(), "(1, (2, 3))");
        assert_eq!(parse("((1, 2), (3, 4))").to_string(), "((1, 2), (3, 4))");
        assert_eq!(
            parse("(fun x => (x, x), fun y => (y, y))").to_string(),
            "((fun x => (x, x)), (fun y => (y, y)))"
        );
        assert_eq!(
            parse("(fun x => x, fun y, z => (y, z))").to_string(),
            "((fun x => x), (fun y, z => (y, z)))"
        );
        assert_eq!(
            parse("fun x, y => (x, y)").to_string(),
            "(fun x, y => (x, y))"
        );
    }

    #[test]
    fn pattern_matching() {
        assert_eq!(
            parse("match (1, 2) { (1, 2) => 1 }").to_string(),
            "(match (1, 2) { (1, 2) => 1 })"
        );
        assert_eq!(
            parse("match x { y => 1 }").to_string(),
            "(match x { y => 1 })"
        );
        assert_eq!(
            parse("match x { y => 1, _ => 2 }").to_string(),
            "(match x { y => 1, _ => 2 })"
        );
        assert_eq!(
            parse("match x { (y, z, 3) => 1, (_, 1, 2) => 2, _ => 3, }").to_string(),
            "(match x { (y, z, 3) => 1, (_, 1, 2) => 2, _ => 3 })"
        );
        assert_eq!(
            parse("match { (y, z, 3) => 1, (_, 1, 2) => 2, _ => 3, }").to_string(),
            "(match { (y, z, 3) => 1, (_, 1, 2) => 2, _ => 3 })"
        );
        assert_eq!(
            parse("f x |> match { (y, z, 3) => 1, (_, 1, 2) => 2, _ => 3, }").to_string(),
            "((f x) |> (match { (y, z, 3) => 1, (_, 1, 2) => 2, _ => 3 }))"
        );
        assert_eq!(
            parse("f (match { (y, z, 3) => 1, (_, 1, 2) => 2, _ => 3, })").to_string(),
            "(f (match { (y, z, 3) => 1, (_, 1, 2) => 2, _ => 3 }))"
        );
        assert_eq!(
            parse("match x { () => 1 }").to_string(),
            "(match x { () => 1 })"
        );
        assert_eq!(
            parse("match x { ((), (1, ()), ()) => 2 }").to_string(),
            "(match x { ((), (1, ()), ()) => 2 })"
        );
    }

    #[test]
    fn type_declarations() {
        let (p, ds) = parse_decls("type None = None;");
        assert_eq!(p.to_string(), "()");
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].to_string(), "type None\n  = None;");

        let (p, ds) = parse_decls("type Foo = Bar | Baz Int;");
        assert_eq!(p.to_string(), "()");
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].to_string(), "type Foo\n  = Bar\n  | Baz Int;");

        let (p, ds) = parse_decls("type Maybe a = Just a | Nothing;");
        assert_eq!(p.to_string(), "()");
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].to_string(), "type Maybe a\n  = Just a\n  | Nothing;");

        let (p, ds) = parse_decls("type Either a b = Left a | Right b;");
        assert_eq!(p.to_string(), "()");
        assert_eq!(ds.len(), 1);
        assert_eq!(
            ds[0].to_string(),
            "type Either a b\n  = Left a\n  | Right b;"
        );

        let (p, ds) = parse_decls(
            "type Expr = Var Int | Abs Expr | App Expr Expr; let e1 = Expr::Var 0; let e2 = Expr::Abs e1; let e = Expr::App e1 e2; e",
        );
        assert_eq!(
            p.to_string(),
            "(let e1 = (Expr::Var 0); (let e2 = (Expr::Abs e1); (let e = ((Expr::App e1) e2); e)))"
        );
        assert_eq!(ds.len(), 1);
        assert_eq!(
            ds[0].to_string(),
            "type Expr\n  = Var Int\n  | Abs Expr\n  | App Expr Expr;"
        );

        let decl = |source| parse_decls(source).1.remove(0).to_string();
        assert_eq!(
            decl("type List a = Nil | Cons a (List a);"),
            "type List a\n  = Nil\n  | Cons a (List a);"
        );
        assert_eq!(
            decl("type F a b = F (a -> b -> a) ((a -> b) -> a);"),
            "type F a b\n  = F (a -> b -> a) ((a -> b) -> a);"
        );
        assert_eq!(
            decl("type P = P (Int, List Int) (Other::T Int) ((Int));"),
            "type P\n  = P (Int, List Int) (Other::T Int) Int;"
        );
        assert_eq!(
            decl("type A = A (List a -> Map k (Option v));"),
            "type A\n  = A (List a -> Map k (Option v));"
        );

        let err = parse_err("type B = B ((a -> b) c);");
        let ParseError::UnexpectedTokenExpecting { expected, .. } = err else {
            panic!("wrong error: {err:?}");
        };
        assert_eq!(expected, "RParen");
    }

    #[track_caller]
    fn assert_expecting(source: &str, actual: &str, expected: &str, span: std::ops::Range<usize>) {
        let err = parse_err(source);
        let ParseError::UnexpectedTokenExpecting {
            actual: a,
            expected: e,
            span: s,
        } = err
        else {
            panic!("wrong error for {source:?}: {err:?}");
        };
        assert_eq!(
            (a.as_str(), e.as_str(), s),
            (actual, expected, SourceSpan::from(span)),
            "for {source:?}"
        );
    }

    #[track_caller]
    fn assert_eof(source: &str) {
        let err = parse_err(source);
        let ParseError::UnexpectedEOF { span } = err else {
            panic!("wrong error for {source:?}: {err:?}");
        };
        let len = source.len();
        assert_eq!(span, SourceSpan::from(len..len), "for {source:?}");
    }

    #[track_caller]
    fn assert_unexpected(source: &str, tok: &str, span: std::ops::Range<usize>) {
        let err = parse_err(source);
        let ParseError::UnexpectedToken { tok: t, span: s } = err else {
            panic!("wrong error for {source:?}: {err:?}");
        };
        assert_eq!(
            (t.as_str(), s),
            (tok, SourceSpan::from(span)),
            "for {source:?}"
        );
    }

    #[test]
    fn bad_block() {
        assert!(matches!(
            parse_err("f { 1 }"),
            ParseError::ExpectedEOF { .. }
        ));
    }

    #[test]
    fn bad_binders() {
        assert_expecting("let x 1", "Int(1)", "Equal", 6..7);
        assert_expecting("let let = 1", "Keyword(Let)", "pattern", 4..7);
        assert_expecting("fun x x", "Identifier(\"x\")", "WideArrow", 6..7);
        assert_eof("fun x =>");
    }

    #[test]
    fn bad_if() {
        assert_expecting("if a else b", "Keyword(Else)", "Keyword(Then)", 5..9);
        assert_eof("if a then b");
    }

    #[test]
    fn missing_semicolons() {
        assert_eof("let x = 1");
        assert_expecting("{ let x = 1 }", "RBrace", "Semi", 12..13);
        // `1 x` is parsed as an application
        assert_eof("let x = 1 x");
    }

    #[test]
    fn unbalanced_delimiters() {
        assert_eof("(1");
        assert_eof("{ a");
        assert_expecting("(1 }", "RBrace", "RParen", 3..4);
        assert_expecting("{ 1 )", "RParen", "RBrace", 4..5);

        let err = parse_err("a }");
        let ParseError::ExpectedEOF { tok, span } = err else {
            panic!("wrong error: {err:?}");
        };
        assert_eq!((tok.as_str(), span), ("RBrace", SourceSpan::from(2..3)));

        let err = parse_err("1 )");
        let ParseError::ExpectedEOF { tok, span } = err else {
            panic!("wrong error: {err:?}");
        };
        assert_eq!((tok.as_str(), span), ("RParen", SourceSpan::from(2..3)));
    }

    #[test]
    fn stray_tokens() {
        assert_unexpected("+ 1", "Plus", 0..1);
        assert_unexpected("then", "Keyword(Then)", 0..4);
        assert_unexpected("1 + )", "RParen", 4..5);
        assert_unexpected("a; ;", "Semi", 3..4);
        assert_eof("1 +");
    }

    #[test]
    fn chained_non_associative_ops() {
        for (source, first, first_span, second, second_span) in [
            ("a < b < c", "<", 2..3, "<", 6..7),
            ("a < b > c", "<", 2..3, ">", 6..7),
            ("a == b <= c", "==", 2..4, "<=", 7..9),
        ] {
            let err = parse_err(source);
            let ParseError::ChainNonAssociativeOp {
                first_op: f,
                first_span: fs,
                second_op: s,
                second_span: ss,
            } = err
            else {
                panic!("wrong error for {source:?}: {err:?}");
            };
            assert_eq!(
                (f.as_str(), fs),
                (first, SourceSpan::from(first_span)),
                "for {source:?}"
            );
            assert_eq!(
                (s.as_str(), ss),
                (second, SourceSpan::from(second_span)),
                "for {source:?}"
            );
        }
    }

    #[test]
    fn detects_mixed_pipes() {
        for (source, first, first_span, second, second_span) in [
            ("x |> f <| ", "|>", 2..4, "<|", 7..9),
            ("f <| x |> g ", "<|", 2..4, "|>", 7..9),
        ] {
            let err = parse_err(source);
            let ParseError::MixedPipeOperators {
                first_op: f,
                first_span: fs,
                second_op: s,
                second_span: ss,
            } = err
            else {
                panic!("wrong error for {source:?}: {err:?}");
            };
            assert_eq!(
                (f.as_str(), fs),
                (first, SourceSpan::from(first_span)),
                "for {source:?}"
            );
            assert_eq!(
                (s.as_str(), ss),
                (second, SourceSpan::from(second_span)),
                "for {source:?}"
            );
        }
    }

    #[test]
    fn detects_bad_paths() {
        let err = parse_err("Foo:: Bar");
        let ParseError::BadPath { .. } = err else {
            panic!("wrong error: {err:?}");
        };

        let err = parse_err("Foo::Bar:: Baz");
        let ParseError::BadPath { .. } = err else {
            panic!("wrong error: {err:?}");
        };

        let err = parse_err("Foo::Bar ::Baz");
        let ParseError::ExpectedEOF { .. } = err else {
            panic!("wrong error: {err:?}");
        };
    }

    #[test]
    fn lex_errors_propagate() {
        assert!(matches!(
            parse_err("$"),
            ParseError::Lex(LexError::UnexpectedChar { ch: '$', .. })
        ));
        assert!(matches!(
            parse_err("1 $"),
            ParseError::Lex(LexError::UnexpectedChar { ch: '$', .. })
        ));
        assert!(matches!(
            parse_err("1 + 999999999999999999999999"),
            ParseError::Lex(LexError::IntOutOfRange { .. })
        ));
    }
}
