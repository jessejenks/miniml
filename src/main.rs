mod ast;
mod codegen;
mod elaborate;
mod interpret;
mod lexer;
mod parser;
mod surface;
mod typecheck;

use crate::codegen::{Compiler, ResultKind};
use crate::elaborate::surface_to_core;
use crate::interpret::interpret;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::typecheck::TypeChecker;
use clap::Parser as _;
use inkwell::context::Context;
use miette::{Diagnostic, NamedSource, Report};
use std::path::{Path, PathBuf};

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
    #[arg(long, visible_alias = "emit-llvm")]
    codegen: bool,
    /// Write assembly to `<output>.s`
    #[arg(short = 'S')]
    assembly: bool,
    /// Write an object file to `<output>.o`
    #[arg(short = 'c')]
    object: bool,
    /// Optimize the generated LLVM
    #[arg(short = 'O', value_parser = clap::value_parser!(u8).range(0..=3))]
    optimize: Option<u8>,
    /// Compile to a native executable at this path
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    /// Target triple, expects same format as `clang` [default: host]
    #[arg(long)]
    target: Option<String>,
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
    add_builtins(&mut type_checker);
    let type_check_result = type_checker.infer(&elaborated_program);
    let (result_kind, result_label) = match type_check_result {
        Err(e) => {
            let rep = report(e, &args.path, &source);
            if !args.force {
                return Err(rep);
            }
            eprintln!("{rep:?}");
            (ResultKind::ExitCode, String::new())
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
            return Ok(());
        }
        Ok(sch) => (result_kind(sch.tp()), format!("- : {sch} = ")),
    };

    if args.interpret {
        let interpreted = interpret(&elaborated_program);
        println!("{result_label}{interpreted}");
        return Ok(());
    }

    let context = Context::create();
    let mut compiler =
        Compiler::new(&context, &args.path).map_err(|e| report(e, &args.path, &source))?;
    if let Some(triple) = &args.target {
        compiler.set_target(triple.clone());
    }
    compiler
        .compile(&elaborated_program, result_kind, &result_label)
        .map_err(|e| report(e, &args.path, &source))?;
    if let Some(level) = args.optimize {
        compiler
            .optimize(level)
            .map_err(|e| report(e, &args.path, &source))?;
    }
    if args.codegen {
        print!("{}", compiler.ir());
        return Ok(());
    }

    let output = args.output.clone().unwrap_or_else(|| default_output(input));
    if args.assembly {
        return compiler
            .write_assembly(&output.with_extension("s"))
            .map_err(|e| report(e, &args.path, &source));
    }
    if args.object {
        return compiler
            .write_object(&output.with_extension("o"))
            .map_err(|e| report(e, &args.path, &source));
    }
    compiler
        .build_executable(&output)
        .map_err(|e| report(e, &args.path, &source))
}

fn result_kind(tp: &typecheck::TypeExpr) -> ResultKind {
    match tp {
        typecheck::TypeExpr::Int => ResultKind::Int,
        typecheck::TypeExpr::Bool => ResultKind::Bool,
        typecheck::TypeExpr::Unit => ResultKind::Unit,
        typecheck::TypeExpr::Arrow(..) => ResultKind::Function,
        typecheck::TypeExpr::Var(..) | typecheck::TypeExpr::UnifVar(_) => ResultKind::Never,
        _ => todo!(),
    }
}

fn default_output(input: &Path) -> PathBuf {
    let stem = input.with_extension("");
    if stem == input {
        PathBuf::from("a.out")
    } else {
        stem
    }
}

fn add_builtins(type_checker: &mut typecheck::TypeChecker) {
    type_checker.set_env(
        "print".to_string(),
        typecheck::Scheme::as_monotype(typecheck::TypeExpr::arrow(
            typecheck::TypeExpr::String,
            typecheck::TypeExpr::Unit,
        )),
    );
    type_checker.set_env(
        "len".to_string(),
        typecheck::Scheme::as_monotype(typecheck::TypeExpr::arrow(
            typecheck::TypeExpr::String,
            typecheck::TypeExpr::Int,
        )),
    );
    type_checker.set_env(
        "unitToStr".to_string(),
        typecheck::Scheme::as_monotype(typecheck::TypeExpr::arrow(
            typecheck::TypeExpr::Unit,
            typecheck::TypeExpr::String,
        )),
    );
    type_checker.set_env(
        "boolToStr".to_string(),
        typecheck::Scheme::as_monotype(typecheck::TypeExpr::arrow(
            typecheck::TypeExpr::Bool,
            typecheck::TypeExpr::String,
        )),
    );
    type_checker.set_env(
        "charToStr".to_string(),
        typecheck::Scheme::as_monotype(typecheck::TypeExpr::arrow(
            typecheck::TypeExpr::Char,
            typecheck::TypeExpr::String,
        )),
    );
    type_checker.set_env(
        "intToStr".to_string(),
        typecheck::Scheme::as_monotype(typecheck::TypeExpr::arrow(
            typecheck::TypeExpr::Int,
            typecheck::TypeExpr::String,
        )),
    );
}

fn report<E: Diagnostic + Send + Sync + 'static>(e: E, path: &str, source: &str) -> Report {
    Report::new(e).with_source_code(NamedSource::new(path, source.to_string()))
}
