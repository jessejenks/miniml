mod ast;
mod codegen;
mod elaborate;
mod interpret;
mod lexer;
mod parser;
mod surface;
mod typecheck;

use crate::codegen::Compiler;
use crate::elaborate::surface_to_core;
use crate::interpret::interpret;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::typecheck::TypeChecker;
use clap::Parser as _;
use inkwell::context::Context;
use miette::{Diagnostic, NamedSource, Report};
use std::path::Path;

#[derive(clap::Parser, Debug)]
struct Args {
    /// Input File
    #[arg(required = true)]
    path: String,
    /// Run the lexer
    #[arg(long)]
    lex: bool,
    /// Run the parser
    #[arg(long)]
    parse: bool,
    /// Run the elaborator
    #[arg(long)]
    elab: bool,
    /// Print types for declarations
    #[arg(long, visible_alias = "interface", short = 'i')]
    types: bool,
    /// Run interpreter or codegen even if type check failed
    #[arg(long)]
    force: bool,
    /// Run the interpreter
    #[arg(long)]
    interpret: bool,
    /// Output to LLVM
    #[arg(long)]
    codegen: bool,
}

fn main() -> miette::Result<()> {
    let args = Args::parse();
    let input = Path::new(&args.path);

    let source = match std::fs::read_to_string(input) {
        Err(e) => {
            eprintln!("failed to read file: {e}");
            std::process::exit(1)
        }
        Ok(s) => s,
    };

    if args.lex {
        let lexer = Lexer::new(&source);
        for span in lexer {
            match span {
                Err(e) => {
                    return Err(report(e, &args.path, &source));
                }
                Ok((start, tok, end)) => println!("{tok:?} ({start}, {end})"),
            }
        }
        return Ok(());
    }

    let program = Parser::parse(&source).map_err(|e| report(e, &args.path, &source))?;
    if args.parse {
        println!("{program}");
        return Ok(());
    }

    let elaborated_program = surface_to_core(program);
    if args.elab {
        println!("{elaborated_program}");
        return Ok(());
    }

    let mut type_checker = TypeChecker::new();
    type_checker.set_env(
        "print".to_string(),
        typecheck::Scheme::as_monotype(typecheck::TypeExpr::arrow(
            typecheck::TypeExpr::Int,
            typecheck::TypeExpr::Unit,
        )),
    );
    let type_check_result = type_checker.infer(&elaborated_program);
    match type_check_result {
        Err(e) => {
            let rep = report(e, &args.path, &source);
            if !args.force {
                return Err(rep);
            }
            eprintln!("{rep:?}");
        }
        Ok(sch) if args.types => {
            let mut spans = Vec::new();
            let mut expr = &elaborated_program;
            loop {
                match &expr.kind {
                    ast::ExprKind::LetIn(_, Some(span), _, body) => {
                        spans.push(*span);
                        expr = body;
                    }
                    ast::ExprKind::Application(f, _) => match &f.kind {
                        ast::ExprKind::Abstraction(None, rest) => expr = rest,
                        _ => break,
                    },
                    _ => break,
                }
            }
            if !spans.is_empty() {
                let bindings = type_checker.bindings();
                for span in spans {
                    if let Some((_, scheme)) = bindings.iter().find(|(sp, _)| *sp == span) {
                        println!("{} : {scheme}", &source[span.start..span.end]);
                    }
                }
            }
            println!("- : {sch}");
        }
        _ => {}
    }

    if args.interpret {
        let interpreted = interpret(&elaborated_program);
        println!("{interpreted}");
    }

    if args.codegen {
        let context = Context::create();
        let mut compiler =
            Compiler::new(&context, &args.path).map_err(|e| report(e, &args.path, &source))?;
        compiler
            .compile(&elaborated_program)
            .map_err(|e| report(e, &args.path, &source))?;
        print!("{}", compiler.ir());
    }

    Ok(())
}

fn report<E: Diagnostic + Send + Sync + 'static>(e: E, path: &str, source: &str) -> Report {
    Report::new(e).with_source_code(NamedSource::new(path, source.to_string()))
}
