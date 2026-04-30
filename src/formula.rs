//! Formula parser & evaluator. Lifted from basecalc's tokenizer + shunting-yard but specialized for real-valued S43 evaluation: no complex literals, no variable store (just the bound `x`), no base switching, no assignment, no `&` previous-result. Char codes for operators and constants match basecalc's OPERATORS / CONSTANTS tables so dispatch tables can be cross-referenced.

use spirix::ScalarF4E3 as S43;

#[derive(Clone, Copy, Debug)]
pub enum RpnOp {
    Num(S43),
    Var,
    Const(char),
    Unary(char),
    Binary(char),
}

#[derive(Debug, Clone)]
pub enum ParseError {
    UnexpectedChar(usize, char),
    UnknownIdent(usize, String),
    BadNumber(usize),
    MismatchedParen(usize),
    UnexpectedEof,
    EmptyInput,
}

const UNARY_OPS: &[(&str, char)] = &[
    ("sqrt", 'q'),
    ("abs", 'a'),
    ("ln", 'l'),
    ("sin", 's'),
    ("cos", 'o'),
    ("tan", 't'),
    ("asin", 'S'),
    ("acos", 'O'),
    ("atan", 'T'),
    ("ceil", 'c'),
    ("floor", 'f'),
    ("round", 'r'),
    ("int", 'I'),
    ("frac", 'F'),
    ("sign", 'g'),
];

const CONSTS: &[(&str, char)] = &[
    ("pi", 'p'),
    ("phi", 'P'),
    ("e", 'E'),
    ("gamma", 'G'),
    ("catalan", 'C'),
];

#[derive(Clone, Copy, Debug)]
enum Tok {
    Num(S43),
    Var,
    Const(char),
    Unary(char),
    Binary(char),
    LParen,
    RParen,
}

fn tokenize(text: &str) -> Result<Vec<(usize, Tok)>, ParseError> {
    let mut out: Vec<(usize, Tok)> = Vec::new();
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    // True at start, after a binary op, after `(`, or after a unary function — i.e. when the next thing should be an operand. Drives unary/binary disambiguation for `+` and `-`.
    let mut expect_operand = true;

    while i < n {
        let pos = i;
        let c = bytes[i] as char;

        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < n {
                let d = bytes[i] as char;
                if d.is_ascii_digit() || d == '.' {
                    i += 1;
                } else {
                    break;
                }
            }
            let v: f64 = text[start..i].parse().map_err(|_| ParseError::BadNumber(start))?;
            out.push((pos, Tok::Num(S43::from(v))));
            expect_operand = false;
            continue;
        }

        if c == '#' {
            i += 1;
            let id_start = i;
            while i < n && (bytes[i] as char).is_ascii_alphabetic() {
                i += 1;
            }
            let id = &text[id_start..i];
            let op = UNARY_OPS
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(id))
                .ok_or_else(|| ParseError::UnknownIdent(pos, format!("#{}", id)))?;
            out.push((pos, Tok::Unary(op.1)));
            expect_operand = true;
            continue;
        }

        if c == '@' {
            i += 1;
            let id_start = i;
            while i < n && (bytes[i] as char).is_ascii_alphabetic() {
                i += 1;
            }
            let id = &text[id_start..i];
            let con = CONSTS
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(id))
                .ok_or_else(|| ParseError::UnknownIdent(pos, format!("@{}", id)))?;
            out.push((pos, Tok::Const(con.1)));
            expect_operand = false;
            continue;
        }

        if c == 'x' || c == 'X' {
            out.push((pos, Tok::Var));
            i += 1;
            expect_operand = false;
            continue;
        }

        match c {
            '(' => {
                out.push((pos, Tok::LParen));
                i += 1;
                expect_operand = true;
            }
            ')' => {
                out.push((pos, Tok::RParen));
                i += 1;
                expect_operand = false;
            }
            '+' => {
                if expect_operand {
                    // Unary plus: no-op, swallow.
                    i += 1;
                } else {
                    out.push((pos, Tok::Binary('+')));
                    i += 1;
                    expect_operand = true;
                }
            }
            '-' => {
                if expect_operand {
                    out.push((pos, Tok::Unary('n')));
                    i += 1;
                    // Still expecting operand for the negated value.
                } else {
                    out.push((pos, Tok::Binary('-')));
                    i += 1;
                    expect_operand = true;
                }
            }
            '*' | '/' | '%' | '^' | '$' => {
                out.push((pos, Tok::Binary(c)));
                i += 1;
                expect_operand = true;
            }
            _ => return Err(ParseError::UnexpectedChar(pos, c)),
        }
    }

    if out.is_empty() {
        return Err(ParseError::EmptyInput);
    }
    Ok(out)
}

fn precedence(op: char) -> u8 {
    match op {
        '+' | '-' => 1,
        '*' | '/' | '%' => 2,
        '^' | '$' => 3,
        _ => 4, // unary functions and 'n' (negate) bind tighter than any binary
    }
}

fn is_right_assoc(op: char) -> bool {
    op == '^'
}

fn to_rpn(toks: Vec<(usize, Tok)>) -> Result<Vec<RpnOp>, ParseError> {
    let mut out: Vec<RpnOp> = Vec::new();
    let mut stack: Vec<(usize, Tok)> = Vec::new();

    for (pos, tok) in toks {
        match tok {
            Tok::Num(v) => out.push(RpnOp::Num(v)),
            Tok::Var => out.push(RpnOp::Var),
            Tok::Const(c) => out.push(RpnOp::Const(c)),
            Tok::Unary(c) => stack.push((pos, Tok::Unary(c))),
            Tok::Binary(c) => {
                while let Some(&(_, top)) = stack.last() {
                    let pop = match top {
                        Tok::Unary(_) => true,
                        Tok::Binary(top_c) => {
                            let p_top = precedence(top_c);
                            let p_cur = precedence(c);
                            p_top > p_cur || (p_top == p_cur && !is_right_assoc(c))
                        }
                        _ => false,
                    };
                    if !pop {
                        break;
                    }
                    match stack.pop().unwrap().1 {
                        Tok::Unary(uc) => out.push(RpnOp::Unary(uc)),
                        Tok::Binary(bc) => out.push(RpnOp::Binary(bc)),
                        _ => unreachable!(),
                    }
                }
                stack.push((pos, Tok::Binary(c)));
            }
            Tok::LParen => stack.push((pos, Tok::LParen)),
            Tok::RParen => {
                let mut found = false;
                while let Some((_, top)) = stack.pop() {
                    match top {
                        Tok::LParen => {
                            found = true;
                            break;
                        }
                        Tok::Binary(c) => out.push(RpnOp::Binary(c)),
                        Tok::Unary(c) => out.push(RpnOp::Unary(c)),
                        _ => return Err(ParseError::MismatchedParen(pos)),
                    }
                }
                if !found {
                    return Err(ParseError::MismatchedParen(pos));
                }
            }
        }
    }

    while let Some((pos, tok)) = stack.pop() {
        match tok {
            Tok::Binary(c) => out.push(RpnOp::Binary(c)),
            Tok::Unary(c) => out.push(RpnOp::Unary(c)),
            Tok::LParen => return Err(ParseError::MismatchedParen(pos)),
            _ => return Err(ParseError::UnexpectedEof),
        }
    }

    Ok(out)
}

pub fn parse(text: &str) -> Result<Vec<RpnOp>, ParseError> {
    let toks = tokenize(text)?;
    to_rpn(toks)
}

pub fn evaluate(rpn: &[RpnOp], x: S43) -> S43 {
    // RPN well-formedness (validated at parse) prevents stack underflow; on the off chance one slips through (caller passing arbitrary slices), fall back to INFINITY so the pixel column shows as a yellow stripe instead of panicking.
    let mut stack: Vec<S43> = Vec::with_capacity(rpn.len());
    for op in rpn {
        match *op {
            RpnOp::Num(v) => stack.push(v),
            RpnOp::Var => stack.push(x),
            RpnOp::Const(c) => stack.push(eval_const(c)),
            RpnOp::Unary(c) => {
                let a = stack.pop().unwrap_or(S43::INFINITY);
                stack.push(apply_unary(c, a));
            }
            RpnOp::Binary(c) => {
                let b = stack.pop().unwrap_or(S43::INFINITY);
                let a = stack.pop().unwrap_or(S43::INFINITY);
                stack.push(apply_binary(c, a, b));
            }
        }
    }
    stack.pop().unwrap_or(S43::INFINITY)
}

fn eval_const(c: char) -> S43 {
    match c {
        'p' => S43::PI,
        'P' => S43::PHI,
        'E' => S43::E,
        'G' => S43::EULER_GAMMA,
        'C' => S43::CATALAN,
        _ => S43::INFINITY,
    }
}

fn apply_unary(c: char, a: S43) -> S43 {
    match c {
        'n' => S43::ZERO - a,
        's' => a.sin(),
        'o' => a.cos(),
        't' => a.tan(),
        'S' => a.asin(),
        'O' => a.acos(),
        'T' => a.atan(),
        'q' => a.sqrt(),
        'a' => a.magnitude(),
        'l' => a.ln(),
        'c' => a.ceil(),
        'f' => a.floor(),
        'r' => a.round(),
        'F' => a.frac(),
        'g' => a.sign(),
        // Truncation toward zero. Spirix's `frac` is floor-style (always non-negative for normal values), so `a - a.frac()` would be `floor(a)` — wrong for negatives. `sign(a) * |a|.floor()` gets true truncation.
        'I' => a.sign() * a.magnitude().floor(),
        _ => S43::INFINITY,
    }
}

fn apply_binary(c: char, a: S43, b: S43) -> S43 {
    match c {
        '+' => a + b,
        '-' => a - b,
        '*' => a * b,
        '/' => a / b,
        '^' => a.pow(b),
        '%' => a % b,
        '$' => a.log(b),
        _ => S43::INFINITY,
    }
}
