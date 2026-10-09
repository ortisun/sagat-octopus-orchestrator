//! Turns source text into tokens. Iterative, with a hard token cap.

use crate::error::ExprError;
use crate::limits::{MAX_SOURCE_BYTES, MAX_TOKENS};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Token {
    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Question,
    Colon,
    Bang,
    Minus,
    Plus,
    Star,
    Slash,
    Percent,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    Ne,
    AndAnd,
    OrOr,
    In,
    True,
    False,
    Null,
    Eof,
}

pub(crate) fn lex(source: &str) -> Result<Vec<(Token, usize)>, ExprError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ExprError::TooLong);
    }
    let bytes = source.as_bytes();
    let mut out: Vec<(Token, usize)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if out.len() >= MAX_TOKENS {
            return Err(ExprError::TooComplex);
        }
        let start = i;
        let token = match c {
            b'(' => single(&mut i, Token::LParen),
            b')' => single(&mut i, Token::RParen),
            b'[' => single(&mut i, Token::LBracket),
            b']' => single(&mut i, Token::RBracket),
            b',' => single(&mut i, Token::Comma),
            b'.' => single(&mut i, Token::Dot),
            b'?' => single(&mut i, Token::Question),
            b':' => single(&mut i, Token::Colon),
            b'+' => single(&mut i, Token::Plus),
            b'-' => single(&mut i, Token::Minus),
            b'*' => single(&mut i, Token::Star),
            b'/' => single(&mut i, Token::Slash),
            b'%' => single(&mut i, Token::Percent),
            b'!' => double(bytes, &mut i, b'=', Token::Ne, Token::Bang),
            b'<' => double(bytes, &mut i, b'=', Token::Le, Token::Lt),
            b'>' => double(bytes, &mut i, b'=', Token::Ge, Token::Gt),
            b'=' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    i += 2;
                    Token::EqEq
                } else {
                    return Err(ExprError::UnexpectedCharacter(i));
                }
            }
            b'&' => {
                if bytes.get(i + 1) == Some(&b'&') {
                    i += 2;
                    Token::AndAnd
                } else {
                    return Err(ExprError::UnexpectedCharacter(i));
                }
            }
            b'|' => {
                if bytes.get(i + 1) == Some(&b'|') {
                    i += 2;
                    Token::OrOr
                } else {
                    return Err(ExprError::UnexpectedCharacter(i));
                }
            }
            b'\'' | b'"' => string(source, &mut i)?,
            b'0'..=b'9' => number(source, &mut i)?,
            b'_' | b'a'..=b'z' | b'A'..=b'Z' => {
                while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
                    i += 1;
                }
                match &source[start..i] {
                    "true" => Token::True,
                    "false" => Token::False,
                    "null" => Token::Null,
                    "in" => Token::In,
                    word => Token::Ident(word.to_owned()),
                }
            }
            _ => return Err(ExprError::UnexpectedCharacter(i)),
        };
        out.push((token, start));
    }
    out.push((Token::Eof, bytes.len()));
    Ok(out)
}

fn single(i: &mut usize, token: Token) -> Token {
    *i += 1;
    token
}

fn double(bytes: &[u8], i: &mut usize, second: u8, two: Token, one: Token) -> Token {
    if bytes.get(*i + 1) == Some(&second) {
        *i += 2;
        two
    } else {
        *i += 1;
        one
    }
}

fn number(source: &str, i: &mut usize) -> Result<Token, ExprError> {
    let bytes = source.as_bytes();
    let start = *i;
    while *i < bytes.len() && bytes[*i].is_ascii_digit() {
        *i += 1;
    }
    let mut is_float = false;
    // A '.' is part of the number only when a digit follows; otherwise it is a member access.
    if bytes.get(*i) == Some(&b'.') && bytes.get(*i + 1).is_some_and(u8::is_ascii_digit) {
        is_float = true;
        *i += 1;
        while *i < bytes.len() && bytes[*i].is_ascii_digit() {
            *i += 1;
        }
    }
    if matches!(bytes.get(*i), Some(b'e' | b'E')) {
        let mut j = *i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if bytes.get(j).is_some_and(u8::is_ascii_digit) {
            is_float = true;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            *i = j;
        }
    }
    // A number glued to a letter ("12abc") is a typo, not two tokens.
    if bytes
        .get(*i)
        .is_some_and(|b| *b == b'_' || b.is_ascii_alphabetic())
    {
        return Err(ExprError::InvalidNumber(start));
    }
    let text = &source[start..*i];
    if is_float {
        text.parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .map(Token::Float)
            .ok_or(ExprError::InvalidNumber(start))
    } else {
        text.parse::<i64>()
            .map(Token::Int)
            .map_err(|_| ExprError::InvalidNumber(start))
    }
}

fn string(source: &str, i: &mut usize) -> Result<Token, ExprError> {
    let bytes = source.as_bytes();
    let start = *i;
    let quote = bytes[*i];
    *i += 1;
    let mut out = String::new();
    loop {
        let Some(&c) = bytes.get(*i) else {
            return Err(ExprError::UnterminatedString(start));
        };
        if c == quote {
            *i += 1;
            return Ok(Token::Str(out));
        }
        if c == b'\\' {
            let escaped = match bytes.get(*i + 1) {
                Some(b'n') => '\n',
                Some(b't') => '\t',
                Some(b'r') => '\r',
                Some(b'\\') => '\\',
                Some(b'\'') => '\'',
                Some(b'"') => '"',
                _ => return Err(ExprError::InvalidEscape(*i)),
            };
            out.push(escaped);
            *i += 2;
            continue;
        }
        // Copy one whole UTF-8 character.
        let ch = source[*i..]
            .chars()
            .next()
            .ok_or(ExprError::UnterminatedString(start))?;
        out.push(ch);
        *i += ch.len_utf8();
    }
}
