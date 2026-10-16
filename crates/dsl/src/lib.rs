//! DSL front end: lexer, parser, AST, name resolution, type check.
//! No I/O. The server never evaluates expression values; see `docs/design.md`.

pub mod ast;
pub mod diag;
pub mod interp;
pub mod lexer;
pub mod names;
pub mod parser;
pub mod print;
pub mod span;
pub mod state;
pub mod stmt;
