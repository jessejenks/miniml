use miette::{Diagnostic, SourceSpan};
use std::iter::Peekable;
use thiserror::Error;

use crate::ast::{BinaryOp, UnaryOp};
use crate::lexer::{Keyword, LexError, Lexer, Spanned, Token};
use crate::surface;

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

    #[error("cannot chain non-associative operator {tok}")]
    #[diagnostic(code(parser::chain_non_associative_operator))]
    ChainNonAssociativeOp {
        tok: String,
        #[label("this operator is non-associative")]
        span: SourceSpan,
    },
}

const PREFIX_RBP: u8 = 9;
const APPLICATION_LBP: u8 = 11;
const APPLICATION_RBP: u8 = 12;

fn token_to_binary_op(tok: &Token) -> Option<BinaryOp> {
    match tok {
        Token::Plus => Some(BinaryOp::Add),
        Token::Dash => Some(BinaryOp::Sub),
        Token::Star => Some(BinaryOp::Mul),
        Token::Slash => Some(BinaryOp::Div),
        Token::Lt => Some(BinaryOp::Lt),
        Token::Gt => Some(BinaryOp::Gt),
        Token::Lte => Some(BinaryOp::Lte),
        Token::Gte => Some(BinaryOp::Gte),
        Token::EqualEqual => Some(BinaryOp::Eq),
        Token::Amp => Some(BinaryOp::BitAnd),
        Token::Pipe => Some(BinaryOp::BitOr),
        Token::AmpAmp => Some(BinaryOp::And),
        Token::PipePipe => Some(BinaryOp::Or),
        _ => None,
    }
}

pub struct Parser<'a> {
    source: &'a str,
    lexer: Peekable<Lexer<'a>>,
    prev_token: Option<Spanned<'a>>,
}

impl<'a> Parser<'a> {
    pub fn parse(source: &'a str) -> Result<surface::Expr<'a>, ParseError> {
        let mut p = Parser {
            source,
            lexer: Lexer::new(source).peekable(),
            prev_token: None,
        };
        let program = p.block_body(0)?;
        if let Some((start, tok, end)) = p.peek()? {
            return Err(ParseError::ExpectedEOF {
                tok: format!("{tok:?}"),
                span: (start..end).into(),
            });
        }
        Ok(program)
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

    fn block_body(&mut self, start: usize) -> Result<surface::Expr<'a>, ParseError> {
        if matches!(self.peek()?, None | Some((_, Token::RBrace, _))) {
            return Ok(surface::Expr::new(surface::ExprKind::Unit, start, start));
        }
        if (self.chomp_if_next_is(Token::Keyword(Keyword::Let)))? {
            let (let_start, _, _) = self.prev_token.expect("just read `let`");
            let is_recursive = self.chomp_if_next_is(Token::Keyword(Keyword::Rec))?;
            let variable = self.expect_identifier()?;
            self.expect(Token::Equal)?;
            let expr = self.expression(0)?;
            let body_start = if self.just_closed_block() {
                let (_, _, brace_end) = self.prev_token.expect("just closed a block");
                self.chomp_if_next_is(Token::Semi)?;
                brace_end
            } else {
                let (_, _, semi_end) = self.expect(Token::Semi)?;
                semi_end
            };
            let body = self.block_body(body_start)?;
            let end = body.span.end;
            return Ok(surface::Expr::new(
                surface::ExprKind::LetIn(is_recursive, variable, Box::new(expr), Box::new(body)),
                let_start,
                end,
            ));
        }
        let expr = self.expression(0)?;
        let ended_with_block = self.just_closed_block();
        let at_end = matches!(self.peek()?, None | Some((_, Token::RBrace, _)));
        if self.chomp_if_next_is(Token::Semi)? || (ended_with_block && !at_end) {
            let (_, _, rest_start) = self.prev_token.expect("just read `;` or `}`");
            let rest = self.block_body(rest_start)?;
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

    fn expression(&mut self, min_bp: u8) -> Result<surface::Expr<'a>, ParseError> {
        let (start, tok, end) = self.next()?.ok_or_else(|| self.eof_error())?;

        let mut left = self.atom(tok, start, end)?;

        while let Some((_, tok, _)) = self.peek()? {
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
                    tok: format!("{peek:?}"),
                    span: (peek_start..peek_end).into(),
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
                | Token::Int(_)
                | Token::Identifier(_)
                | Token::Keyword(Keyword::Fun | Keyword::Let | Keyword::If)
                | Token::Bang
                | Token::Tilde
                | Token::LParen
                | Token::LBrace
        )
    }

    fn atom(
        &mut self,
        tok: Token<'a>,
        start: usize,
        end: usize,
    ) -> Result<surface::Expr<'a>, ParseError> {
        match tok {
            Token::Unit => Ok(surface::Expr::new(surface::ExprKind::Unit, start, end)),
            Token::Bool(b) => Ok(surface::Expr::new(surface::ExprKind::Bool(b), start, end)),
            Token::Int(n) => Ok(surface::Expr::new(surface::ExprKind::Int(n), start, end)),
            Token::Identifier(x) => Ok(surface::Expr::new(
                surface::ExprKind::Variable(x),
                start,
                end,
            )),
            Token::Keyword(Keyword::Fun) => self.abstraction(start),
            Token::Keyword(Keyword::Let) => self.let_in(start),
            Token::Keyword(Keyword::If) => self.if_else(start),
            Token::LParen => {
                let e = self.expression(0)?;
                let (_, _, paren_end) = self.expect(Token::RParen)?;
                Ok(surface::Expr::new(e.kind, start, paren_end))
            }
            Token::LBrace => {
                let body = self.block_body(end)?;
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

    fn abstraction(&mut self, start: usize) -> Result<surface::Expr<'a>, ParseError> {
        // already read `fun`
        let variable = self.expect_identifier()?;
        self.expect(Token::WideArrow)?;
        let body = self.expression(0)?;
        let end = body.span.end;
        Ok(surface::Expr::new(
            surface::ExprKind::Abstraction(variable, Box::new(body)),
            start,
            end,
        ))
    }

    fn let_in(&mut self, start: usize) -> Result<surface::Expr<'a>, ParseError> {
        // already read `let`
        let is_recursive = self.chomp_if_next_is(Token::Keyword(Keyword::Rec))?;
        let variable = self.expect_identifier()?;
        self.expect(Token::Equal)?;
        let expr = self.expression(0)?;
        self.expect(Token::Semi)?;
        let body = self.expression(0)?;
        let end = body.span.end;
        Ok(surface::Expr::new(
            surface::ExprKind::LetIn(is_recursive, variable, Box::new(expr), Box::new(body)),
            start,
            end,
        ))
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
            BinaryOp::Or => (1, 2, true),
            BinaryOp::And => (3, 4, true),
            BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Lte | BinaryOp::Gte | BinaryOp::Eq => {
                (5, 6, false)
            }
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::BitOr => (7, 8, true),
            BinaryOp::Mul | BinaryOp::Div | BinaryOp::BitAnd => (9, 10, true),
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
        Parser::parse(source).unwrap()
    }

    fn parse_err(source: &str) -> ParseError {
        match Parser::parse(source) {
            Ok(e) => panic!("expected a parse error for {source:?}, but got {e}"),
            Err(e) => e,
        }
    }

    #[test]
    fn parses_literals() {
        assert!(matches!(parse("()").kind, surface::ExprKind::Unit));
        assert!(matches!(parse("true").kind, surface::ExprKind::Bool(true)));
        assert!(matches!(
            parse("false").kind,
            surface::ExprKind::Bool(false)
        ));
        assert!(matches!(parse("1").kind, surface::ExprKind::Int(1)));
        assert!(matches!(parse("x").kind, surface::ExprKind::Variable("x")));
        assert!(matches!(
            parse("x_y").kind,
            surface::ExprKind::Variable("x_y")
        ));
        assert!(matches!(
            parse("x'''").kind,
            surface::ExprKind::Variable("x'''")
        ));
    }

    #[test]
    fn parses_expressions() {
        assert_eq!(parse("(((x)))").to_string(), "x");
        assert_eq!(parse("{}").to_string(), "()");
        assert_eq!(parse("{ a; }").to_string(), "(a; ())");
        assert_eq!(parse("a; b;").to_string(), "(a; (b; ()))");
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
        assert_eq!(parse("f { 1 }").to_string(), "(f 1)");
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
    }

    #[test]
    fn basic_syntax_errors() {
        let err = parse_err("let 1 = x");
        let ParseError::UnexpectedTokenExpecting { actual, span, .. } = err else {
            panic!("wrong error: {err:?}");
        };
        assert_eq!(actual, "Int(1)");
        assert_eq!(span, SourceSpan::from(4..5));
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
    fn bad_binders() {
        assert_expecting("let x 1", "Int(1)", "Equal", 6..7);
        assert_expecting("let let = 1", "Keyword(Let)", "identifier", 4..7);
        assert_expecting("fun => x", "WideArrow", "identifier", 4..6);
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
        for (source, tok, span) in [
            ("a < b < c", "Lt", 6..7),
            ("a < b > c", "Gt", 6..7),
            ("a == b <= c", "Lte", 7..9),
        ] {
            let err = parse_err(source);
            let ParseError::ChainNonAssociativeOp { tok: t, span: s } = err else {
                panic!("wrong error for {source:?}: {err:?}");
            };
            assert_eq!(
                (t.as_str(), s),
                (tok, SourceSpan::from(span)),
                "for {source:?}"
            );
        }
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
