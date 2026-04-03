/// Kinds of symbols we extract from source files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Const,
    Static,
    TypeAlias,
    Module,
    Macro,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Const => "const",
            Self::Static => "static",
            Self::TypeAlias => "type_alias",
            Self::Module => "module",
            Self::Macro => "macro",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "function" => Some(Self::Function),
            "struct" => Some(Self::Struct),
            "enum" => Some(Self::Enum),
            "trait" => Some(Self::Trait),
            "impl" => Some(Self::Impl),
            "const" => Some(Self::Const),
            "static" => Some(Self::Static),
            "type_alias" => Some(Self::TypeAlias),
            "module" => Some(Self::Module),
            "macro" => Some(Self::Macro),
            _ => None,
        }
    }
}

/// A symbol extracted from a source file.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub line: u32,
    pub end_line: u32,
    pub parent: Option<String>,
    pub signature: Option<String>,
}

/// Parse symbols from file content given a language identifier.
/// Returns empty vec for unsupported languages.
pub fn parse_symbols(content: &[u8], language: &str) -> Vec<Symbol> {
    match language {
        "rust" => parse_rust_symbols(content),
        _ => Vec::new(),
    }
}

/// Returns true if the given language is supported for symbol parsing.
pub fn is_language_supported(language: &str) -> bool {
    matches!(language, "rust")
}

/// Tree-sitter S-expression query for Rust symbol extraction.
const RUST_QUERY: &str = r#"
(function_item name: (identifier) @name) @definition
(struct_item name: (type_identifier) @name) @definition
(enum_item name: (type_identifier) @name) @definition
(trait_item name: (type_identifier) @name) @definition
(impl_item type: (_) @name) @definition
(const_item name: (identifier) @name) @definition
(static_item name: (identifier) @name) @definition
(type_item name: (type_identifier) @name) @definition
(mod_item name: (identifier) @name) @definition
(macro_definition name: (identifier) @name) @definition
"#;

fn parse_rust_symbols(content: &[u8]) -> Vec<Symbol> {
    use tree_sitter::StreamingIterator;

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("failed to set rust language");

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let query = tree_sitter::Query::new(&tree_sitter_rust::LANGUAGE.into(), RUST_QUERY)
        .expect("invalid rust query");
    let name_idx = query.capture_index_for_name("name").unwrap();
    let def_idx = query.capture_index_for_name("definition").unwrap();

    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content);

    let mut symbols = Vec::new();

    while let Some(m) = matches.next() {
        let def_node = m.captures.iter().find(|c| c.index == def_idx).unwrap().node;
        let name_node = m
            .captures
            .iter()
            .find(|c| c.index == name_idx)
            .unwrap()
            .node;

        let name = match name_node.utf8_text(content) {
            Ok(n) => n.to_string(),
            Err(_) => continue,
        };

        let kind = match def_node.kind() {
            "function_item" => SymbolKind::Function,
            "struct_item" => SymbolKind::Struct,
            "enum_item" => SymbolKind::Enum,
            "trait_item" => SymbolKind::Trait,
            "impl_item" => SymbolKind::Impl,
            "const_item" => SymbolKind::Const,
            "static_item" => SymbolKind::Static,
            "type_item" => SymbolKind::TypeAlias,
            "mod_item" => SymbolKind::Module,
            "macro_definition" => SymbolKind::Macro,
            _ => continue,
        };

        let parent = find_parent_scope(def_node, content);

        let signature = if kind == SymbolKind::Function {
            Some(build_function_signature(def_node, content, &name))
        } else {
            None
        };

        symbols.push(Symbol {
            name,
            kind,
            line: def_node.start_position().row as u32,
            end_line: def_node.end_position().row as u32,
            parent,
            signature,
        });
    }

    symbols
}

/// Walk up the tree to find the enclosing impl or mod block.
fn find_parent_scope(node: tree_sitter::Node<'_>, content: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "impl_item" => {
                return parent
                    .child_by_field_name("type")
                    .and_then(|n| n.utf8_text(content).ok())
                    .map(String::from);
            }
            "mod_item" => {
                return parent
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(content).ok())
                    .map(String::from);
            }
            _ => current = parent.parent(),
        }
    }
    None
}

/// Build a short function signature: `fn name(params) -> ReturnType`
fn build_function_signature(node: tree_sitter::Node<'_>, content: &[u8], name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .and_then(|n| n.utf8_text(content).ok())
        .unwrap_or("()");
    let ret = node
        .child_by_field_name("return_type")
        .and_then(|n| n.utf8_text(content).ok())
        .map(|r| format!(" -> {r}"))
        .unwrap_or_default();
    format!("fn {name}{params}{ret}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rust_function() {
        let src = b"fn hello() -> bool { true }";
        let symbols = parse_symbols(src, "rust");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].line, 0);
        assert!(symbols[0].signature.as_ref().unwrap().contains("fn hello"));
    }

    #[test]
    fn test_parse_rust_struct_and_impl_method() {
        let src = br#"
pub struct Foo {
    x: i32,
}

impl Foo {
    pub fn bar(&self) -> i32 {
        self.x
    }
}
"#;
        let symbols = parse_symbols(src, "rust");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Foo"), "should find struct Foo: {names:?}");
        assert!(names.contains(&"bar"), "should find method bar: {names:?}");

        let foo = symbols
            .iter()
            .find(|s| s.name == "Foo" && s.kind == SymbolKind::Struct)
            .unwrap();
        assert_eq!(foo.kind, SymbolKind::Struct);

        let bar = symbols.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.kind, SymbolKind::Function);
        assert_eq!(bar.parent.as_deref(), Some("Foo"));
    }

    #[test]
    fn test_parse_rust_enum_trait_const_static_type() {
        let src = br#"
pub enum Color { Red, Green, Blue }
pub trait Drawable { fn draw(&self); }
pub const MAX: u32 = 100;
pub static COUNTER: u32 = 0;
type Alias = Vec<String>;
"#;
        let symbols = parse_symbols(src, "rust");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Color"), "missing Color: {names:?}");
        assert!(names.contains(&"Drawable"), "missing Drawable: {names:?}");
        assert!(names.contains(&"MAX"), "missing MAX: {names:?}");
        assert!(names.contains(&"COUNTER"), "missing COUNTER: {names:?}");
        assert!(names.contains(&"Alias"), "missing Alias: {names:?}");

        let color = symbols.iter().find(|s| s.name == "Color").unwrap();
        assert_eq!(color.kind, SymbolKind::Enum);
        let drawable = symbols.iter().find(|s| s.name == "Drawable").unwrap();
        assert_eq!(drawable.kind, SymbolKind::Trait);
        let max = symbols.iter().find(|s| s.name == "MAX").unwrap();
        assert_eq!(max.kind, SymbolKind::Const);
        let counter = symbols.iter().find(|s| s.name == "COUNTER").unwrap();
        assert_eq!(counter.kind, SymbolKind::Static);
        let alias = symbols.iter().find(|s| s.name == "Alias").unwrap();
        assert_eq!(alias.kind, SymbolKind::TypeAlias);
    }

    #[test]
    fn test_parse_rust_impl_block_as_symbol() {
        let src = br#"
struct Foo;
impl Foo {
    fn method(&self) {}
}
"#;
        let symbols = parse_symbols(src, "rust");
        let impl_sym = symbols.iter().find(|s| s.kind == SymbolKind::Impl).unwrap();
        assert_eq!(impl_sym.name, "Foo");
    }

    #[test]
    fn test_parse_rust_module() {
        let src = br#"
mod inner {
    fn nested() {}
}
"#;
        let symbols = parse_symbols(src, "rust");
        let inner = symbols.iter().find(|s| s.name == "inner").unwrap();
        assert_eq!(inner.kind, SymbolKind::Module);
        let nested = symbols.iter().find(|s| s.name == "nested").unwrap();
        assert_eq!(nested.parent.as_deref(), Some("inner"));
    }

    #[test]
    fn test_parse_rust_macro() {
        let src = br#"
macro_rules! my_macro {
    () => {};
}
"#;
        let symbols = parse_symbols(src, "rust");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "my_macro");
        assert_eq!(symbols[0].kind, SymbolKind::Macro);
    }

    #[test]
    fn test_parse_unsupported_language() {
        let symbols = parse_symbols(b"def foo(): pass", "python");
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_function_signature_extraction() {
        let src = b"pub fn process(x: i32, y: &str) -> bool { true }";
        let symbols = parse_symbols(src, "rust");
        let sig = symbols[0].signature.as_ref().unwrap();
        assert!(sig.starts_with("fn process("), "sig was: {sig}");
        assert!(sig.contains("-> bool"), "sig was: {sig}");
    }

    #[test]
    fn test_is_language_supported() {
        assert!(is_language_supported("rust"));
        assert!(!is_language_supported("python"));
        assert!(!is_language_supported("unknown"));
    }
}
