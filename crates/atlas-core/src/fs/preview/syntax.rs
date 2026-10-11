use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Whitespace,
    Keyword,
    Type,
    Identifier,
    Number,
    String,
    Comment,
    Preprocessor,
    Symbol,
    DiffAdd,
    DiffRemove,
    DiffHeader,
    LogError,
    LogWarn,
    LogInfo,
}

impl TokenKind {
    /// Maps our token kind to a canonical theme token name compatible with HighlightTheme.
    pub fn theme_name(&self) -> &'static str {
        match self {
            TokenKind::Whitespace => "whitespace",
            TokenKind::Keyword => "keyword",
            TokenKind::Type => "type",
            TokenKind::Identifier => "variable",
            TokenKind::Number => "number",
            TokenKind::String => "string",
            TokenKind::Comment => "comment",
            TokenKind::Preprocessor => "preproc",
            TokenKind::Symbol => "operator",
            TokenKind::DiffAdd => "string",         // Green
            TokenKind::DiffRemove => "preproc",      // Red
            TokenKind::DiffHeader => "keyword",      // Cyan/Blue
            TokenKind::LogError => "preproc",        // Red
            TokenKind::LogWarn => "number",          // Yellow
            TokenKind::LogInfo => "comment",         // Dim
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxToken {
    pub kind: TokenKind,
    pub range: Range<usize>,
}

static C_KEYWORDS: &[&str] = &[
    "auto", "break", "case", "catch", "class", "const", "constexpr", "continue",
    "default", "delete", "do", "else", "enum", "explicit", "export", "extern",
    "for", "friend", "goto", "if", "inline", "mutable", "namespace", "new",
    "noexcept", "operator", "private", "protected", "public", "register",
    "return", "sizeof", "static", "static_assert", "static_cast", "struct",
    "switch", "template", "this", "throw", "try", "typedef", "typeid", "typename",
    "union", "using", "virtual", "void", "volatile", "while", "nullptr", "true", "false",
];

static C_TYPES: &[&str] = &[
    "bool", "char", "char8_t", "char16_t", "char32_t", "double", "float",
    "int", "long", "short", "signed", "unsigned", "wchar_t", "size_t",
    "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t",
    "uint32_t", "uint64_t", "uintptr_t", "intptr_t", "U8", "U16", "U32", "U64",
    "S8", "S16", "S32", "S64", "F32", "F64", "B32", "String8", "Vec2F32", "Vec4F32",
];

static JS_KEYWORDS: &[&str] = &[
    "abstract", "arguments", "await", "async", "boolean", "break", "byte", "case",
    "catch", "char", "class", "const", "continue", "debugger", "default",
    "delete", "do", "double", "else", "enum", "eval", "export", "extends",
    "false", "final", "finally", "float", "for", "function", "goto", "if",
    "implements", "import", "in", "instanceof", "int", "interface", "let",
    "long", "native", "new", "null", "package", "private", "protected",
    "public", "return", "short", "static", "super", "switch", "synchronized",
    "this", "throw", "throws", "transient", "true", "try", "typeof", "var",
    "void", "volatile", "while", "with", "yield", "type", "readonly", "as", "from", "of",
];

static PY_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "False", "finally", "for",
    "from", "global", "if", "import", "in", "is", "lambda", "None",
    "nonlocal", "not", "or", "pass", "raise", "return", "True", "try",
    "while", "with", "yield",
];

static RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn",
    "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];

static RUST_TYPES: &[&str] = &[
    "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize",
    "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64",
    "String", "Vec", "Option", "Result", "Some", "None", "Ok", "Err",
    "Box", "Rc", "Arc", "RefCell", "Mutex",
];

static SQL_KEYWORDS: &[&str] = &[
    "select", "from", "where", "insert", "into", "values", "update", "set",
    "delete", "create", "table", "index", "view", "trigger", "drop", "alter",
    "add", "column", "primary", "key", "foreign", "references", "not", "null",
    "unique", "default", "autoincrement", "check", "constraint", "and", "or",
    "in", "is", "like", "order", "by", "group", "having", "limit", "offset",
    "join", "left", "right", "inner", "outer", "on", "as", "distinct",
];

static SQL_TYPES: &[&str] = &[
    "integer", "int", "text", "real", "blob", "varchar", "boolean", "numeric",
    "float", "double", "datetime", "timestamp",
];

static ZIG_KEYWORDS: &[&str] = &[
    "fn", "pub", "const", "var", "struct", "enum", "union", "error", "inline",
    "comptime", "test", "try", "catch", "defer", "errdefer", "while", "for",
    "if", "else", "switch", "return", "break", "continue", "asm", "usingnamespace",
    "align", "linksection", "threadlocal", "allowzero", "volatile", "export",
    "extern", "naked", "anytype", "unreachable", "and", "or", "orelse", "null",
    "undefined", "true", "false",
];

static ZIG_TYPES: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
    "f16", "f32", "f64", "f80", "f128", "bool", "void", "noreturn", "type", "anyerror",
    "anyopaque", "c_int", "c_uint", "c_long", "c_ulong", "c_void",
];

static GO_KEYWORDS: &[&str] = &[
    "package", "import", "func", "type", "struct", "interface", "var", "const",
    "return", "if", "else", "for", "range", "switch", "case", "default", "select",
    "chan", "go", "defer", "map", "make", "new", "len", "cap", "append", "copy",
    "delete", "panic", "recover", "true", "false", "nil",
];

static GO_TYPES: &[&str] = &[
    "string", "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16",
    "uint32", "uint64", "uintptr", "byte", "rune", "float32", "float64",
    "complex64", "complex128", "bool", "error", "any",
];

static SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "case", "esac",
    "while", "until", "function", "return", "exit", "echo", "export", "source", "local",
    "set", "unset", "alias", "read",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum LangKind {
    Cpp,
    Js,
    Python,
    Rust,
    Zig,
    Go,
    Sql,
    Shell,
    Config,
    Diff,
    Log,
    Plain,
}

fn detect_lang(ext: &str) -> LangKind {
    let lower = ext.trim_start_matches('.').to_ascii_lowercase();
    match lower.as_str() {
        "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" | "inl" | "hlsl" | "glsl" | "frag" | "vert" => {
            LangKind::Cpp
        }
        "js" | "ts" | "jsx" | "tsx" | "mjs" | "cjs" | "html" | "xml" | "svelte" | "vue"
        | "astro" => LangKind::Js,
        "py" => LangKind::Python,
        "rs" => LangKind::Rust,
        "zig" => LangKind::Zig,
        "go" => LangKind::Go,
        "sql" | "sqlite" | "db" => LangKind::Sql,
        "sh" | "bash" | "zsh" | "ps1" | "bat" | "cmd" => LangKind::Shell,
        "toml" | "yaml" | "yml" | "ini" | "cfg" | "conf" | "env" | "dockerfile" | "makefile"
        | "gitignore" | "gitattributes" => LangKind::Config,
        "diff" | "patch" => LangKind::Diff,
        "log" | "err" | "out" => LangKind::Log,
        _ => LangKind::Plain,
    }
}

/// Tokenizes a single line of text with stateful multi-line block comment tracking.
pub fn tokenize_line(
    text: &str,
    ext: &str,
    in_block_comment: &mut bool,
) -> Vec<SyntaxToken> {
    let mut tokens = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return tokens;
    }

    let lang = detect_lang(ext);

    // Fast path: Diff / Patch
    if lang == LangKind::Diff {
        if text.starts_with("--- ")
            || text.starts_with("+++ ")
            || text.starts_with("@@")
            || text.starts_with("diff ")
            || text.starts_with("index ")
        {
            tokens.push(SyntaxToken {
                kind: TokenKind::DiffHeader,
                range: 0..len,
            });
            return tokens;
        } else if bytes[0] == b'+' {
            tokens.push(SyntaxToken {
                kind: TokenKind::DiffAdd,
                range: 0..len,
            });
            return tokens;
        } else if bytes[0] == b'-' {
            tokens.push(SyntaxToken {
                kind: TokenKind::DiffRemove,
                range: 0..len,
            });
            return tokens;
        }
    }

    let mut i = 0;

    // Check if continuing inside a multi-line block comment
    if *in_block_comment {
        if let Some(close_idx) = text.find("*/") {
            let end = close_idx + 2;
            tokens.push(SyntaxToken {
                kind: TokenKind::Comment,
                range: 0..end,
            });
            *in_block_comment = false;
            i = end;
        } else {
            tokens.push(SyntaxToken {
                kind: TokenKind::Comment,
                range: 0..len,
            });
            return tokens;
        }
    }

    while i < len {
        let b = bytes[i];

        // 1. Whitespace
        if b == b' ' || b == b'\t' || b == b'\r' || b == b'\n' {
            let start = i;
            while i < len && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\r' || bytes[i] == b'\n') {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::Whitespace,
                range: start..i,
            });
            continue;
        }

        // 2. Multi-line Block Comments /* ... */
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            let start = i;
            if let Some(close_idx) = text[i + 2..].find("*/") {
                let end = i + 2 + close_idx + 2;
                tokens.push(SyntaxToken {
                    kind: TokenKind::Comment,
                    range: start..end,
                });
                i = end;
            } else {
                tokens.push(SyntaxToken {
                    kind: TokenKind::Comment,
                    range: start..len,
                });
                *in_block_comment = true;
                break;
            }
            continue;
        }

        // 3. Line Comments
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            tokens.push(SyntaxToken {
                kind: TokenKind::Comment,
                range: i..len,
            });
            break;
        }
        if (lang == LangKind::Python || lang == LangKind::Shell || lang == LangKind::Config) && b == b'#' {
            tokens.push(SyntaxToken {
                kind: TokenKind::Comment,
                range: i..len,
            });
            break;
        }
        if lang == LangKind::Sql && b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            tokens.push(SyntaxToken {
                kind: TokenKind::Comment,
                range: i..len,
            });
            break;
        }

        // 4. Preprocessor (C/C++ or HLSL/GLSL)
        if lang == LangKind::Cpp && b == b'#' {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' || bytes[i] == b'#') {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::Preprocessor,
                range: start..i,
            });
            continue;
        }

        // 5. Strings: "...", '...', and `...`
        if b == b'"' || b == b'\'' || (lang == LangKind::Js && b == b'`') {
            let quote = b;
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::String,
                range: start..i,
            });
            continue;
        }

        // 6. Numbers
        if b.is_ascii_digit() || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit()) {
            let start = i;
            if b == b'0' && i + 1 < len && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
                i += 2;
                while i < len && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
                    i += 1;
                }
            } else if b == b'0' && i + 1 < len && (bytes[i + 1] == b'b' || bytes[i + 1] == b'B') {
                i += 2;
                while i < len && (bytes[i] == b'0' || bytes[i] == b'1' || bytes[i] == b'_') {
                    i += 1;
                }
            } else {
                while i < len
                    && (bytes[i].is_ascii_digit()
                        || bytes[i] == b'.'
                        || bytes[i] == b'e'
                        || bytes[i] == b'E'
                        || bytes[i] == b'f'
                        || bytes[i] == b'u'
                        || bytes[i] == b'l'
                        || bytes[i] == b'L'
                        || bytes[i] == b'_')
                {
                    i += 1;
                }
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::Number,
                range: start..i,
            });
            continue;
        }

        // 7. JSX / TSX Tag Recognition (<Component or </Component>)
        if lang == LangKind::Js
            && b == b'<'
            && i + 1 < len
            && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'/' || bytes[i + 1] == b'>')
        {
            let start = i;
            i += 1;
            if i < len && bytes[i] == b'/' {
                i += 1;
            }
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'-') {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::Type,
                range: start..i,
            });
            continue;
        }

        // 7b. Zig Builtin Functions (@import, @as, etc.)
        if lang == LangKind::Zig && b == b'@' && i + 1 < len && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_') {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::Keyword,
                range: start..i,
            });
            continue;
        }

        // 8. Identifiers / Keywords / Types
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            let mut kind = TokenKind::Identifier;

            match lang {
                LangKind::Cpp => {
                    if C_KEYWORDS.contains(&word) {
                        kind = TokenKind::Keyword;
                    } else if C_TYPES.contains(&word) {
                        kind = TokenKind::Type;
                    }
                }
                LangKind::Js => {
                    if JS_KEYWORDS.contains(&word) {
                        kind = TokenKind::Keyword;
                    }
                }
                LangKind::Python => {
                    if PY_KEYWORDS.contains(&word) {
                        kind = TokenKind::Keyword;
                    }
                }
                LangKind::Rust => {
                    if RUST_KEYWORDS.contains(&word) {
                        kind = TokenKind::Keyword;
                    } else if RUST_TYPES.contains(&word) {
                        kind = TokenKind::Type;
                    }
                }
                LangKind::Zig => {
                    if ZIG_KEYWORDS.contains(&word) {
                        kind = TokenKind::Keyword;
                    } else if ZIG_TYPES.contains(&word) {
                        kind = TokenKind::Type;
                    }
                }
                LangKind::Go => {
                    if GO_KEYWORDS.contains(&word) {
                        kind = TokenKind::Keyword;
                    } else if GO_TYPES.contains(&word) {
                        kind = TokenKind::Type;
                    }
                }
                LangKind::Sql => {
                    let lower_word = word.to_ascii_lowercase();
                    if SQL_KEYWORDS.contains(&lower_word.as_str()) {
                        kind = TokenKind::Keyword;
                    } else if SQL_TYPES.contains(&lower_word.as_str()) {
                        kind = TokenKind::Type;
                    }
                }
                LangKind::Shell => {
                    if SHELL_KEYWORDS.contains(&word) {
                        kind = TokenKind::Keyword;
                    }
                }
                LangKind::Log => {
                    if word == "ERROR" || word == "FATAL" || word == "FAIL" {
                        kind = TokenKind::LogError;
                    } else if word == "WARN" || word == "WARNING" {
                        kind = TokenKind::LogWarn;
                    } else if word == "INFO" || word == "DEBUG" || word == "TRACE" {
                        kind = TokenKind::LogInfo;
                    }
                }
                _ => {}
            }

            tokens.push(SyntaxToken {
                kind,
                range: start..i,
            });
            continue;
        }

        // 9. Symbols / Operators
        tokens.push(SyntaxToken {
            kind: TokenKind::Symbol,
            range: i..i + 1,
        });
        i += 1;
    }

    tokens
}

/// Tokenize an entire document into lines of (TokenKind, text slice)
pub fn tokenize_document<'a>(
    text: &'a str,
    ext: &str,
) -> Vec<Vec<(TokenKind, &'a str)>> {
    let mut in_block_comment = false;
    text.lines()
        .map(|line| {
            let tokens = tokenize_line(line, ext, &mut in_block_comment);
            tokens
                .into_iter()
                .map(|t| (t.kind, &line[t.range]))
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_tokens() {
        let code = "fn main() {\n    let x: u32 = 42;\n    println!(\"hello\"); // comment\n}";
        let lines = tokenize_document(code, "rs");
        assert_eq!(lines.len(), 4);

        // Line 0: fn main() {
        let kinds_0: Vec<TokenKind> = lines[0].iter().map(|(k, _)| *k).collect();
        assert!(kinds_0.contains(&TokenKind::Keyword)); // "fn"
        assert!(kinds_0.contains(&TokenKind::Identifier)); // "main"

        // Line 1: let x: u32 = 42;
        let kinds_1: Vec<TokenKind> = lines[1].iter().map(|(k, _)| *k).collect();
        assert!(kinds_1.contains(&TokenKind::Keyword)); // "let"
        assert!(kinds_1.contains(&TokenKind::Type)); // "u32"
        assert!(kinds_1.contains(&TokenKind::Number)); // "42"

        // Line 2: println!("hello"); // comment
        let kinds_2: Vec<TokenKind> = lines[2].iter().map(|(k, _)| *k).collect();
        assert!(kinds_2.contains(&TokenKind::String)); // "\"hello\""
        assert!(kinds_2.contains(&TokenKind::Comment)); // "// comment"
    }

    #[test]
    fn test_c_multiline_comment() {
        let code = "/* start\n   middle\n   end */ int x = 1;";
        let lines = tokenize_document(code, "c");
        assert_eq!(lines.len(), 3);

        assert_eq!(lines[0][0].0, TokenKind::Comment);
        assert_eq!(lines[1][0].0, TokenKind::Comment);
        assert_eq!(lines[2][0].0, TokenKind::Comment);
        let kinds_2: Vec<TokenKind> = lines[2].iter().map(|(k, _)| *k).collect();
        assert!(kinds_2.contains(&TokenKind::Type)); // "int"
        assert!(kinds_2.contains(&TokenKind::Number)); // "1"
    }

    #[test]
    fn test_python_tokens() {
        let code = "def hello():\n    # greeting\n    return 'world'";
        let lines = tokenize_document(code, "py");
        assert_eq!(lines.len(), 3);
        let kinds_0: Vec<TokenKind> = lines[0].iter().map(|(k, _)| *k).collect();
        assert!(kinds_0.contains(&TokenKind::Keyword)); // "def"
        let kinds_1: Vec<TokenKind> = lines[1].iter().map(|(k, _)| *k).collect();
        assert!(kinds_1.contains(&TokenKind::Comment)); // "# greeting"
        let kinds_2: Vec<TokenKind> = lines[2].iter().map(|(k, _)| *k).collect();
        assert!(kinds_2.contains(&TokenKind::Keyword)); // "return"
        assert!(kinds_2.contains(&TokenKind::String)); // "'world'"
    }

    #[test]
    fn test_diff_tokens() {
        let diff = "--- a/file.txt\n+++ b/file.txt\n@@ -1,3 +1,3 @@\n-old\n+new";
        let lines = tokenize_document(diff, "diff");
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0][0].0, TokenKind::DiffHeader);
        assert_eq!(lines[1][0].0, TokenKind::DiffHeader);
        assert_eq!(lines[2][0].0, TokenKind::DiffHeader);
        assert_eq!(lines[3][0].0, TokenKind::DiffRemove);
        assert_eq!(lines[4][0].0, TokenKind::DiffAdd);
    }

    #[test]
    fn test_log_tokens() {
        let log = "[ERROR] Connection failed\n[WARN] High memory\n[INFO] Starting up";
        let lines = tokenize_document(log, "log");
        assert_eq!(lines.len(), 3);
        let kinds_0: Vec<TokenKind> = lines[0].iter().map(|(k, _)| *k).collect();
        assert!(kinds_0.contains(&TokenKind::LogError));
        let kinds_1: Vec<TokenKind> = lines[1].iter().map(|(k, _)| *k).collect();
        assert!(kinds_1.contains(&TokenKind::LogWarn));
        let kinds_2: Vec<TokenKind> = lines[2].iter().map(|(k, _)| *k).collect();
        assert!(kinds_2.contains(&TokenKind::LogInfo));
    }
}

