use crate::{ast, elaborate::elaborate, parser::Parser};

pub fn front_end(source: &str) -> (ast::Expr, Vec<ast::TypeDecl>) {
    let (program, decls) =
        Parser::parse(source).unwrap_or_else(|e| panic!("parse error in {source:?}: {e}"));
    elaborate(program, &decls).unwrap_or_else(|e| panic!("elaboration error in {source:?}: {e}"))
}
