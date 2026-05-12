//! W1.1: WIT Lexer — Tokenizer for WebAssembly Interface Type files.
//!
//! Tokenizes `.wit` source into a stream of `WitToken` values.
//! Handles all WIT keywords: `package`, `world`, `interface`, `resource`,
//! `use`, `func`, `record`, `enum`, `variant`, `flags`, `type`, `static`,
//! `constructor`, `method`, `own`, `borrow`, `import`, `export`, `include`.

#![allow(missing_docs)] // P6.E4: data-heavy enum/struct module; field+variant names self-document

use std::fmt;

// ═══════════════════════════════════════════════════════════════════════
// Token Types
// ═══════════════════════════════════════════════════════════════════════

/// A token produced by the WIT lexer.
#[derive(Debug, Clone, PartialEq)]
pub struct WitToken {
    /// The kind of token.
    pub kind: WitTokenKind,
    /// The byte offset in the source where this token starts.
    pub offset: usize,
    /// The length of this token in bytes.
    pub len: usize,
}

impl WitToken {
    /// Creates a new token.
    pub fn new(kind: WitTokenKind, offset: usize, len: usize) -> Self {
        Self { kind, offset, len }
    }
}

/// All possible WIT token kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum WitTokenKind {
    // ── Keywords ──
    Package,
    World,
    Interface,
    Resource,
    Use,
    Func,
    Record,
    Enum,
    Variant,
    Flags,
    Type,
    Static,
    Constructor,
    Own,
    Borrow,
    Import,
    Export,
    Include,

    // ── Primitive type keywords ──
    U8,
    U16,
    U32,
    U64,
    S8,
    S16,
    S32,
    S64,
    F32,
    F64,
    Bool,
    Char,
    StringKw,

    // ── Built-in generic types ──
    List,
    Option_,
    Result_,
    Tuple_,

    // ── Symbols ──
    Colon,
    Semicolon,
    Comma,
    Dot,
    Slash,
    At,
    Equals,
    Arrow, // ->
    Star,  // *

    // ── Delimiters ──
    LBrace,
    RBrace,
    LParen,
    RParen,
    LAngle,
    RAngle,

    // ── Identifiers & Literals ──
    Ident(String),
    /// Semantic version string, e.g. "0.2.0"
    SemVer(String),
    /// Integer literal (for version numbers, etc.)
    Integer(u64),

    // ── Special ──
    /// Single-line comment `// ...`
    Comment(String),
    /// Documentation comment `/// ...`
    DocComment(String),
    /// End of file.
    Eof,
}

impl fmt::Display for WitTokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Package => write!(f, "package"),
            Self::World => write!(f, "world"),
            Self::Interface => write!(f, "interface"),
            Self::Resource => write!(f, "resource"),
            Self::Use => write!(f, "use"),
            Self::Func => write!(f, "func"),
            Self::Record => write!(f, "record"),
            Self::Enum => write!(f, "enum"),
            Self::Variant => write!(f, "variant"),
            Self::Flags => write!(f, "flags"),
            Self::Type => write!(f, "type"),
            Self::Static => write!(f, "static"),
            Self::Constructor => write!(f, "constructor"),
            Self::Own => write!(f, "own"),
            Self::Borrow => write!(f, "borrow"),
            Self::Import => write!(f, "import"),
            Self::Export => write!(f, "export"),
            Self::Include => write!(f, "include"),
            Self::U8 => write!(f, "u8"),
            Self::U16 => write!(f, "u16"),
            Self::U32 => write!(f, "u32"),
            Self::U64 => write!(f, "u64"),
            Self::S8 => write!(f, "s8"),
            Self::S16 => write!(f, "s16"),
            Self::S32 => write!(f, "s32"),
            Self::S64 => write!(f, "s64"),
            Self::F32 => write!(f, "f32"),
            Self::F64 => write!(f, "f64"),
            Self::Bool => write!(f, "bool"),
            Self::Char => write!(f, "char"),
            Self::StringKw => write!(f, "string"),
            Self::List => write!(f, "list"),
            Self::Option_ => write!(f, "option"),
            Self::Result_ => write!(f, "result"),
            Self::Tuple_ => write!(f, "tuple"),
            Self::Colon => write!(f, ":"),
            Self::Semicolon => write!(f, ";"),
            Self::Comma => write!(f, ","),
            Self::Dot => write!(f, "."),
            Self::Slash => write!(f, "/"),
            Self::At => write!(f, "@"),
            Self::Equals => write!(f, "="),
            Self::Arrow => write!(f, "->"),
            Self::Star => write!(f, "*"),
            Self::LBrace => write!(f, "{{"),
            Self::RBrace => write!(f, "}}"),
            Self::LParen => write!(f, "("),
            Self::RParen => write!(f, ")"),
            Self::LAngle => write!(f, "<"),
            Self::RAngle => write!(f, ">"),
            Self::Ident(s) => write!(f, "{s}"),
            Self::SemVer(v) => write!(f, "{v}"),
            Self::Integer(n) => write!(f, "{n}"),
            Self::Comment(c) => write!(f, "// {c}"),
            Self::DocComment(c) => write!(f, "/// {c}"),
            Self::Eof => write!(f, "EOF"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Lexer Error
// ═══════════════════════════════════════════════════════════════════════

/// Error during WIT tokenization.
#[derive(Debug, Clone, PartialEq)]
pub struct WitLexError {
    /// Error message.
    pub message: String,
    /// Byte offset where the error occurred.
    pub offset: usize,
}

impl fmt::Display for WitLexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "WIT lex error at offset {}: {}",
            self.offset, self.message
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Lexer
// ═══════════════════════════════════════════════════════════════════════

/// WIT file tokenizer.
pub struct WitLexer<'src> {
    source: &'src [u8],
    pos: usize,
}

impl<'src> WitLexer<'src> {
    /// Creates a new WIT lexer from source text.
    pub fn new(source: &'src str) -> Self {
        Self {
            source: source.as_bytes(),
            pos: 0,
        }
    }

    /// Tokenizes the entire source into a token vector.
    pub fn tokenize(&mut self) -> Result<Vec<WitToken>, WitLexError> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            if tok.kind == WitTokenKind::Eof {
                tokens.push(tok);
                break;
            }
            // Skip comments in the token stream (but doc comments are kept)
            if matches!(tok.kind, WitTokenKind::Comment(_)) {
                continue;
            }
            tokens.push(tok);
        }
        Ok(tokens)
    }

    fn peek_byte(&self) -> Option<u8> {
        self.source.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.source.get(self.pos).copied()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_whitespace(&mut self) {
        while let Some(b) = self.peek_byte() {
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self) -> Result<WitToken, WitLexError> {
        self.skip_whitespace();

        let start = self.pos;

        let Some(b) = self.advance() else {
            return Ok(WitToken::new(WitTokenKind::Eof, start, 0));
        };

        match b {
            // ── Comments ──
            b'/' if self.peek_byte() == Some(b'/') => {
                self.pos += 1; // skip second /
                let is_doc = self.peek_byte() == Some(b'/');
                if is_doc {
                    self.pos += 1; // skip third /
                }
                // Skip optional leading space
                if self.peek_byte() == Some(b' ') {
                    self.pos += 1;
                }
                let content_start = self.pos;
                while let Some(c) = self.peek_byte() {
                    if c == b'\n' {
                        break;
                    }
                    self.pos += 1;
                }
                let content =
                    String::from_utf8_lossy(&self.source[content_start..self.pos]).to_string();
                let kind = if is_doc {
                    WitTokenKind::DocComment(content)
                } else {
                    WitTokenKind::Comment(content)
                };
                Ok(WitToken::new(kind, start, self.pos - start))
            }

            b'/' => Ok(WitToken::new(WitTokenKind::Slash, start, 1)),

            // ── Symbols ──
            b':' => Ok(WitToken::new(WitTokenKind::Colon, start, 1)),
            b';' => Ok(WitToken::new(WitTokenKind::Semicolon, start, 1)),
            b',' => Ok(WitToken::new(WitTokenKind::Comma, start, 1)),
            b'.' => Ok(WitToken::new(WitTokenKind::Dot, start, 1)),
            b'@' => Ok(WitToken::new(WitTokenKind::At, start, 1)),
            b'=' => Ok(WitToken::new(WitTokenKind::Equals, start, 1)),
            b'*' => Ok(WitToken::new(WitTokenKind::Star, start, 1)),
            b'{' => Ok(WitToken::new(WitTokenKind::LBrace, start, 1)),
            b'}' => Ok(WitToken::new(WitTokenKind::RBrace, start, 1)),
            b'(' => Ok(WitToken::new(WitTokenKind::LParen, start, 1)),
            b')' => Ok(WitToken::new(WitTokenKind::RParen, start, 1)),
            b'<' => Ok(WitToken::new(WitTokenKind::LAngle, start, 1)),
            b'>' => Ok(WitToken::new(WitTokenKind::RAngle, start, 1)),

            // ── Arrow ──
            b'-' if self.peek_byte() == Some(b'>') => {
                self.pos += 1;
                Ok(WitToken::new(WitTokenKind::Arrow, start, 2))
            }

            // ── Numbers (could be part of semver or standalone) ──
            b'0'..=b'9' => self.lex_number(start),

            // ── Percent-prefixed ident (WIT convention for method/constructor/static) ──
            b'%' => {
                let ident_start = self.pos;
                while let Some(c) = self.peek_byte() {
                    if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&self.source[ident_start..self.pos]).to_string();
                Ok(WitToken::new(
                    WitTokenKind::Ident(text),
                    start,
                    self.pos - start,
                ))
            }

            // ── Identifiers & keywords ──
            c if c.is_ascii_alphabetic() || c == b'_' => self.lex_ident(start),

            other => Err(WitLexError {
                message: format!("unexpected character: '{}'", other as char),
                offset: start,
            }),
        }
    }

    fn lex_ident(&mut self, start: usize) -> Result<WitToken, WitLexError> {
        while let Some(c) = self.peek_byte() {
            if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }

        let text = &self.source[start..self.pos];
        let text_str = std::str::from_utf8(text).unwrap_or("");

        let kind = match text_str {
            "package" => WitTokenKind::Package,
            "world" => WitTokenKind::World,
            "interface" => WitTokenKind::Interface,
            "resource" => WitTokenKind::Resource,
            "use" => WitTokenKind::Use,
            "func" => WitTokenKind::Func,
            "record" => WitTokenKind::Record,
            "enum" => WitTokenKind::Enum,
            "variant" => WitTokenKind::Variant,
            "flags" => WitTokenKind::Flags,
            "type" => WitTokenKind::Type,
            "static" => WitTokenKind::Static,
            "constructor" => WitTokenKind::Constructor,
            "own" => WitTokenKind::Own,
            "borrow" => WitTokenKind::Borrow,
            "import" => WitTokenKind::Import,
            "export" => WitTokenKind::Export,
            "include" => WitTokenKind::Include,
            // Primitive types
            "u8" => WitTokenKind::U8,
            "u16" => WitTokenKind::U16,
            "u32" => WitTokenKind::U32,
            "u64" => WitTokenKind::U64,
            "s8" => WitTokenKind::S8,
            "s16" => WitTokenKind::S16,
            "s32" => WitTokenKind::S32,
            "s64" => WitTokenKind::S64,
            "f32" => WitTokenKind::F32,
            "f64" => WitTokenKind::F64,
            "bool" => WitTokenKind::Bool,
            "char" => WitTokenKind::Char,
            "string" => WitTokenKind::StringKw,
            // Built-in generics
            "list" => WitTokenKind::List,
            "option" => WitTokenKind::Option_,
            "result" => WitTokenKind::Result_,
            "tuple" => WitTokenKind::Tuple_,
            _ => WitTokenKind::Ident(text_str.to_string()),
        };

        Ok(WitToken::new(kind, start, self.pos - start))
    }

    fn lex_number(&mut self, start: usize) -> Result<WitToken, WitLexError> {
        // Collect digits
        while let Some(c) = self.peek_byte() {
            if c.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }

        // Check for semver: digits.digits.digits
        if self.peek_byte() == Some(b'.') {
            let save = self.pos;
            self.pos += 1;
            if self.peek_byte().is_some_and(|c| c.is_ascii_digit()) {
                while let Some(c) = self.peek_byte() {
                    if c.is_ascii_digit() {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                if self.peek_byte() == Some(b'.') {
                    self.pos += 1;
                    while let Some(c) = self.peek_byte() {
                        if c.is_ascii_digit() {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    // Also allow pre-release: -alpha, -rc.1, etc.
                    if self.peek_byte() == Some(b'-') {
                        self.pos += 1;
                        while let Some(c) = self.peek_byte() {
                            if c.is_ascii_alphanumeric() || c == b'.' || c == b'-' {
                                self.pos += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    let ver = String::from_utf8_lossy(&self.source[start..self.pos]).to_string();
                    return Ok(WitToken::new(
                        WitTokenKind::SemVer(ver),
                        start,
                        self.pos - start,
                    ));
                }
            }
            // Not a semver, restore position
            self.pos = save;
        }

        let text = std::str::from_utf8(&self.source[start..self.pos]).unwrap_or("0");
        let n = text.parse::<u64>().unwrap_or(0);
        Ok(WitToken::new(
            WitTokenKind::Integer(n),
            start,
            self.pos - start,
        ))
    }
}

/// Convenience function: tokenize a WIT source string.
pub fn tokenize_wit(source: &str) -> Result<Vec<WitToken>, WitLexError> {
    WitLexer::new(source).tokenize()
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn w1_1_tokenize_empty() {
        let tokens = tokenize_wit("").unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, WitTokenKind::Eof);
    }

    #[test]
    fn w1_1_tokenize_keywords() {
        let src = "package world interface resource use func record enum variant flags type";
        let tokens = tokenize_wit(src).unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(kinds[0], &WitTokenKind::Package);
        assert_eq!(kinds[1], &WitTokenKind::World);
        assert_eq!(kinds[2], &WitTokenKind::Interface);
        assert_eq!(kinds[3], &WitTokenKind::Resource);
        assert_eq!(kinds[4], &WitTokenKind::Use);
        assert_eq!(kinds[5], &WitTokenKind::Func);
        assert_eq!(kinds[6], &WitTokenKind::Record);
        assert_eq!(kinds[7], &WitTokenKind::Enum);
        assert_eq!(kinds[8], &WitTokenKind::Variant);
        assert_eq!(kinds[9], &WitTokenKind::Flags);
        assert_eq!(kinds[10], &WitTokenKind::Type);
    }

    #[test]
    fn w1_1_tokenize_primitives() {
        let src = "u8 u16 u32 u64 s8 s16 s32 s64 f32 f64 bool char string";
        let tokens = tokenize_wit(src).unwrap();
        assert_eq!(tokens[0].kind, WitTokenKind::U8);
        assert_eq!(tokens[4].kind, WitTokenKind::S8);
        assert_eq!(tokens[10].kind, WitTokenKind::Bool);
        assert_eq!(tokens[11].kind, WitTokenKind::Char);
        assert_eq!(tokens[12].kind, WitTokenKind::StringKw);
    }

    #[test]
    fn w1_1_tokenize_symbols() {
        let src = ": ; , . / @ = -> * { } ( ) < >";
        let tokens = tokenize_wit(src).unwrap();
        assert_eq!(tokens[0].kind, WitTokenKind::Colon);
        assert_eq!(tokens[1].kind, WitTokenKind::Semicolon);
        assert_eq!(tokens[7].kind, WitTokenKind::Arrow);
        assert_eq!(tokens[8].kind, WitTokenKind::Star);
    }

    #[test]
    fn w1_1_tokenize_semver() {
        let src = "0.2.0";
        let tokens = tokenize_wit(src).unwrap();
        assert_eq!(tokens[0].kind, WitTokenKind::SemVer("0.2.0".into()));
    }

    #[test]
    fn w1_1_tokenize_package_decl() {
        let src = "package wasi:cli@0.2.0;";
        let tokens = tokenize_wit(src).unwrap();
        assert_eq!(tokens[0].kind, WitTokenKind::Package);
        assert_eq!(tokens[1].kind, WitTokenKind::Ident("wasi".into()));
        assert_eq!(tokens[2].kind, WitTokenKind::Colon);
        assert_eq!(tokens[3].kind, WitTokenKind::Ident("cli".into()));
        assert_eq!(tokens[4].kind, WitTokenKind::At);
        assert_eq!(tokens[5].kind, WitTokenKind::SemVer("0.2.0".into()));
        assert_eq!(tokens[6].kind, WitTokenKind::Semicolon);
    }

    #[test]
    fn w1_1_tokenize_doc_comments() {
        let src = "/// A doc comment\ninterface test {}";
        let tokens = tokenize_wit(src).unwrap();
        assert!(matches!(&tokens[0].kind, WitTokenKind::DocComment(c) if c == "A doc comment"));
        assert_eq!(tokens[1].kind, WitTokenKind::Interface);
    }

    #[test]
    fn w1_1_tokenize_regular_comments_skipped() {
        let src = "// regular comment\ninterface test {}";
        let tokens = tokenize_wit(src).unwrap();
        // Regular comments are skipped
        assert_eq!(tokens[0].kind, WitTokenKind::Interface);
    }

    #[test]
    fn w1_1_tokenize_wasi_cli_command_world() {
        let src = r#"
package wasi:cli@0.2.0;

world command {
    import wasi:io/streams@0.2.0;
    import wasi:filesystem/types@0.2.0;
    export run: func() -> result;
}
"#;
        let tokens = tokenize_wit(src).unwrap();
        // Should successfully tokenize the entire WASI CLI command world
        assert!(tokens.iter().any(|t| t.kind == WitTokenKind::Package));
        assert!(tokens.iter().any(|t| t.kind == WitTokenKind::World));
        assert!(tokens.iter().any(|t| t.kind == WitTokenKind::Import));
        assert!(tokens.iter().any(|t| t.kind == WitTokenKind::Export));
        assert!(
            tokens
                .iter()
                .any(|t| matches!(&t.kind, WitTokenKind::Ident(s) if s == "command"))
        );
    }

    #[test]
    fn w1_1_tokenize_hyphenated_idents() {
        let src = "input-stream output-stream monotonic-clock";
        let tokens = tokenize_wit(src).unwrap();
        assert_eq!(tokens[0].kind, WitTokenKind::Ident("input-stream".into()));
        assert_eq!(tokens[1].kind, WitTokenKind::Ident("output-stream".into()));
        assert_eq!(
            tokens[2].kind,
            WitTokenKind::Ident("monotonic-clock".into())
        );
    }
}
