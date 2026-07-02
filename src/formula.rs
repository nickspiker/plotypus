//! Formula tokenizer + shunting-yard + evaluator. Lifted from basecalc's
//! `tokenize` / `evaluate_tokens` / `apply_unary_operator` / `apply_binary_operator` /
//! `parse_constant` / `parse_number` / `parse_operator` / `token2num` (basecalc/src/main.rs
//! lines 1271-2360 + 2904+), with these substitutions:
//!
//! - `Complex` → `S43` (real-only).
//! - `state.base` → fixed `BASE = 12` (dozenal).
//! - `state.radians` → always radians.
//! - `state.precision` → dropped (S43 has fixed precision).
//! - Dropped: assignment (`=`), `&` previous-result, `@rand`/`@grand`, complex literals
//!   `[a, b]`, base-switching commands, custom variables.
//! - Added: `x` as a built-in CONSTANTS entry mapping to the bound variable supplied
//!   to `evaluate(rpn, x)`.
//!
//! The function-name and control-flow shape is preserved so behaviour matches basecalc's.

use crate::plotnum::PlotNum;
use spirix::ScalarF4E3 as S43;

/// Default numeric base (dozenal — basecalc heritage). Overridden at runtime by the base
/// input box. The base is purely an input/label concern: it controls how a literal like
/// `2000` is converted into an S43 value (and how axis labels are rendered, since Spirix's
/// Display takes the base as the format precision). All Spirix math downstream is
/// base-agnostic.
pub const DEFAULT_BASE: u8 = 12;

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy)]
enum Precedence {
    // Lowest — bitwise `& | ~` bind looser than `+ -` (variant order defines the ordering).
    Logic,
    // Bit shifts `<< >>` bind looser than `+ -` (C convention: `a + b << c` = `(a+b) << c`).
    Shift,
    Addition,
    Multiplication,
    Exponentiation,
    Unary,
    Parenthesis,
}

#[derive(Clone, Debug)]
pub struct Token {
    operator: char,
    operands: u8,
    real_integer: Vec<u8>,
    real_fraction: Vec<u8>,
    sign: bool,
}

impl Token {
    fn new() -> Token {
        Token {
            operator: 0 as char,
            operands: 0,
            real_integer: Vec::new(),
            real_fraction: Vec::new(),
            sign: false,
        }
    }
}

/// Operator table — same shape as basecalc's, minus `=` (no assignment) and the complex-only
/// `#re`/`#im`/`#angle`/`#erf` entries (real-only port). Char codes match basecalc 1:1 so
/// `apply_unary_operator` / `apply_binary_operator` can dispatch off the same key.
static OPERATORS: &[(&str, char, u8, &str)] = &[
    // Basic arithmetic
    ("+", '+', 2, "addition"),
    ("-", '-', 2, "subtraction"),
    ("*", '*', 2, "multiplication"),
    ("/", '/', 2, "division"),
    ("^", '^', 2, "exponentiation"),
    ("%", '%', 2, "modulus"),
    ("$", '$', 2, "logarithm (number$base = log base of number)"),
    // Bit shifts — Spirix's power-of-two scale by an integer amount (RHS truncated to int).
    // `<<` = ×2ⁿ, `>>` = ÷2ⁿ. Two chars, so listed before the single-char ops.
    ("<<", '<', 2, "left shift (×2ⁿ)"),
    (">>", '>', 2, "right shift (÷2ⁿ)"),
    // Bitwise logic — Spirix's two's-complement ops aligned at the binary point. Binary
    // `& | ~` are lowest precedence (looser than + -), like C. `^` is taken by
    // exponentiation, so xor is `~`. `!` is unary prefix bitwise NOT.
    ("&", '&', 2, "bitwise and"),
    ("|", '|', 2, "bitwise or"),
    ("~", '~', 2, "bitwise xor"),
    ("!", '!', 1, "bitwise not"),
    // Parentheses
    ("(", '(', 1, "left parenthesis"),
    (")", ')', 1, "right parenthesis"),
    // Functions. ORDER MATTERS: parse_operator takes the first `starts_with` match, so any
    // name that is a prefix of another must come after it. `#sinh` before `#sin`, etc.
    // (Inverse hyperbolics asinh/acosh/atanh are Circle-only in Spirix — not real Scalars.)
    //
    // Multi-argument functions: comma-separated args, `fn(a, b[, c])`. The `operands` field
    // stays 1 (they parse as prefix functions); their true arity lives in `function_arity`.
    // `#atan2` before `#atan` (prefix), `#clamp` before nothing it collides with.
    ("#clamp", 'K', 1, "clamp(x, lo, hi)"),
    ("#atan2", 'A', 1, "two-arg arctangent atan2(y, x)"),
    ("#min", 'm', 1, "minimum min(a, b)"),
    ("#max", 'M', 1, "maximum max(a, b)"),
    ("#square", 'Q', 1, "x squared"),
    ("#recip", 'R', 1, "reciprocal (1/x)"),
    ("#round", 'r', 1, "gaussian rounding"),
    ("#floor", 'f', 1, "gaussian floor"),
    ("#sinh", 'h', 1, "hyperbolic sine"),
    ("#cosh", 'H', 1, "hyperbolic cosine"),
    ("#tanh", 'y', 1, "hyperbolic tangent"),
    ("#asin", 'S', 1, "inverse sine"),
    ("#acos", 'O', 1, "inverse cosine"),
    ("#atan", 'T', 1, "inverse tangent"),
    ("#sqrt", 'q', 1, "square root"),
    ("#sign", 'g', 1, "sign"),
    ("#ceil", 'c', 1, "gaussian ceiling"),
    ("#frac", 'F', 1, "fractional part"),
    ("#powb", 'b', 1, "2^x"),
    ("#exp", 'e', 1, "e^x"),
    ("#abs", 'a', 1, "absolute value"),
    ("#sin", 's', 1, "sine"),
    ("#cos", 'o', 1, "cosine"),
    ("#tan", 't', 1, "tangent"),
    ("#lb", 'L', 1, "binary logarithm (log2)"),
    ("#ln", 'l', 1, "natural logarithm"),
];

/// Constants table — basecalc minus `@rand`, `@grand`, `&` (state-dependent), plus `x` as
/// the input variable. Sorted longest-first so `parse_constant`'s `starts_with` lookup
/// resolves `@phi` before `@pi`, `@catalan` before `@e`, etc.
static CONSTANTS: &[(&str, char, &str)] = &[
    ("@catalan", 'C', "Catalan's constant"),
    ("@gamma", 'G', "Euler-Mascheroni constant"),
    ("@fgrand", 'Z', "fixed gaussian random (one draw per plot)"),
    ("@frand", 'W', "fixed uniform random -1..1 (one draw per plot)"),
    ("@grand", 'z', "gaussian random (fresh per sample)"),
    ("@rand", 'w', "uniform random -1..1 (fresh per sample)"),
    ("@phi", 'P', "Golden ratio"),
    ("@pi", 'p', "Pi"),
    ("@e", 'E', "Euler's number"),
    ("x", 'X', "input variable"),
];

#[derive(Debug, Clone)]
pub enum ParseError {
    Message(String, usize),
}

impl ParseError {
    fn msg(s: &str, pos: usize) -> Self {
        ParseError::Message(s.to_string(), pos)
    }
}

// ============================================================================
// tokenize — basecalc/src/main.rs:1466-1629, minus state-config branches
// ============================================================================

pub fn tokenize(input_str: &str, base: u8) -> Result<Vec<Token>, ParseError> {
    let input = input_str.as_bytes();
    let mut tokens: Vec<Token> = Vec::new();
    let mut index = 0;
    let mut paren_count: i32 = 0;
    let mut start = true;
    let mut expect_number = true;
    let mut follows_number = false;

    while index < input.len() {
        if input[index] == b' ' || input[index] == b'_' || input[index] == b'\t' {
            index += 1;
            continue;
        }
        if input[index] == b'(' {
            if !start && follows_number {
                return Err(ParseError::msg("Expected operator!", index));
            }
            tokens.push(Token {
                operator: '(',
                operands: 1,
                ..Token::new()
            });
            paren_count += 1;
            index += 1;
            start = false;
            continue;
        }
        if input[index] == b')' {
            if paren_count == 0 {
                return Err(ParseError::msg("Mismatched parentheses!", index));
            }
            if !follows_number {
                return Err(ParseError::msg("Expected number!", index));
            }
            tokens.push(Token {
                operator: ')',
                operands: 1,
                ..Token::new()
            });
            paren_count -= 1;
            index += 1;
            continue;
        }
        // Argument separator for multi-arg functions. Must follow a completed value; emits a
        // comma token that the shunting-yard flushes operators against (up to the '(').
        if input[index] == b',' {
            if !follows_number {
                return Err(ParseError::msg("Expected value before comma!", index));
            }
            tokens.push(Token {
                operator: ',',
                operands: 1,
                ..Token::new()
            });
            index += 1;
            start = false;
            expect_number = true;
            follows_number = false;
            continue;
        }
        if expect_number {
            if let Ok((token, new_index)) = parse_constant(input, index) {
                tokens.push(token);
                index = new_index;
                start = false;
                expect_number = false;
                follows_number = true;
                continue;
            }
            match parse_number(input, base, index) {
                Ok((token, new_index)) => {
                    tokens.push(token);
                    index = new_index;
                    start = false;
                    expect_number = false;
                    follows_number = true;
                    continue;
                }
                Err(e) => {
                    let (mut token, new_index) = parse_operator(input, index);
                    if token.operator == '\0' || token.operands == 2 {
                        if token.operator == '-' {
                            token.operator = 'n';
                            token.operands = 1;
                            tokens.push(token);
                            index = new_index;
                            continue;
                        } else {
                            return Err(e);
                        }
                    }
                    tokens.push(token);
                    index = new_index;
                    start = false;
                    expect_number = true;
                    continue;
                }
            }
        }
        let (token, new_index) = parse_operator(input, index);
        if token.operator == '\0' {
            return Err(ParseError::msg("Invalid operator!", new_index));
        }
        if token.operands == 1 && follows_number {
            return Err(ParseError::msg("Expected operator!", index));
        }
        tokens.push(token);
        index = new_index;
        expect_number = true;
        follows_number = false;
    }

    if paren_count != 0 {
        return Err(ParseError::msg("Mismatched parentheses!", input.len()));
    }
    if tokens.is_empty() {
        return Err(ParseError::msg("Empty expression", 0));
    }
    let last_token = tokens.last().unwrap();
    if last_token.operands > 0 && last_token.operator != ')' {
        return Err(ParseError::msg("Incomplete expression!", input.len()));
    }
    Ok(tokens)
}

/// Per-plot random constants: drawn ONCE when the plot is (re)committed and held constant
/// across the whole curve — unlike `@rand`/`@grand`, which redraw per sample. `uniform` backs
/// `@frand`, `gauss` backs `@fgrand`. Passed into `evaluate` so every sample sees the same
/// draw.
#[derive(Clone, Copy)]
pub struct FixedRand<T> {
    pub uniform: T,
    pub gauss: T,
}

// ============================================================================
// evaluate_tokens — basecalc:1642-1796, minus the assignment branch
// ============================================================================

pub fn evaluate<T: PlotNum>(
    tokens: &[Token],
    x: T,
    base: u8,
    fixed: FixedRand<T>,
) -> Result<T, String> {
    let mut output_queue: Vec<T> = Vec::new();
    let mut operator_stack: Vec<char> = Vec::new();

    for token in tokens {
        match token.operands {
            0 => {
                let mut value = token2num(token, x, base, fixed);
                while let Some(&op) = operator_stack.last() {
                    if get_precedence(op) == Precedence::Unary {
                        let operator = operator_stack.pop().unwrap();
                        value = apply_unary_operator(operator, value)?;
                    } else {
                        break;
                    }
                }
                output_queue.push(value);
            }
            1 => {
                if token.operator == '(' {
                    operator_stack.push('(');
                } else if token.operator == ')' {
                    while let Some(&op) = operator_stack.last() {
                        if op == '(' {
                            operator_stack.pop();
                            break;
                        }
                        apply_operator(&mut output_queue, operator_stack.pop().unwrap())?;
                    }
                    if let Some(&op) = operator_stack.last() {
                        if get_precedence(op) == Precedence::Unary {
                            apply_operator(&mut output_queue, operator_stack.pop().unwrap())?;
                        }
                    }
                } else if token.operator == ',' {
                    // Argument separator: apply the current argument's pending operators back
                    // to the enclosing '(' (left on the stack), leaving each finished arg as a
                    // single value on the output queue for the function to collect at its ')'.
                    while let Some(&op) = operator_stack.last() {
                        if op == '(' {
                            break;
                        }
                        apply_operator(&mut output_queue, operator_stack.pop().unwrap())?;
                    }
                } else {
                    operator_stack.push(token.operator);
                }
            }
            2 => {
                while let Some(&op) = operator_stack.last() {
                    if op == '(' || get_precedence(token.operator) > get_precedence(op) {
                        break;
                    }
                    apply_operator(&mut output_queue, operator_stack.pop().unwrap())?;
                }
                operator_stack.push(token.operator);
            }
            _ => return Err(format!("Invalid token: '{}'", token.operator)),
        }
    }

    while let Some(op) = operator_stack.pop() {
        if op == '(' {
            return Err("Mismatched parentheses".to_string());
        }
        apply_operator(&mut output_queue, op)?;
    }

    if output_queue.len() != 1 {
        return Err("Invalid expression".to_string());
    }
    Ok(output_queue.pop().unwrap())
}

// ============================================================================
// apply_operator / get_precedence — basecalc:1798-1830
// ============================================================================

fn apply_operator<T: PlotNum>(output_queue: &mut Vec<T>, op: char) -> Result<(), String> {
    match op {
        '+' | '-' | '*' | '/' | '^' | '%' | '$' | '&' | '|' | '~' | '<' | '>' => {
            apply_binary_operator(output_queue, op)?
        }
        // Two-argument functions. Args left-to-right on the queue, so pop b then a.
        'm' | 'M' | 'A' => {
            if let (Some(b), Some(a)) = (output_queue.pop(), output_queue.pop()) {
                let result = match op {
                    'm' => a.min(b),
                    'M' => a.max(b),
                    'A' => a.atan2(b), // atan2(y = a, x = b)
                    _ => unreachable!(),
                };
                output_queue.push(result);
            } else {
                return Err(format!("Not enough operands for {}", op));
            }
        }
        // Three-argument clamp(x, lo, hi): queue is [x, lo, hi], pop hi, lo, x.
        'K' => {
            if let (Some(hi), Some(lo), Some(x)) =
                (output_queue.pop(), output_queue.pop(), output_queue.pop())
            {
                output_queue.push(x.clamp(lo, hi));
            } else {
                return Err("Not enough operands for clamp".to_string());
            }
        }
        'n' | '!' | 'a' | 'O' | 'o' | 'S' | 'T' | 'c' | 'f' | 'F' | 'l' | 'r' | 'g' | 's' | 'q'
        | 't' | 'e' | 'b' | 'L' | 'Q' | 'R' | 'h' | 'H' | 'y' => {
            if let Some(value) = output_queue.pop() {
                let result = apply_unary_operator(op, value)?;
                output_queue.push(result);
            } else {
                return Err(format!("Not enough operands for {}", op));
            }
        }
        _ => return Err(format!("Unknown operator: {}", op)),
    }
    Ok(())
}

fn get_precedence(op: char) -> Precedence {
    match op {
        '&' | '|' | '~' => Precedence::Logic,
        '<' | '>' => Precedence::Shift,
        '+' | '-' => Precedence::Addition,
        '*' | '/' | '%' => Precedence::Multiplication,
        '^' | '$' => Precedence::Exponentiation,
        'n' | '!' | 'a' | 'O' | 'o' | 'S' | 'T' | 'c' | 'f' | 'F' | 'l' | 'r' | 'g' | 's' | 'q'
        | 't' | 'e' | 'b' | 'L' | 'Q' | 'R' | 'h' | 'H' | 'y' | 'm' | 'M' | 'K' | 'A' => {
            Precedence::Unary
        }
        '(' | ')' => Precedence::Parenthesis,
        _ => Precedence::Addition,
    }
}

// ============================================================================
// apply_unary_operator — basecalc:1831-1965, real-only with S43 ops
// ============================================================================

fn apply_unary_operator<T: PlotNum>(op: char, value: T) -> Result<T, String> {
    let result = match op {
        'n' => value.neg(),
        '!' => value.not(),
        'a' => value.abs(),
        'S' => value.asin(),
        'O' => value.acos(),
        'T' => value.atan(),
        'c' => value.ceil(),
        'f' => value.floor(),
        'F' => value.frac(),
        'l' => value.ln(),
        'r' => value.round(),
        'g' => value.sign(),
        'q' => value.sqrt(),
        's' => value.sin(),
        'o' => value.cos(),
        't' => value.tan(),
        'e' => value.exp(),
        'b' => value.powb(),
        'L' => value.lb(),
        'Q' => value.square(),
        'R' => value.recip(),
        'h' => value.sinh(),
        'H' => value.cosh(),
        'y' => value.tanh(),
        _ => return Err(format!("Unknown unary operator: {}", op)),
    };
    Ok(result)
}

// ============================================================================
// apply_binary_operator — basecalc:1980-2007, with S43 ops
// ============================================================================

fn apply_binary_operator<T: PlotNum>(output_queue: &mut Vec<T>, op: char) -> Result<(), String> {
    if let (Some(b), Some(a)) = (output_queue.pop(), output_queue.pop()) {
        let result = match op {
            '%' => a.rem(b),
            '^' => a.pow(b),
            '$' => a.log(b),
            '*' => a.mul(b),
            '+' => a.add(b),
            '-' => a.sub(b),
            '/' => a.div(b),
            // Spirix two's-complement bitwise, aligned at the binary point.
            '&' => a.bitand(b),
            '|' => a.bitor(b),
            '~' => a.bitxor(b),
            // Bit shifts by an integer amount (RHS truncated toward zero). ×2ⁿ / ÷2ⁿ.
            '<' => a.shl(b.to_f64() as i32),
            '>' => a.shr(b.to_f64() as i32),
            _ => return Err(format!("Unknown binary operator: {}", op)),
        };
        output_queue.push(result);
        Ok(())
    } else {
        Err(format!(
            "Not enough operands for {}!",
            OPERATORS
                .iter()
                .find(|&&(_, sym, _, _)| sym == op)
                .map(|(_, _, _, desc)| *desc)
                .unwrap_or("unknown operator")
        ))
    }
}

// ============================================================================
// parse_constant — basecalc:2045-2069 (the constants-only branch; @-variable
// machinery dropped)
// ============================================================================

fn parse_constant(input: &[u8], mut index: usize) -> Result<(Token, usize), ParseError> {
    while index < input.len() && (input[index] == b' ' || input[index] == b'_' || input[index] == b'\t') {
        index += 1;
    }
    for &(name, op, _desc) in CONSTANTS {
        if input[index..]
            .to_ascii_lowercase()
            .starts_with(name.as_bytes())
        {
            return Ok((
                Token {
                    operator: op,
                    ..Token::new()
                },
                index + name.len(),
            ));
        }
    }
    Err(ParseError::msg("not a constant", index))
}

// ============================================================================
// parse_number — basecalc:2159-2323, real-only (drop complex `[a, b]`)
// ============================================================================

fn parse_number(
    input: &[u8],
    base: u8,
    mut index: usize,
) -> Result<(Token, usize), ParseError> {
    let mut integer = true;
    let mut expect_sign = true;
    let mut token = Token {
        operator: 1 as char, // 1 denotes number
        ..Token::new()
    };
    while index < input.len()
        && (input[index] == b' ' || input[index] == b'_' || input[index] == b'\t')
    {
        index += 1;
    }
    if index >= input.len() {
        return Err(ParseError::msg("Incomplete expression!", index));
    }
    while index < input.len() {
        let c = input[index];

        if c == b' ' || c == b'_' || c == b'\t' {
            index += 1;
            continue;
        }

        if expect_sign && c == b'-' {
            token.sign = !token.sign;
            index += 1;
            continue;
        }

        if c == b'.' {
            if !integer {
                return Err(ParseError::msg("Multiple decimals in number!", index));
            }
            integer = false;
            index += 1;
            continue;
        }

        let digit = if c.is_ascii_digit() {
            c - b'0'
        } else if c.is_ascii_uppercase() {
            c - b'A' + 10
        } else if c.is_ascii_lowercase() {
            c - b'a' + 10
        } else {
            if token.real_integer.is_empty() && token.real_fraction.is_empty() {
                return Err(ParseError::msg("Invalid number!", index));
            }
            return Ok((token, index));
        };

        if digit >= base {
            // For non-digit characters that are also "out of base range" (like 'x' in
            // base 12), we fall back to "stop here" rather than erroring — but only if
            // we already collected at least one digit. Otherwise it's a real error.
            if token.real_integer.is_empty() && token.real_fraction.is_empty() {
                return Err(ParseError::msg("Invalid number!", index));
            }
            return Ok((token, index));
        }
        expect_sign = false;
        if integer {
            token.real_integer.push(digit);
        } else {
            token.real_fraction.push(digit);
        }
        index += 1;
    }

    if token.real_integer.is_empty() && token.real_fraction.is_empty() {
        return Err(ParseError::msg("Invalid number!", index));
    }
    Ok((token, index))
}

// ============================================================================
// parse_operator — basecalc:2334-2358
// ============================================================================

fn parse_operator(input: &[u8], mut index: usize) -> (Token, usize) {
    let mut token = Token::new();
    if index < input.len() {
        for &(op_str, op_char, operands, _) in OPERATORS {
            if input[index..]
                .to_ascii_lowercase()
                .starts_with(op_str.as_bytes())
            {
                token.operator = op_char;
                token.operands = operands;
                index += op_str.len();
                return (token, index);
            }
        }
    }
    (token, index)
}

// ============================================================================
// token2num — basecalc:2904-2966, real-only with S43 (constants come from
// Spirix; numeric literals accumulate digits in base BASE)
// ============================================================================

fn token2num<T: PlotNum>(token: &Token, x: T, base: u8, fixed: FixedRand<T>) -> T {
    match token.operator {
        // Built-in constants (chars match basecalc's CONSTANTS table)
        'E' => T::e(),
        'G' => T::gamma(),
        'C' => T::catalan(),
        'p' => T::pi(),
        'P' => T::phi(),
        'X' => x,
        // Per-sample random — a fresh draw each time the token is evaluated (per pixel-column
        // in the plot / per audio sample), so @rand / @grand render as a noise band.
        'w' => T::random(),
        'z' => T::random_gauss(),
        // Per-plot random — one draw held constant across the whole curve (@frand / @fgrand).
        'W' => fixed.uniform,
        'Z' => fixed.gauss,

        // Regular numeric literal — accumulate base digits in f64, then convert once to T.
        // f64 is ample for user-typed literals; the target precision T applies to the maths,
        // not the parse of the constant itself.
        _ => {
            let b = base as f64;
            let mut real = 0.0f64;
            for &digit in &token.real_integer {
                real = real * b + digit as f64;
            }
            let mut frac = 0.0f64;
            for &digit in token.real_fraction.iter().rev() {
                frac = (frac + digit as f64) / b;
            }
            real += frac;
            if token.sign {
                real = -real;
            }
            T::from_f64(real)
        }
    }
}

// ============================================================================
// Public entry: parse + evaluate in one shot
// ============================================================================

/// Parse the formula text in `base`, returning the token vector for caching. The same
/// vector is then fed to `evaluate(&tokens, x, base)` per pixel-x.
pub fn parse(text: &str, base: u8) -> Result<Vec<Token>, ParseError> {
    tokenize(text, base)
}

/// Parse a plain numeric string (e.g. "-1.5") in `base` to S43. Used by the range/bound
/// input boxes (not the formula box, which goes through `parse`). Returns `None` on any
/// invalid character for the base.
pub fn parse_value(text: &str, base: u8) -> Option<S43> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (negative, rest) = if let Some(t) = text.strip_prefix('-') {
        (true, t)
    } else if let Some(t) = text.strip_prefix('+') {
        (false, t)
    } else {
        (false, text)
    };
    let base_s = S43::from(base as u32);
    let mut parts = rest.splitn(2, '.');

    let int_str = parts.next().unwrap_or("");
    let mut int_part = S43::ZERO;
    for ch in int_str.chars() {
        let digit = char_to_digit(ch, base)?;
        int_part = int_part * base_s + S43::from(digit as u32);
    }

    let mut frac_part = S43::ZERO;
    if let Some(frac_str) = parts.next() {
        for ch in frac_str.chars().rev() {
            let digit = char_to_digit(ch, base)?;
            frac_part = frac_part + S43::from(digit as u32);
            frac_part = frac_part / base_s;
        }
    }

    let mut result = int_part + frac_part;
    if negative {
        result = S43::ZERO - result;
    }
    Some(result)
}

/// Map a digit character to its value, or `None` if it's out of range for `base`.
fn char_to_digit(ch: char, base: u8) -> Option<u8> {
    let digit = match ch {
        '0'..='9' => ch as u8 - b'0',
        'a'..='z' => ch as u8 - b'a' + 10,
        'A'..='Z' => ch as u8 - b'A' + 10,
        _ => return None,
    };
    if digit < base { Some(digit) } else { None }
}

/// Single-character label for a base value (2..=36): `2`..`9`, then `a`..`z`. So dozenal
/// (12) shows as `c`. Used by the base input box's display.
pub fn base_to_char(base: u8) -> char {
    if base <= 9 {
        (b'0' + base) as char
    } else {
        (b'a' + base - 10) as char
    }
}

/// Parse a single character into a base value (2..=36). Inverse of [`base_to_char`].
pub fn char_to_base(ch: char) -> Option<u8> {
    let base = match ch {
        '2'..='9' => ch as u8 - b'0',
        'a'..='z' => ch as u8 - b'a' + 10,
        'A'..='Z' => ch as u8 - b'A' + 10,
        _ => return None,
    };
    if base >= 2 { Some(base) } else { None }
}
