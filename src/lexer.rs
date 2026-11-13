use miette::{Diagnostic, SourceSpan};
use std::{fmt::Display, str::CharIndices};
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum LexError {
    #[error("unexpected character {ch:?}")]
    #[diagnostic(code(lexer::unexpected_char))]
    UnexpectedChar {
        ch: char,
        #[label("this character was not expected")]
        span: SourceSpan,
    },
    #[error("integer literal is too large")]
    #[diagnostic(code(lexer::int_out_of_range))]
    IntOutOfRange {
        #[label("doesn't fit in 32 bits")]
        span: SourceSpan,
    },
    #[error("not a character")]
    #[diagnostic(code(lexer::not_a_character))]
    NotACharacter {
        #[label("single quotes denote a character")]
        span: SourceSpan,
    },
    #[error("unterminated string")]
    #[diagnostic(code(lexer::unterminated_string))]
    UnterminatedString {
        #[label("no closing quote found for string")]
        span: SourceSpan,
    },
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum Keyword {
    Fun,
    Let,
    Rec,
    If,
    Then,
    Else,
    Type,
}

impl Display for Keyword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Keyword::Fun => write!(f, "fun"),
            Keyword::Let => write!(f, "let"),
            Keyword::Rec => write!(f, "rec"),
            Keyword::If => write!(f, "if"),
            Keyword::Then => write!(f, "then"),
            Keyword::Else => write!(f, "else"),
            Keyword::Type => write!(f, "type"),
        }
    }
}

impl Keyword {
    pub fn from_identifier(ident: &str) -> Option<Keyword> {
        match ident {
            "fun" => Some(Keyword::Fun),
            "let" => Some(Keyword::Let),
            "rec" => Some(Keyword::Rec),
            "if" => Some(Keyword::If),
            "then" => Some(Keyword::Then),
            "else" => Some(Keyword::Else),
            "type" => Some(Keyword::Type),
            _ => None,
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum Token<'a> {
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Bang,
    Tilde,
    Plus,
    PlusPlus,
    Dash,
    Star,
    Slash,
    Amp,
    AmpAmp,
    Pipe,
    PipePipe,
    Lt,
    LtLt,
    Lte,
    Gt,
    GtGt,
    Gte,
    Equal,
    EqualEqual,
    WideArrow,
    Unit,
    Bool(bool),
    Char(char), // utf-8 code point
    Int(i64),
    String(&'a str),
    Identifier(&'a str),
    Keyword(Keyword),
}

pub type Spanned<'a> = (usize, Token<'a>, usize);

pub struct Lexer<'a> {
    source: &'a str,
    chars: CharIndices<'a>,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Lexer {
            source,
            chars: source.char_indices(),
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.chars.clone().next().map(|(_, c)| c)
    }

    fn chomp_while(&mut self, pred: impl Fn(char) -> bool) -> usize {
        while self.peek_char().is_some_and(&pred) {
            self.chars.next();
        }
        self.chars.offset()
    }

    fn skip_whitespace(&mut self) {
        self.chomp_while(char::is_whitespace);
    }

    fn skip_trivia(&mut self) {
        loop {
            self.skip_whitespace();
            if self.peek_char() == Some('#') {
                self.chars.next();
                self.chomp_while(|c| c != '\n');
                continue;
            }
            break;
        }
    }

    fn is_identifier_start(ch: char) -> bool {
        ch.is_alphabetic() || ch == '_'
    }

    fn is_identifier_rest(ch: char) -> bool {
        ch.is_alphabetic() || ch.is_ascii_digit() || ch == '_'
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<Spanned<'a>, LexError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.skip_trivia();
        let (start, ch) = self.chars.next()?;
        let tok = match ch {
            '(' => {
                if self.peek_char() == Some(')') {
                    self.chars.next();
                    Token::Unit
                } else {
                    Token::LParen
                }
            }
            ')' => Token::RParen,
            '{' => Token::LBrace,
            '}' => Token::RBrace,
            ',' => Token::Comma,
            ';' => Token::Semi,
            '!' => Token::Bang,
            '~' => Token::Tilde,
            '+' => {
                if self.peek_char() == Some('+') {
                    self.chars.next();
                    Token::PlusPlus
                } else {
                    Token::Plus
                }
            }
            '-' => Token::Dash,
            '*' => Token::Star,
            '/' => Token::Slash,
            '&' => {
                if self.peek_char() == Some('&') {
                    self.chars.next();
                    Token::AmpAmp
                } else {
                    Token::Amp
                }
            }
            '|' => {
                if self.peek_char() == Some('|') {
                    self.chars.next();
                    Token::PipePipe
                } else {
                    Token::Pipe
                }
            }
            '<' => match self.peek_char() {
                Some('<') => {
                    self.chars.next();
                    Token::LtLt
                }
                Some('=') => {
                    self.chars.next();
                    Token::Lte
                }
                _ => Token::Lt,
            },
            '>' => match self.peek_char() {
                Some('>') => {
                    self.chars.next();
                    Token::GtGt
                }
                Some('=') => {
                    self.chars.next();
                    Token::Gte
                }
                _ => Token::Gt,
            },
            '=' => match self.peek_char() {
                Some('=') => {
                    self.chars.next();
                    Token::EqualEqual
                }
                Some('>') => {
                    self.chars.next();
                    Token::WideArrow
                }
                _ => Token::Equal,
            },
            '0'..='9' => {
                let end = self.chomp_while(|c| c.is_ascii_digit());
                if let Some(ident_ch) = self.peek_char().filter(|&c| Self::is_identifier_start(c)) {
                    let pos = self.chars.offset();
                    return Some(Err(LexError::UnexpectedChar {
                        ch: ident_ch,
                        span: (pos..pos + ident_ch.len_utf8()).into(),
                    }));
                }
                match self.source[start..end].parse() {
                    Ok(n) => Token::Int(n),
                    Err(_) => {
                        return Some(Err(LexError::IntOutOfRange {
                            span: (start..end).into(),
                        }));
                    }
                }
            }
            '\'' => {
                let c = match self.chars.next() {
                    Some((_, '\\')) => match self.chars.next() {
                        Some((_, 'n')) => '\n',
                        Some((_, 'r')) => '\r',
                        Some((_, 't')) => '\t',
                        Some((_, '0')) => '\0',
                        Some((_, '\\')) => '\\',
                        Some((_, '\'')) => '\'',
                        _ => {
                            return Some(Err(LexError::NotACharacter {
                                span: (start..self.chars.offset()).into(),
                            }));
                        }
                    },
                    Some((_, '\'')) | None => {
                        return Some(Err(LexError::NotACharacter {
                            span: (start..self.chars.offset()).into(),
                        }));
                    }
                    Some((_, c)) => c,
                };
                if !matches!(self.chars.next(), Some((_, '\''))) {
                    return Some(Err(LexError::NotACharacter {
                        span: (start..self.chars.offset()).into(),
                    }));
                }
                Token::Char(c)
            }
            '"' => {
                loop {
                    match self.chars.next() {
                        Some((_, '\\')) => {
                            self.chars.next();
                        }
                        Some((_, '"')) => {
                            break;
                        }
                        Some(_) => {}
                        _ => {
                            return Some(Err(LexError::UnterminatedString {
                                span: (start..self.chars.offset()).into(),
                            }));
                        }
                    }
                }
                let end = self.chars.offset();
                let value = &self.source[(start + 1)..(end - 1)];
                Token::String(value)
            }
            _ if Self::is_identifier_start(ch) => {
                self.chomp_while(Self::is_identifier_rest);
                let end = self.chomp_while(|c| c == '\''); // only allow trailing primes
                let value = &self.source[start..end];
                match value {
                    "true" => Token::Bool(true),
                    "false" => Token::Bool(false),
                    _ => {
                        if let Some(kw) = Keyword::from_identifier(value) {
                            Token::Keyword(kw)
                        } else {
                            Token::Identifier(value)
                        }
                    }
                }
            }
            _ => {
                return Some(Err(LexError::UnexpectedChar {
                    ch,
                    span: (start..self.chars.offset()).into(),
                }));
            }
        };
        Some(Ok((start, tok, self.chars.offset())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(src: &str) -> Vec<Token<'_>> {
        Lexer::new(src).map(|r| r.unwrap().1).collect()
    }

    #[test]
    fn skips_trivia() {
        assert_eq!(tokens(""), vec![]);
        assert_eq!(tokens("    \n\t\r  "), vec![]);
        assert_eq!(tokens("#comment"), vec![]);
        assert_eq!(tokens("# comment\n\n\n#   another comment\n"), vec![]);
    }

    #[test]
    fn lexes_simple_program() {
        assert_eq!(
            tokens("let rec f = fun n => {\n  if n < 2 then 1 else n + f (n - 1)\n} f 3"),
            vec![
                Token::Keyword(Keyword::Let),
                Token::Keyword(Keyword::Rec),
                Token::Identifier("f"),
                Token::Equal,
                Token::Keyword(Keyword::Fun),
                Token::Identifier("n"),
                Token::WideArrow,
                Token::LBrace,
                Token::Keyword(Keyword::If),
                Token::Identifier("n"),
                Token::Lt,
                Token::Int(2),
                Token::Keyword(Keyword::Then),
                Token::Int(1),
                Token::Keyword(Keyword::Else),
                Token::Identifier("n"),
                Token::Plus,
                Token::Identifier("f"),
                Token::LParen,
                Token::Identifier("n"),
                Token::Dash,
                Token::Int(1),
                Token::RParen,
                Token::RBrace,
                Token::Identifier("f"),
                Token::Int(3),
            ]
        );
    }

    #[test]
    fn lexes_chars_and_strings() {
        assert_eq!(
            tokens(r"'a' '\n' '\''"),
            vec![Token::Char('a'), Token::Char('\n'), Token::Char('\'')]
        );
        assert_eq!(
            tokens(r#""" "a\"b""#),
            vec![Token::String(""), Token::String(r#"a\"b"#)]
        );
        assert!(matches!(
            Lexer::new("''").next(),
            Some(Err(LexError::NotACharacter { .. }))
        ));
    }
}
