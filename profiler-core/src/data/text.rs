use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone)]
pub struct Text<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextError {
    TooLong { capacity: usize, length: usize },
    EmbeddedNul,
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong { capacity, length } => {
                write!(f, "text has {length} bytes, capacity is {capacity}")
            }
            Self::EmbeddedNul => f.write_str("text contains an embedded NUL"),
        }
    }
}

impl std::error::Error for TextError {}

impl<const N: usize> Text<N> {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len])
            .expect("Text copies complete UTF-8 strings through its checked constructor")
    }
}

impl<const N: usize> Default for Text<N> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
}

impl<const N: usize> TryFrom<&str> for Text<N> {
    type Error = TextError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.len() > N {
            return Err(TextError::TooLong {
                capacity: N,
                length: value.len(),
            });
        }
        if value.as_bytes().contains(&0) {
            return Err(TextError::EmbeddedNul);
        }
        let mut text = Self::default();
        text.bytes[..value.len()].copy_from_slice(value.as_bytes());
        text.len = value.len();
        Ok(text)
    }
}

impl<const N: usize> Deref for Text<N> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl<const N: usize> fmt::Display for Text<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl<const N: usize> fmt::Debug for Text<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl<const N: usize> PartialEq for Text<N> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl<const N: usize> Eq for Text<N> {}

impl<const N: usize> PartialEq<str> for Text<N> {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl<const N: usize> PartialEq<&str> for Text<N> {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl<const N: usize> Hash for Text<N> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl<const N: usize> Serialize for Text<N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de, const N: usize> Deserialize<'de> for Text<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TextVisitor<const N: usize>;

        impl<const N: usize> Visitor<'_> for TextVisitor<N> {
            type Value = Text<N>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "a NUL-free UTF-8 string of at most {N} bytes")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Text::try_from(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(TextVisitor::<N>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_uses_utf8_bytes_without_changing_identity() {
        assert_eq!(
            Text::<4>::try_from("abcd").expect("fits four bytes"),
            "abcd"
        );
        assert_eq!(
            Text::<4>::try_from("abcde"),
            Err(TextError::TooLong {
                capacity: 4,
                length: 5,
            })
        );
        assert_eq!(Text::<4>::try_from("éé").expect("fits four bytes"), "éé");
        assert!(matches!(
            Text::<3>::try_from("éé"),
            Err(TextError::TooLong { .. })
        ));
        assert_eq!(Text::<0>::try_from("").expect("empty fits zero bytes"), "");
        assert_eq!(Text::<4>::try_from("a\0b"), Err(TextError::EmbeddedNul));
    }

    #[test]
    fn escaped_json_obeys_decoded_byte_limits_and_nul_policy() {
        let text: Text<4> = serde_json::from_str(r#""\u00e9\u00e9""#)
            .expect("escaped characters decode to four bytes");
        assert_eq!(text, "éé");
        assert_eq!(
            serde_json::to_string(&text).expect("bounded text serializes as a string"),
            "\"éé\""
        );
        assert!(serde_json::from_str::<Text<3>>(r#""\u00e9\u00e9""#).is_err());
        assert!(serde_json::from_str::<Text<4>>(r#""a\u0000b""#).is_err());
        assert!(serde_json::from_str::<Text<4>>(r#""abcde""#).is_err());
        assert!(serde_json::from_str::<Text<4>>("null").is_err());
    }
}
