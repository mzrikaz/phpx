//! Operators and conversions with PHP 8 semantics.


use phpx_syntax::ast::BinOp;

use crate::interp::{Interp, R};
use crate::value::{compare, fmt_float, identical, loose_eq, parse_numeric, parse_numeric_prefix, Key, Num, PStr, PhpArray, Value};

impl Interp {
    /// String conversion: `__toString` for objects, warning for arrays.
    pub fn to_str(&self, v: &Value) -> R<PStr> {
        if let Some(b) = v.scalar_bytes() {
            return Ok(b);
        }
        match v {
            Value::Array(_) => {
                self.warn("Array to string conversion");
                Ok(crate::value::pbytes(b"Array"))
            }
            Value::Object(o) => {
                if o.class.find_method("__tostring").is_some() {
                    match self.call_method(o, "__toString", vec![])? {
                        Value::Str(s) => Ok(s),
                        _ => Err(self.error("Error", &format!("{}::__toString(): Return value must be of type string", o.class.name))),
                    }
                } else {
                    Err(self.error("Error", &format!("Object of class {} could not be converted to string", o.class.name)))
                }
            }
            _ => unreachable!(),
        }
    }

    pub fn to_string_lossy(&self, v: &Value) -> R<String> {
        Ok(String::from_utf8_lossy(&self.to_str(v)?).into_owned())
    }

    pub fn to_key(&self, v: &Value) -> R<Key> {
        Ok(match v {
            Value::Int(n) => Key::Int(*n),
            Value::Str(s) => Key::from_pstr(s),
            Value::Bool(b) => Key::Int(*b as i64),
            Value::Null => Key::Str(crate::value::pbytes(b"")),
            Value::Float(f) => {
                if f.fract() != 0.0 && f.is_finite() {
                    self.deprecated(&format!("Implicit conversion from float {} to int loses precision", fmt_float(*f)));
                }
                Key::Int(float_to_int(*f))
            }
            other => return Err(self.error("TypeError", &format!("Cannot access offset of type {} on array", other.debug_type()))),
        })
    }

    /// Operand for arithmetic. Numeric strings convert like PHP; leading-
    /// numeric strings ("5 apples") warn; anything else is a TypeError.
    fn to_num(&self, v: &Value, op: &str, other: &Value, left: bool) -> R<Num> {
        Ok(match v {
            Value::Ref(s) => return self.to_num(&s.borrow().clone(), op, other, left),
            Value::Int(n) => Num::Int(*n),
            Value::Float(f) => Num::Float(*f),
            Value::Null => Num::Int(0),
            Value::Bool(b) => Num::Int(*b as i64),
            Value::Str(s) => match parse_numeric(s) {
                Some(n) => n,
                None => match parse_numeric_prefix(s) {
                    Some((n, _)) => {
                        self.warn("A non-numeric value encountered");
                        n
                    }
                    None => {
                        return Err(self.error(
                            "TypeError",
                            &format!("Unsupported operand types: {}", op_types(v, op, other, left)),
                        ))
                    }
                },
            },
            Value::Resource(r) => Num::Int(r.id),
            Value::Array(_) | Value::Object(_) => {
                return Err(self.error("TypeError", &format!("Unsupported operand types: {}", op_types(v, op, other, left))))
            }
        })
    }

    fn to_int_operand(&self, v: &Value, op: &str, other: &Value, left: bool) -> R<i64> {
        Ok(match self.to_num(v, op, other, left)? {
            Num::Int(n) => n,
            Num::Float(f) => {
                if f.fract() != 0.0 && f.is_finite() {
                    self.deprecated(&format!("Implicit conversion from float {} to int loses precision", fmt_float(f)));
                }
                float_to_int(f)
            }
        })
    }

    #[inline(never)]
    pub fn binop(&self, op: BinOp, a: &Value, b: &Value) -> R<Value> {
        use BinOp::*;
        Ok(match op {
            Concat => {
                let mut s = self.to_str(a)?.to_vec();
                s.extend_from_slice(&self.to_str(b)?);
                Value::bytes(s)
            }
            Add if matches!((a, b), (Value::Array(_), Value::Array(_))) => {
                // Union: left wins on duplicate keys.
                let (Value::Array(x), Value::Array(y)) = (a, b) else { unreachable!() };
                let mut r = (**x).clone();
                for (k, v) in y.iter() {
                    if r.get(&k).is_none() {
                        r.insert(k.clone(), v.clone());
                    }
                }
                Value::array(r)
            }
            Add | Sub | Mul => {
                let sym = match op {
                    Add => "+",
                    Sub => "-",
                    _ => "*",
                };
                let x = self.to_num(a, sym, b, true)?;
                let y = self.to_num(b, sym, a, false)?;
                match (x, y) {
                    (Num::Int(p), Num::Int(q)) => {
                        let r = match op {
                            Add => p.checked_add(q),
                            Sub => p.checked_sub(q),
                            _ => p.checked_mul(q),
                        };
                        match r {
                            Some(n) => Value::Int(n),
                            None => Value::Float(float_op(op, p as f64, q as f64)),
                        }
                    }
                    _ => Value::Float(float_op(op, x.as_f64(), y.as_f64())),
                }
            }
            Div => {
                let x = self.to_num(a, "/", b, true)?;
                let y = self.to_num(b, "/", a, false)?;
                if y.as_f64() == 0.0 {
                    return Err(self.error("DivisionByZeroError", "Division by zero"));
                }
                match (x, y) {
                    (Num::Int(p), Num::Int(q)) if q != 0 && p.checked_rem(q) == Some(0) => match p.checked_div(q) {
                        Some(n) => Value::Int(n),
                        None => Value::Float(p as f64 / q as f64),
                    },
                    _ => Value::Float(x.as_f64() / y.as_f64()),
                }
            }
            Mod => {
                let x = self.to_int_operand(a, "%", b, true)?;
                let y = self.to_int_operand(b, "%", a, false)?;
                if y == 0 {
                    return Err(self.error("DivisionByZeroError", "Modulo by zero"));
                }
                Value::Int(x.wrapping_rem(y))
            }
            Pow => {
                let x = self.to_num(a, "**", b, true)?;
                let y = self.to_num(b, "**", a, false)?;
                match (x, y) {
                    (Num::Int(p), Num::Int(q)) if q >= 0 => match u32::try_from(q).ok().and_then(|q| p.checked_pow(q)) {
                        Some(n) => Value::Int(n),
                        None => Value::Float((p as f64).powf(q as f64)),
                    },
                    _ => Value::Float(x.as_f64().powf(y.as_f64())),
                }
            }
            BitAnd | BitOr | BitXor => {
                if let (Value::Str(x), Value::Str(y)) = (a, b) {
                    // Byte-wise string ops: & and ^ truncate, | pads.
                    let r: Vec<u8> = match op {
                        BitOr => {
                            let (long, short) = if x.len() >= y.len() { (x, y) } else { (y, x) };
                            long.iter().enumerate().map(|(i, c)| c | short.get(i).copied().unwrap_or(0)).collect()
                        }
                        BitAnd => x.iter().zip(y.iter()).map(|(p, q)| p & q).collect(),
                        _ => x.iter().zip(y.iter()).map(|(p, q)| p ^ q).collect(),
                    };
                    return Ok(Value::bytes(r));
                }
                let sym = match op {
                    BitAnd => "&",
                    BitOr => "|",
                    _ => "^",
                };
                let x = self.to_int_operand(a, sym, b, true)?;
                let y = self.to_int_operand(b, sym, a, false)?;
                Value::Int(match op {
                    BitAnd => x & y,
                    BitOr => x | y,
                    _ => x ^ y,
                })
            }
            Shl | Shr => {
                let sym = if op == Shl { "<<" } else { ">>" };
                let x = self.to_int_operand(a, sym, b, true)?;
                let y = self.to_int_operand(b, sym, a, false)?;
                if y < 0 {
                    return Err(self.error("ArithmeticError", "Bit shift by negative number"));
                }
                Value::Int(match (op, y >= 64) {
                    (Shl, true) => 0,
                    (Shl, false) => x.wrapping_shl(y as u32),
                    (_, true) => if x < 0 { -1 } else { 0 },
                    (_, false) => x >> y,
                })
            }
            Eq => Value::Bool(loose_eq(a, b)),
            NotEq => Value::Bool(!loose_eq(a, b)),
            Identical => Value::Bool(identical(a, b)),
            NotIdentical => Value::Bool(!identical(a, b)),
            Lt => Value::Bool(compare(a, b) == Some(std::cmp::Ordering::Less)),
            LtEq => Value::Bool(matches!(compare(a, b), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal))),
            Gt => Value::Bool(compare(a, b) == Some(std::cmp::Ordering::Greater)),
            GtEq => Value::Bool(matches!(compare(a, b), Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal))),
            Spaceship => Value::Int(match compare(a, b) {
                Some(std::cmp::Ordering::Less) => -1,
                Some(std::cmp::Ordering::Equal) => 0,
                _ => 1,
            }),
            Xor => Value::Bool(a.truthy() ^ b.truthy()),
            And | Or | Coalesce => unreachable!("short-circuit ops are handled in eval"),
        })
    }

    pub fn bitnot(&self, v: &Value) -> R<Value> {
        Ok(match v {
            Value::Int(n) => Value::Int(!n),
            Value::Float(f) => Value::Int(!float_to_int(*f)),
            Value::Str(s) => Value::bytes(s.iter().map(|c| !c).collect()),
            other => return Err(self.error("TypeError", &format!("Cannot perform bitwise not on {}", other.type_name()))),
        })
    }

    pub fn incdec(&self, v: &Value, inc: bool) -> R<Value> {
        Ok(match v {
            Value::Ref(s) => return self.incdec(&s.borrow().clone(), inc),
            Value::Null => if inc { Value::Int(1) } else { Value::Null },
            Value::Int(n) => match if inc { n.checked_add(1) } else { n.checked_sub(1) } {
                Some(m) => Value::Int(m),
                None => Value::Float(*n as f64 + if inc { 1.0 } else { -1.0 }),
            },
            Value::Float(f) => Value::Float(f + if inc { 1.0 } else { -1.0 }),
            Value::Bool(_) => v.clone(),
            Value::Str(s) if s.is_empty() => if inc { Value::str("1") } else { Value::Int(-1) },
            Value::Str(s) => match parse_numeric(s) {
                Some(n) => self.incdec(&n.to_value(), inc)?,
                None if inc => Value::bytes(str_increment(s)),
                None => v.clone(),
            },
            Value::Array(_) | Value::Object(_) | Value::Resource(_) => {
                return Err(self.error(
                    "TypeError",
                    &format!("Cannot {} {}", if inc { "increment" } else { "decrement" }, v.debug_type()),
                ))
            }
        })
    }

    pub fn to_int(&self, v: &Value) -> R<i64> {
        Ok(match v {
            Value::Ref(s) => return self.to_int(&s.borrow().clone()),
            Value::Null => 0,
            Value::Bool(b) => *b as i64,
            Value::Int(n) => *n,
            Value::Float(f) => float_to_int(*f),
            Value::Str(s) => match parse_numeric_prefix(s) {
                Some((Num::Int(n), _)) => n,
                Some((Num::Float(f), _)) => float_to_int(f),
                None => 0,
            },
            Value::Array(a) => !a.is_empty() as i64,
            Value::Resource(r) => r.id,
            Value::Object(o) => {
                self.warn(&format!("Object of class {} could not be converted to int", o.class.name));
                1
            }
        })
    }

    pub fn to_float(&self, v: &Value) -> R<f64> {
        Ok(match v {
            Value::Float(f) => *f,
            Value::Str(s) => match parse_numeric_prefix(s) {
                Some((n, _)) => n.as_f64(),
                None => 0.0,
            },
            other => self.to_int(other)? as f64,
        })
    }

    pub fn cast(&self, ty: &str, v: Value) -> R<Value> {
        Ok(match ty {
            "int" => Value::Int(self.to_int(&v)?),
            "float" => Value::Float(self.to_float(&v)?),
            "string" => Value::Str(self.to_str(&v)?),
            "bool" => Value::Bool(v.truthy()),
            "array" => match v {
                Value::Array(_) => v,
                Value::Null => Value::array(PhpArray::new()),
                Value::Object(o) if o.class.is_a("closure") => Value::array(PhpArray::from_list([Value::Object(o)])),
                // Built-in classes whose properties are their storage
                // (ArrayObject, ArrayIterator).
                Value::Object(o) if o.class.find_method("__phpx_cast").is_some() => self.call_method(&o, "__phpx_cast", vec![])?,
                Value::Object(o) => {
                    let mut a = PhpArray::new();
                    // (array) cast: private/protected keys are mangled.
                    for (k, val) in o.props.borrow().iter() {
                        a.insert(Key::from_bytes(&crate::ext::mangled(&o, k)), val.clone());
                    }
                    Value::array(a)
                }
                scalar => Value::array(PhpArray::from_list([scalar])),
            },
            "object" => match v {
                Value::Object(_) => v,
                other => {
                    let cls = self.classes.borrow().get("stdclass").cloned().unwrap();
                    let o = crate::value::Obj::new(cls, crate::value::Internal::None);
                    match other {
                        Value::Array(a) => {
                            for (k, val) in a.iter() {
                                let name = match k {
                                    Key::Int(n) => n.to_string(),
                                    Key::Str(s) => String::from_utf8_lossy(&s).into_owned(),
                                };
                                o.set_prop(&name, val.clone());
                            }
                        }
                        Value::Null => {}
                        scalar => o.set_prop("scalar", scalar),
                    }
                    Value::Object(o)
                }
            },
            _ => Value::Null,
        })
    }
}

fn float_op(op: BinOp, x: f64, y: f64) -> f64 {
    match op {
        BinOp::Add => x + y,
        BinOp::Sub => x - y,
        _ => x * y,
    }
}

/// PHP float to int: NaN/Inf become 0, out-of-range wraps modulo 2^64.
pub fn float_to_int(f: f64) -> i64 {
    if !f.is_finite() {
        return 0;
    }
    if f >= -9.223372036854775808e18 && f < 9.223372036854775808e18 {
        return f as i64;
    }
    let m = f.trunc() % 18446744073709551616.0;
    let m = if m < 0.0 { m + 18446744073709551616.0 } else { m };
    (m as u64) as i64
}

fn op_types(v: &Value, op: &str, other: &Value, left: bool) -> String {
    if left {
        format!("{} {op} {}", v.type_name(), other.type_name())
    } else {
        format!("{} {op} {}", other.type_name(), v.type_name())
    }
}

/// Perl-style string increment: "a" -> "b", "Az" -> "Ba", "zz" -> "aaa".
fn str_increment(s: &[u8]) -> Vec<u8> {
    let mut b = s.to_vec();
    let mut i = b.len();
    loop {
        if i == 0 {
            // Carried past the start: prepend based on the first char's kind.
            let first = match s[0] {
                b'0'..=b'9' => b'1',
                b'A'..=b'Z' => b'A',
                _ => b'a',
            };
            b.insert(0, first);
            return b;
        }
        i -= 1;
        match b[i] {
            b'z' => b[i] = b'a',
            b'Z' => b[i] = b'A',
            b'9' => b[i] = b'0',
            c if c.is_ascii_alphanumeric() => {
                b[i] = c + 1;
                return b;
            }
            _ => return b,
        }
    }
}
