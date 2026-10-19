#![allow(dead_code)]

#[derive(Clone, Copy)]
pub enum Preprocess {
  Trim,
  Lowercase,
  Uppercase,
}

#[derive(Clone, Copy)]
pub enum Constraint {
  MinLen(usize),
  MaxLen(usize),
  ExactLen(usize),
  MinBytes(usize),
  MaxBytes(usize),
  MinDigits(usize),
  MaxDigits(usize),
  NoControlChars,
  AsciiDigitsOnly,
  Regex { source: &'static str, compiled: fn() -> &'static regex::Regex, hint: Option<&'static str> },
  RequireUppercase,
  RequireLowercase,
  RequireDigit,
  RequireSymbol,
  Min(i64),
  Max(i64),
}

pub struct ValidationSpec {
  pub preprocess: &'static [Preprocess],
  pub constraints: &'static [Constraint],
}

#[derive(Clone, Copy)]
pub enum Value<'a> {
  Str(&'a str),
  Int(i64),
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Violation {
  TooShort { min: usize },
  TooLong { max: usize },
  ExactLength { expected: usize, actual: usize },
  TooFewBytes { min: usize },
  TooManyBytes { max: usize },
  TooFewDigits { min: usize },
  TooManyDigits { max: usize },
  TooFewItems { min: usize },
  TooManyItems { max: usize },
  InvalidFormat { hint: Option<&'static str> },
  InvalidChars,
  InvalidCharAt { position: usize, character: char },
  NotDigits,
  NotANumber,
  NotABoolean,
  TooSmall { min: i64 },
  TooLarge { max: i64 },
  MissingUppercase,
  MissingLowercase,
  MissingDigit,
  MissingSymbol,
}

impl Violation {
  pub fn message(&self) -> String {
    match self {
      Self::TooShort { min } => format!("muito curto (mínimo {min} caracteres)"),
      Self::TooLong { max } => format!("muito longo (máximo {max} caracteres)"),
      Self::ExactLength { expected, actual } => format!("tamanho inválido (esperado {expected}, recebido {actual})"),
      Self::TooFewBytes { min } => format!("muito curto (mínimo {min} bytes)"),
      Self::TooManyBytes { max } => format!("muito longo (máximo {max} bytes)"),
      Self::TooFewDigits { min } => format!("muito curto (mínimo {min} dígitos)"),
      Self::TooManyDigits { max } => format!("muito longo (máximo {max} dígitos)"),
      Self::TooFewItems { min } => format!("poucos itens (mínimo {min})"),
      Self::TooManyItems { max } => format!("muitos itens (máximo {max})"),
      Self::InvalidFormat { hint: None } => "formato inválido".to_owned(),
      Self::InvalidFormat { hint: Some(h) } => format!("formato inválido ({h})"),
      Self::InvalidChars => "caracteres inválidos".to_owned(),
      Self::InvalidCharAt { position, character } => format!("caractere inválido na posição {position} ('{character}')"),
      Self::NotDigits => "deve conter apenas dígitos".to_owned(),
      Self::NotANumber => "deve ser um número".to_owned(),
      Self::NotABoolean => "deve ser verdadeiro ou falso".to_owned(),
      Self::TooSmall { min } => format!("muito pequeno (mínimo {min})"),
      Self::TooLarge { max } => format!("muito grande (máximo {max})"),
      Self::MissingUppercase => "deve conter uma letra maiúscula".to_owned(),
      Self::MissingLowercase => "deve conter uma letra minúscula".to_owned(),
      Self::MissingDigit => "deve conter um dígito".to_owned(),
      Self::MissingSymbol => "deve conter um símbolo".to_owned(),
    }
  }
}

impl std::fmt::Display for Violation {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.message())
  }
}

impl std::error::Error for Violation {}

fn normalize<'a>(spec: &ValidationSpec, value: &'a str) -> std::borrow::Cow<'a, str> {
  spec.preprocess.iter().fold(std::borrow::Cow::Borrowed(value), |value, step| match step {
    Preprocess::Trim => match value {
      std::borrow::Cow::Borrowed(s) => std::borrow::Cow::Borrowed(s.trim()),
      std::borrow::Cow::Owned(s) => std::borrow::Cow::Owned(s.trim().to_owned()),
    },
    Preprocess::Lowercase => std::borrow::Cow::Owned(value.to_ascii_lowercase()),
    Preprocess::Uppercase => std::borrow::Cow::Owned(value.to_ascii_uppercase()),
  })
}

fn check_one(c: &Constraint, value: Value) -> Option<Violation> {
  let s = if let Value::Str(s) = value { s } else { "" };
  let has = |f: fn(char) -> bool| s.chars().any(f);
  match (c, value) {
    (Constraint::MinLen(min), Value::Str(s)) => (s.chars().count() < *min).then_some(Violation::TooShort { min: *min }),
    (Constraint::MaxLen(max), Value::Str(s)) => (s.chars().count() > *max).then_some(Violation::TooLong { max: *max }),
    (Constraint::ExactLen(len), Value::Str(s)) => {
      let actual = s.chars().count();
      (actual != *len).then_some(Violation::ExactLength { expected: *len, actual })
    }
    (Constraint::MinBytes(min), Value::Str(s)) => (s.len() < *min).then_some(Violation::TooFewBytes { min: *min }),
    (Constraint::MaxBytes(max), Value::Str(s)) => (s.len() > *max).then_some(Violation::TooManyBytes { max: *max }),
    (Constraint::MinDigits(min), Value::Str(s)) => (s.chars().filter(char::is_ascii_digit).count() < *min).then_some(Violation::TooFewDigits { min: *min }),
    (Constraint::MaxDigits(max), Value::Str(s)) => (s.chars().filter(char::is_ascii_digit).count() > *max).then_some(Violation::TooManyDigits { max: *max }),
    (Constraint::NoControlChars, Value::Str(s)) => s.chars().any(char::is_control).then_some(Violation::InvalidChars),
    (Constraint::AsciiDigitsOnly, Value::Str(s)) => (!s.chars().all(|c| c.is_ascii_digit())).then_some(Violation::NotDigits),
    (Constraint::Regex { compiled, hint, .. }, Value::Str(s)) => (!compiled().is_match(s)).then_some(Violation::InvalidFormat { hint: *hint }),
    (Constraint::RequireUppercase, Value::Str(_)) => (!has(|c| c.is_uppercase())).then_some(Violation::MissingUppercase),
    (Constraint::RequireLowercase, Value::Str(_)) => (!has(|c| c.is_lowercase())).then_some(Violation::MissingLowercase),
    (Constraint::RequireDigit, Value::Str(_)) => (!has(|c| c.is_ascii_digit())).then_some(Violation::MissingDigit),
    (Constraint::RequireSymbol, Value::Str(_)) => (!has(|c| !c.is_alphanumeric())).then_some(Violation::MissingSymbol),
    (Constraint::Min(min), Value::Int(n)) => (n < *min).then_some(Violation::TooSmall { min: *min }),
    (Constraint::Max(max), Value::Int(n)) => (n > *max).then_some(Violation::TooLarge { max: *max }),
    (Constraint::MinLen(_) | Constraint::MaxLen(_) | Constraint::ExactLen(_) | Constraint::MinBytes(_) | Constraint::MaxBytes(_) | Constraint::MinDigits(_) | Constraint::MaxDigits(_) | Constraint::NoControlChars | Constraint::AsciiDigitsOnly | Constraint::Regex { .. } | Constraint::RequireUppercase | Constraint::RequireLowercase | Constraint::RequireDigit | Constraint::RequireSymbol, Value::Int(_)) => None,
    (Constraint::Min(_) | Constraint::Max(_), Value::Str(_)) => None,
  }
}

pub fn check(spec: &ValidationSpec, value: Value) -> Option<Violation> {
  let owned;
  let value = match value {
    Value::Str(s) => {
      owned = normalize(spec, s);
      Value::Str(&owned)
    }
    v => v,
  };
  spec.constraints.iter().find_map(|c| check_one(c, value))
}

pub fn check_all(spec: &ValidationSpec, value: Value) -> Vec<Violation> {
  let owned;
  let value = match value {
    Value::Str(s) => {
      owned = normalize(spec, s);
      Value::Str(&owned)
    }
    v => v,
  };
  spec.constraints.iter().filter_map(|c| check_one(c, value)).collect()
}
