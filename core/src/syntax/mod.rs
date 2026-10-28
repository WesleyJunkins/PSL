//! Lexer, AST and parser for the PSL grammar (docs/04).

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod span;
pub mod token;

pub use ast::*;
pub use parser::parse_document;
pub use span::{FileId, Span};
