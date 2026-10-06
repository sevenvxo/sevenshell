//! a small calculator for the launcher w + - * / ^ % brackets and a few functions like sqrt

/// the answer to text or none if its not math
pub fn eval(text: &str) -> Option<f64> {
    let mut p = Parser { chars: text.chars().filter(|c| !c.is_whitespace()).collect(), at: 0, depth: 0 };
    let value = p.sum()?;
    (p.at == p.chars.len() && value.is_finite()).then_some(value)
}

/// whether text looks like math someone typed and not an app name
pub fn looks_like_math(text: &str) -> bool {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix('=') {
        return !rest.trim().is_empty();
    }
    t.chars().any(|c| c.is_ascii_digit())
        && t.chars().any(|c| "+-*/^%(".contains(c))
        && t.chars().all(|c| c.is_ascii_digit() || c.is_ascii_alphabetic() || " .+-*/^%()".contains(c))
}

/// the answer w up to 10 decimals and no trailing zeros
pub fn show(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let text = format!("{value:.10}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// how deep brackets signs and powers can nest so a huge paste cant overflow the stack
const MAX_DEPTH: usize = 200;

struct Parser {
    chars: Vec<char>,
    at: usize,
    /// how many unarys deep we are rn
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn sum(&mut self) -> Option<f64> {
        let mut value = self.product()?;
        loop {
            if self.eat('+') {
                value += self.product()?;
            } else if self.eat('-') {
                value -= self.product()?;
            } else {
                return Some(value);
            }
        }
    }

    fn product(&mut self) -> Option<f64> {
        let mut value = self.power()?;
        loop {
            if self.eat('*') || self.eat('x') {
                value *= self.power()?;
            } else if self.eat('/') {
                value /= self.power()?;
            } else if self.eat('%') {
                value %= self.power()?;
            } else {
                return Some(value);
            }
        }
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        // right to left so 2^3^2 is 2^9
        if self.eat('^') {
            return Some(base.powf(self.nested(Self::power)?));
        }
        Some(base)
    }

    /// run f one level deeper or give up past MAX_DEPTH
    fn nested(&mut self, f: impl FnOnce(&mut Self) -> Option<f64>) -> Option<f64> {
        if self.depth >= MAX_DEPTH {
            return None;
        }
        self.depth += 1;
        let value = f(self);
        self.depth -= 1;
        value
    }

    /// signs brackets and function args all nest thru here
    fn unary(&mut self) -> Option<f64> {
        self.nested(Self::unary_inner)
    }

    fn unary_inner(&mut self) -> Option<f64> {
        if self.eat('-') {
            return Some(-self.unary()?);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.atom()
    }

    fn atom(&mut self) -> Option<f64> {
        if self.eat('(') {
            let value = self.sum()?;
            return self.eat(')').then_some(value);
        }
        let start = self.at;
        if self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.at += 1;
            }
            let name: String = self.chars[start..self.at].iter().collect::<String>().to_lowercase();
            match name.as_str() {
                "pi" => return Some(std::f64::consts::PI),
                "e" => return Some(std::f64::consts::E),
                _ => {}
            }
            let arg = self.atom()?;
            return Some(match name.as_str() {
                "sqrt" => arg.sqrt(),
                "sin" => arg.sin(),
                "cos" => arg.cos(),
                "tan" => arg.tan(),
                "ln" => arg.ln(),
                "log" => arg.log10(),
                "abs" => arg.abs(),
                "round" => arg.round(),
                "floor" => arg.floor(),
                "ceil" => arg.ceil(),
                _ => return None,
            });
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
            self.at += 1;
        }
        self.chars[start..self.at].iter().collect::<String>().parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_works() {
        assert_eq!(eval("2*8"), Some(16.0));
        assert_eq!(eval("2 + 3 * 4"), Some(14.0));
        assert_eq!(eval("(2+3)*4"), Some(20.0));
        assert_eq!(eval("2^3^2"), Some(512.0));
        assert_eq!(eval("-3+10%4"), Some(-1.0));
        assert_eq!(eval("sqrt(16)"), Some(4.0));
        assert_eq!(eval("2*"), None);
        assert_eq!(eval("firefox"), None);
        assert_eq!(show(0.1 + 0.2), "0.3");
        assert_eq!(show(16.0), "16");
    }

    #[test]
    fn deep_nesting_is_not_a_crash() {
        assert_eq!(eval(&format!("{}1{}", "(".repeat(100), ")".repeat(100))), Some(1.0));
        assert_eq!(eval(&format!("{}1", "(".repeat(100_000))), None);
        assert_eq!(eval(&format!("{}1", "-".repeat(100_000))), None);
        assert_eq!(eval(&format!("2{}", "^2".repeat(100_000))), None);
    }

    #[test]
    fn only_math_looks_like_math() {
        assert!(looks_like_math("2*8"));
        assert!(looks_like_math("=pi"));
        assert!(!looks_like_math("firefox"));
        assert!(!looks_like_math("2048"));
        // a dash in a name may look like math but it doesnt add up so no answer shows
        assert_eq!(eval("steam-2"), None);
    }
}
