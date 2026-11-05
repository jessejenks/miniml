mod ast;
mod codegen;
mod elaborate;
mod interpret;
mod lexer;
mod parser;
mod surface;

use crate::codegen::Compiler;
use crate::elaborate::surface_to_core;
use crate::interpret::interpret;
use crate::lexer::Lexer;
use crate::parser::Parser;
use clap::Parser as _;
use inkwell::context::Context;
use miette::{NamedSource, Report};
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
                    return Err(Report::new(e)
                        .with_source_code(NamedSource::new(&args.path, source.clone())));
                }
                Ok((start, tok, end)) => println!("{tok:?} ({start}, {end})"),
            }
        }
        return Ok(());
    }

    let program = Parser::parse(&source).map_err(|e| {
        Report::new(e).with_source_code(NamedSource::new(&args.path, source.clone()))
    })?;
    if args.parse {
        println!("{program}");
        return Ok(());
    }

    let elaborated_program = surface_to_core(program);
    if args.elab {
        println!("{elaborated_program}");
        return Ok(());
    }

    if args.interpret {
        let interpreted = interpret(&elaborated_program);
        println!("{interpreted}");
    }

    if args.codegen {
        let context = Context::create();
        let with_source =
            |e| Report::new(e).with_source_code(NamedSource::new(&args.path, source.clone()));
        let mut compiler = Compiler::new(&context, &args.path).map_err(with_source)?;
        compiler.compile(&elaborated_program).map_err(with_source)?;
        print!("{}", compiler.ir());
    }

    Ok(())
}
