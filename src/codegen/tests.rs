use super::*;
use crate::{elaborate::surface_to_core, interpret::interpret, parser::Parser};
use inkwell::{
    OptimizationLevel,
    targets::{InitializationConfig, Target},
};
use std::sync::{Mutex, PoisonError};

unsafe extern "C" {
    static mut miniml_args: *mut u64;
    fn miniml_run(entry: unsafe extern "C" fn() -> i64) -> i64;
    fn miniml_reset();
    fn miniml_alloc(words: u64, tag: u64) -> *mut u8;
    fn miniml_equal(left: u64, right: u64) -> u64;
    fn miniml_string_concat(left: u64, right: u64) -> u64;
    fn miniml_print_string(s: u64);
    fn miniml_len(s: u64) -> u64;
    fn miniml_unit_to_str() -> u64;
    fn miniml_bool_to_str(b: u64) -> u64;
    fn miniml_char_to_str(c: u64) -> u64;
    fn miniml_int_to_str(n: u64) -> u64;
}

static RUNTIME_LOCK: Mutex<()> = Mutex::new(());

fn run(source: &str) -> i64 {
    // validate and run inkwell JIT
    let program = surface_to_core(Parser::parse(source).unwrap()).unwrap();
    let context = Context::create();
    let mut compiler = Compiler::new(&context, "test").unwrap();
    compiler
        .compile(&program, &ResultKind::ExitCode, "")
        .unwrap();
    if let Err(e) = compiler.module.verify() {
        panic!("invalid IR for {source:?}: {e}\n{}", compiler.ir());
    }

    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let engine = compiler
        .module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    let module = &compiler.module;
    engine.add_global_mapping(
        &module.get_global("miniml_args").unwrap(),
        &raw mut miniml_args as usize,
    );
    engine.add_global_mapping(
        &module.get_function("miniml_alloc").unwrap(),
        miniml_alloc as *const () as usize,
    );
    engine.add_global_mapping(
        &module.get_function("miniml_equal").unwrap(),
        miniml_equal as *const () as usize,
    );
    engine.add_global_mapping(
        &module.get_function("miniml_string_concat").unwrap(),
        miniml_string_concat as *const () as usize,
    );
    engine.add_global_mapping(
        &module.get_function("miniml_print_string").unwrap(),
        miniml_print_string as *const () as usize,
    );
    engine.add_global_mapping(
        &module.get_function("miniml_len").unwrap(),
        miniml_len as *const () as usize,
    );
    engine.add_global_mapping(
        &module.get_function("miniml_unit_to_str").unwrap(),
        miniml_unit_to_str as *const () as usize,
    );
    engine.add_global_mapping(
        &module.get_function("miniml_bool_to_str").unwrap(),
        miniml_bool_to_str as *const () as usize,
    );
    engine.add_global_mapping(
        &module.get_function("miniml_char_to_str").unwrap(),
        miniml_char_to_str as *const () as usize,
    );
    engine.add_global_mapping(
        &module.get_function("miniml_int_to_str").unwrap(),
        miniml_int_to_str as *const () as usize,
    );
    let _guard = RUNTIME_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    unsafe {
        let entry = engine
            .get_function::<unsafe extern "C" fn() -> i64>("miniml_main")
            .unwrap()
            .into_raw();
        let result = miniml_run(entry);
        miniml_reset();
        result
    }
}

fn agrees(source: &str) {
    // compare to reference interpreter
    let program = surface_to_core(Parser::parse(source).unwrap()).unwrap();
    let expected = interpret(&program).to_string();
    assert_eq!(run(source).to_string(), expected, "{source}");
}

#[test]
fn compiles_expressions() {
    agrees("54321");
    agrees("-54321");
    agrees("1 + 2 * 3 - 4");
    agrees("(fun x => x) (1 + 2)");
    agrees("(fun x => x + 2) 1");
    agrees("(fun x => x) (fun x => fun y => x) 1 2");
    agrees("(fun x => fun y => x) (fun x => x) (fun x => x) 5");
    agrees("(fun x => fun y => x) (1 + 2) 99");
    agrees("(fun twice => twice (fun n => n * 3) 5) (fun f => fun x => f (f x))");
    agrees("(fun x => fun y => fun z => x z (y z)) (fun x => fun _ => x) (fun x => fun _ => x) 7");
    agrees("let x = 1; x + x");
    agrees("let x = 1; let y = 2; x - y");
    agrees("let x = 1; let x = x + 10; x");
    agrees("let k = fun x => fun y => x; let f = k 1; f 2");
    agrees("(fun x => { let y = x + 1; y * 2 }) 3");
    agrees("let a = 1; 10 * { let b = 2; b } + a");
    agrees("let rec f = fun x => x + 1; f 2");
    agrees("let rec f = fun x => fun y => x; f 1 2");
    agrees(
        "let rec f = fun b => b (fun _ => 1) (fun _ => 1 + f (fun x => fun y => x)) 0;
        f (fun x => fun y => y)",
    );
    agrees(
        "let rec count = fun n => n (fun _ => 0) (fun _ => 1 + count (fun x => fun y => x)) 0;
        (fun x => { let rec g = fun y => x + y; g 10 }) (count (fun x => fun y => y))",
    );
    agrees("if 1 < 2 then 3 else 4");
    agrees("(if false then 1 else 2) + 10");
    agrees("(fun x => if x then 1 else 2) false");
    agrees("if if true then false else true then 1 else 2");
    agrees("(if 1 < 2 then { let x = 5; x } else 0) + { let y = 1; y }");
    agrees("let rec f = fun n => if n == 0 then 1 else n * f (n - 1); f 3");
    agrees("let rec f = fun n => if n == 0 then 1 else n * f (n - 1); f 5");
    agrees(
        "let rec sum = fun acc => fun n => if n == 0 then acc else sum (acc + n) (n - 1);
        sum 0 100",
    );
    agrees("if true && 2 < 1 then 1 else 2");
    agrees("if false || 1 < 2 then 1 else 2");
    agrees("let x = 2; if x > 0 && (x < 10 || x == 100) then x else 0");
    agrees("(fun b => if b && b || !b then 1 else 2) false");
    // short-circuit
    agrees("if false && (1 2) == 0 then 1 else 2");
    agrees("if true || ((fun x => x x) (fun x => x x)) == 0 then 1 else 2");
    // print. output isn't checked
    agrees("print \"3\"; 4");
    agrees("let p = print; p \"1\"; p \"2\"; 5");
    agrees("(fun f => { f \"6\"; 7 }) print");
    // equality, written as if else because true != 1
    agrees("if () == () then 1 else 2");
    agrees("if true == true then 1 else 2");
    agrees("if true == false then 1 else 2");
    agrees("if 'a' == 'a' then 1 else 2");
    agrees("if 'a' == 'b' then 1 else 2");
    agrees("if 123 == 123 then 1 else 2");
    agrees("if \"abc\" == \"abc\" then 1 else 2");
    agrees("if \"abc\" == \"def\" then 1 else 2");
    agrees("if (1 + 1, \"a\" ++ \"b\") == (2, \"ab\") then 1 else 2");
    agrees("if (1 + 1, \"a\" ++ \"b\") == (3, \"ab\") then 1 else 2");
    agrees("if (1 + 1, \"a\" ++ \"b\") == (1, \"ac\") then 1 else 2");
    agrees("if ((1, \"a\"), ((), true, 'c')) == ((1, \"a\"), ((), true, 'c')) then 1 else 2");
    agrees("let f = fun x => (x, x); if f 3 == (3, 3) then 1 else 2");
    agrees("let t = (1, 2); let g = fun _ => t; if g () == (1, 2) then 1 else 2");
    // string concat
    agrees("if \"\" ++ \"\" == \"\" then 1 else 2");
    agrees("if \"abcdefg\" ++ \"h\" == \"abcdefgh\" then 1 else 2"); // combined string is 8 bytes
    agrees("if \"abc\" ++ \"def\" ++ \"ghi\" == \"abcdefghi\" then 1 else 2");
    agrees("if \"ab\" ++ \"c\" == \"abd\" then 1 else 2");
    // destructuring
    agrees("let x = 1; x");
    agrees("let (x, y) = (1, 2); x + y");
    agrees(
        "let a = 1; let b = 2; let ((x, y), (z, w), (u, v)) = ((1, 2), (3, 4), (a, b)); x + y * z - w + u - v",
    );
    agrees("(fun a => let (x, y) = (1, 2); a + x * y) 10");
    agrees("let f = fun p => let (x, (y, z)) = p; x - y * z; f (10, (2, 3))");
    // match
    agrees("match (1, 2) { (x, y) => x + y }");
    agrees(
        "let y = 10; match (1, (2, true)) { (1, (_, false)) => 0, (x, (2, true)) => x + y, _ => 99 }",
    );
    agrees("match (\"a\", 3) { (\"b\", _) => 0, (\"a\", n) => n * 2, _ => 7 }");
    agrees(
        "let f = match { (true, x) => x, (false, _) => 'z' };
        if (f (true, 'a'), f (false, 'b')) == ('a', 'z') then 1 else 2",
    );
    agrees("let rec fib = match { 0 => 0, 1 => 1, n => fib (n - 1) + fib (n - 2) }; fib 20");
    agrees(
        "let g = fun p => match p { (1, (_, b)) => b, (x, (2, _)) => x == 1, (x, (y, true)) => x == y, _ => false };
        if (g (1, (5, true)), g (3, (2, false)), g (4, (4, true)), g (4, (5, false))) == (true, false, true, false) then 1 else 2",
    );
    // builtins
    agrees("if len \"abcd\" == 4 then 1 else 2");
    agrees("if len \"abcd\" == 5 then 1 else 2");
    agrees("if unitToStr () == \"()\" then 1 else 2");
    agrees("if boolToStr true == \"true\" then 1 else 2");
    agrees("if boolToStr false == \"false\" then 1 else 2");
    agrees("if charToStr 'a' == \"a\" then 1 else 2");
    agrees("if charToStr 'ç' == \"ç\" then 1 else 2");
    agrees("if intToStr 123 == \"123\" then 1 else 2");
}

#[test]
fn unknown_variable() {
    let program = surface_to_core(Parser::parse("1 + foo").unwrap()).unwrap();
    let context = Context::create();
    let mut compiler = Compiler::new(&context, "test").unwrap();
    assert!(matches!(
        compiler.compile(&program, &ResultKind::ExitCode, ""),
        Err(CodegenError::UnknownVariable { name, .. }) if name == "foo"
    ));
}

#[test]
fn non_exhaustive_match() {
    let program = surface_to_core(Parser::parse("match 1 { 2 => 3 }").unwrap()).unwrap();
    let context = Context::create();
    let mut compiler = Compiler::new(&context, "test").unwrap();
    assert!(matches!(
        compiler.compile(&program, &ResultKind::ExitCode, ""),
        Err(CodegenError::PatternMatch(
            PatternMatchError::NonExhaustive { .. }
        ))
    ));
}

#[test]
fn deep_tail_recursion() {
    // too deep for the interpreter
    assert_eq!(
        run(
            "let rec sum = fun acc => fun n => if n == 0 then acc else sum (acc + n) (n - 1);
            sum 0 1000000"
        ),
        500_000_500_000
    );
}
