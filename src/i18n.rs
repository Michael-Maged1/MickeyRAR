//! Minimal runtime translations. UI strings are maintained in main.slint.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Language { Arabic, English }
impl Language {
    pub fn code(self) -> &'static str { match self { Self::Arabic => "ar", Self::English => "en" } }
    pub fn from_code(code: &str) -> Self { if code.trim().eq_ignore_ascii_case("ar") { Self::Arabic } else { Self::English } }
    pub fn is_arabic(self) -> bool { self == Self::Arabic }
    pub fn message(self, english: &str, arabic: &str) -> String {
        if self.is_arabic() { arabic.into() } else { english.into() }
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn languages_round_trip() {
        assert_eq!(Language::from_code("ar").code(), "ar");
        assert_eq!(Language::from_code("EN").code(), "en");
        assert_eq!(Language::from_code("??").code(), "en");
    }
    #[test] fn bilingual_strings() {
        assert_eq!(Language::Arabic.message("Ready", "جاهز"), "جاهز");
        assert_eq!(Language::English.message("Ready", "جاهز"), "Ready");
    }
}
