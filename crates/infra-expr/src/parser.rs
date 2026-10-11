//! A precedence-climbing parser with a hard nesting depth and node budget.
//!
//! Binary operators are folded in loops, so the recursion depth follows the nesting of the
//! *source* (parentheses, lists, calls, ternaries, prefix operators), which is capped at
//! [`MAX_DEPTH`]. The token cap (every node consumes at least one token) bounds the size of the tree
//! and therefore the evaluator's recursion too.

use serde_json::Value;

use crate::error::{ExprError, clip};
use crate::lexer::{Token, lex};
use crate::limits::{MAX_DEPTH, MAX_LIST_LITERAL};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    Literal(Value),
    Var(String),
    List(Vec<Expr>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    /// `has(a.b.c)`: true when the last field exists.
    Has(Box<Expr>, String),
    Call(Function, Vec<Expr>),
    Method(Box<Expr>, Method, Vec<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Function {
    Size,
    String,
    Int,
    Double,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Size,
    Contains,
    StartsWith,
    EndsWith,
}

pub(crate) fn parse(source: &str) -> Result<Expr, ExprError> {
    let tokens = lex(source)?;
    let mut parser = Parser {
        tokens,
        at: 0,
        depth: 0,
    };
    let expr = parser.expression()?;
    if parser.peek() != &Token::Eof {
        return Err(parser.error("end of expression"));
    }
    Ok(expr)
}

struct Parser {
    tokens: Vec<(Token, usize)>,
    at: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.at.min(self.tokens.len() - 1)].0
    }

    fn position(&self) -> usize {
        self.tokens[self.at.min(self.tokens.len() - 1)].1
    }

    fn error(&self, expected: &'static str) -> ExprError {
        ExprError::Syntax {
            position: self.position(),
            expected,
        }
    }

    fn bump(&mut self) -> Token {
        let token = self.peek().clone();
        if self.at < self.tokens.len() - 1 {
            self.at += 1;
        }
        token
    }

    fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == token {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: &Token, expected: &'static str) -> Result<(), ExprError> {
        if self.eat(token) {
            Ok(())
        } else {
            Err(self.error(expected))
        }
    }

    fn enter(&mut self) -> Result<(), ExprError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            Err(ExprError::TooDeep)
        } else {
            Ok(())
        }
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    /// `expr := or ( '?' expr ':' expr )?`
    fn expression(&mut self) -> Result<Expr, ExprError> {
        self.enter()?;
        let condition = self.or()?;
        let result = if self.eat(&Token::Question) {
            let if_true = self.expression()?;
            self.expect(&Token::Colon, "':'")?;
            let if_false = self.expression()?;
            Expr::Cond(Box::new(condition), Box::new(if_true), Box::new(if_false))
        } else {
            condition
        };
        self.leave();
        Ok(result)
    }

    fn or(&mut self) -> Result<Expr, ExprError> {
        let mut left = self.and()?;
        while self.eat(&Token::OrOr) {
            let right = self.and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, ExprError> {
        let mut left = self.relation()?;
        while self.eat(&Token::AndAnd) {
            let right = self.relation()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn relation(&mut self) -> Result<Expr, ExprError> {
        let mut left = self.additive()?;
        loop {
            let op = match self.peek() {
                Token::EqEq => BinOp::Eq,
                Token::Ne => BinOp::Ne,
                Token::Lt => BinOp::Lt,
                Token::Le => BinOp::Le,
                Token::Gt => BinOp::Gt,
                Token::Ge => BinOp::Ge,
                Token::In => BinOp::In,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.additive()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn additive(&mut self) -> Result<Expr, ExprError> {
        let mut left = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.multiplicative()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn multiplicative(&mut self) -> Result<Expr, ExprError> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                Token::Percent => BinOp::Rem,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn unary(&mut self) -> Result<Expr, ExprError> {
        match self.peek() {
            Token::Bang => {
                self.bump();
                self.enter()?;
                let inner = self.unary()?;
                self.leave();
                Ok(Expr::Not(Box::new(inner)))
            }
            Token::Minus => {
                self.bump();
                self.enter()?;
                let inner = self.unary()?;
                self.leave();
                Ok(Expr::Neg(Box::new(inner)))
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Result<Expr, ExprError> {
        let mut expr = self.primary()?;
        loop {
            if self.eat(&Token::Dot) {
                let Token::Ident(name) = self.bump() else {
                    return Err(self.error("a field or method name after '.'"));
                };
                if self.eat(&Token::LParen) {
                    let args = self.arguments()?;
                    let method = method_named(&name)?;
                    expr = Expr::Method(Box::new(expr), method, args);
                } else {
                    expr = Expr::Field(Box::new(expr), name);
                }
            } else if self.eat(&Token::LBracket) {
                let index = self.expression()?;
                self.expect(&Token::RBracket, "']'")?;
                expr = Expr::Index(Box::new(expr), Box::new(index));
            } else {
                return Ok(expr);
            }
        }
    }

    /// Parses `args ')'` (the '(' is already consumed).
    fn arguments(&mut self) -> Result<Vec<Expr>, ExprError> {
        let mut args = Vec::new();
        if self.eat(&Token::RParen) {
            return Ok(args);
        }
        loop {
            args.push(self.expression()?);
            if args.len() > 8 {
                return Err(ExprError::TooComplex);
            }
            if self.eat(&Token::RParen) {
                return Ok(args);
            }
            self.expect(&Token::Comma, "',' or ')'")?;
        }
    }

    fn primary(&mut self) -> Result<Expr, ExprError> {
        let position = self.position();
        match self.bump() {
            Token::Int(n) => Ok(Expr::Literal(Value::from(n))),
            Token::Float(f) => serde_json::Number::from_f64(f)
                .map(|n| Expr::Literal(Value::Number(n)))
                .ok_or(ExprError::InvalidNumber(position)),
            Token::Str(s) => Ok(Expr::Literal(Value::String(s))),
            Token::True => Ok(Expr::Literal(Value::Bool(true))),
            Token::False => Ok(Expr::Literal(Value::Bool(false))),
            Token::Null => Ok(Expr::Literal(Value::Null)),
            Token::LParen => {
                self.enter()?;
                let inner = self.expression()?;
                self.leave();
                self.expect(&Token::RParen, "')'")?;
                Ok(inner)
            }
            Token::LBracket => {
                self.enter()?;
                let mut items = Vec::new();
                if !self.eat(&Token::RBracket) {
                    loop {
                        items.push(self.expression()?);
                        if items.len() > MAX_LIST_LITERAL {
                            return Err(ExprError::TooComplex);
                        }
                        if self.eat(&Token::RBracket) {
                            break;
                        }
                        self.expect(&Token::Comma, "',' or ']'")?;
                    }
                }
                self.leave();
                Ok(Expr::List(items))
            }
            Token::Ident(name) => {
                if self.eat(&Token::LParen) {
                    self.enter()?;
                    let args = self.arguments()?;
                    self.leave();
                    function(&name, args, position)
                } else {
                    Ok(Expr::Var(name))
                }
            }
            _ => {
                self.at = self.at.saturating_sub(1);
                Err(self.error("a value"))
            }
        }
    }
}

fn function(name: &str, mut args: Vec<Expr>, position: usize) -> Result<Expr, ExprError> {
    let one = |args: &mut Vec<Expr>| -> Result<Expr, ExprError> {
        if args.len() == 1 {
            Ok(args.remove(0))
        } else {
            Err(ExprError::Syntax {
                position,
                expected: "exactly one argument",
            })
        }
    };
    match name {
        "has" => match one(&mut args)? {
            Expr::Field(base, field) => Ok(Expr::Has(base, field)),
            _ => Err(ExprError::Syntax {
                position,
                expected: "a field selection such as has(input.name)",
            }),
        },
        "size" => Ok(Expr::Call(Function::Size, vec![one(&mut args)?])),
        "string" => Ok(Expr::Call(Function::String, vec![one(&mut args)?])),
        "int" => Ok(Expr::Call(Function::Int, vec![one(&mut args)?])),
        "double" => Ok(Expr::Call(Function::Double, vec![one(&mut args)?])),
        other => Err(ExprError::Unsupported(clip(other))),
    }
}

fn method_named(name: &str) -> Result<Method, ExprError> {
    match name {
        "size" => Ok(Method::Size),
        "contains" => Ok(Method::Contains),
        "startsWith" => Ok(Method::StartsWith),
        "endsWith" => Ok(Method::EndsWith),
        other => Err(ExprError::Unsupported(clip(other))),
    }
}
