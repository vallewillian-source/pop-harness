use std::fmt;

pub type Secret = Sensitive<String>;

#[derive(Clone, PartialEq, Eq)]
pub struct Sensitive<T>(T);

impl<T> Sensitive<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Sensitive<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[oculto]")
    }
}

impl<T> fmt::Display for Sensitive<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[oculto]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_hide_the_value() {
        let s = Secret::new("valor-muito-secreto-123".to_owned());
        assert_eq!(format!("{s:?}"), "[oculto]");
        assert_eq!(format!("{s}"), "[oculto]");
        assert!(!format!("{s:?} {s} {s:#?}").contains("muito-secreto"));
    }

    #[test]
    fn expose_returns_the_original_value() {
        let s = Secret::new("abc".to_owned());
        assert_eq!(s.expose(), "abc");
    }
}
