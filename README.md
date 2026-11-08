# miniml

![Tests](https://github.com/jessejenks/miniml/actions/workflows/test.yml/badge.svg)

A small ML-style language that compiles to LLVM IR.

```
# factorial function
let rec fact = fun n =>
    if n < 2
        then 1
        else n * fact (n - 1);

let result = fact 5;

print result;

result
```

## Highlights

This is a simple functional language inspired by CAML. It features a Hindley-Milner type checker with SML style equality
types[^1] for `==` comparison. The compiler uses the ZINC abstract machine's compilation scheme, and is based on my
previous ZINC project[^2].

The compiler uses `inkwell`[^3] to write LLVM.

Because the ZINC compilation scheme already tracks tail position, the compiler can use proper `tailcc` with `musttail`.

Values are 64-bit words with 63-bit tagged integers or pointers to closures.

Like my existing WebAssembly version, the environment is a heap-allocated list, and closures are pairs of code pointers
and environments.

Generated code allocates through a small C runtime ([`runtime/runtime.c`](runtime/runtime.c)), which is linked into
every executable. It currently wraps `malloc` and counts allocations.

## Usage

### Creating an Executable

```sh
miniml fact.mml
./fact
```

prints `120` to stdout and then displays the final computed value.

```
120
- : Int = 120
```

### Options

All options can be seen with `--help`, but they mostly match `clang` and `gcc`.

```sh
miniml fact.mml --target x86_64-pc-windows-msvc -c -O2
```

will write a COFF object file at `fact.o`.

Object files (`-c`) and assembly (`-S`) don't include the runtime, so link it yourself.

```sh
miniml fact.mml -c
clang fact.o runtime/runtime.c -o fact
```

### Allocation Stats

Set `MINIML_STATS` to print allocation counts to stderr when the program exits.

```sh
MINIML_STATS=1 ./fact
```

```
120
- : Int = 120
allocations: 26
allocated bytes: 416
```

## Building From Source

This project can be built like any normal Rust project

```sh
cargo build
```

The backend uses `inkwell` 0.11, which supports LLVM 12 through 23, but this project assumes LLVM 21.1.

If LLVM 21 isn't the `llvm-config` on your `PATH`, use the `LLVM_SYS_211_PREFIX` variable.

```sh
LLVM_SYS_211_PREFIX=/path/to/llvm cargo build
```

## Stages

### Lexer

Debugging the lexer.

```sh
miniml fact.mml --lex
```

Just prints each token and its location in the file

```
Keyword(Let) (21, 24)  # `let` keyword at input indices 21 through 24, non-inclusive
Keyword(Rec) (25, 28)
Identifier("fact") (29, 33)
Equal (34, 35)
... etc
```

### Parser

```sh
miniml fact.mml --parse
```

outputs fully parenthesized expression.

```
(let rec fact = (fun n => (if (n < 2) then 1 else (n * (fact (n - 1))))); (let result = (fact 5); ((print result); result)))
```

### Elaborator

```sh
miniml fact.mml --elab
```

Elaborates to a core syntax with de Bruijn indices, and printed in s-expression like format.

```
(let rec (λ.(if (< #0 2) then 1 else (* #0 (#1 (- #0 1))))) in (let (#0 5) in ((λ.#1) (print #0))))
```

Multi-parameter functions are also elaborated to curried functions, so `fun x, y, z => z` is equivalent to
`fun x => fun y => fun z => z`, and both elaborate to `(λ.(λ.(λ.#0)))`.

A function with no arguments like `fun => 1` is equivalent to `fun _ => 1`, which elaborates to `(λ.1)`.

Sequences are derived forms, and are elaborated to lambdas.

`1; 2` elaborates to `((λ.2) 1)`.

### Type Checker

```sh
miniml fact.mml --types
miniml fact.mml --interface
miniml fact.mml -i
```

Prints inferred top-level declarations and type of entire program.

```
fact : Int -> Int
result : Int
- : Int
```

Uses Hindley-Milner type checking. So

```
let id = fun x => x;
let unit = id ();
let bool = id true;
let int = id 1;
let func = id (fun y, z => z);
```

will output

```sh
id : (∀α.α -> α)
unit : Unit
bool : Bool
int : Int
func : (∀α.∀β.α -> β -> β)
- : Unit
```

### Interpreter

This project includes a reference call-by-value interpreter.

```sh
miniml fact.mml --interpret
```

(The example program prints the result before returning).

```
120
- : Int = 120
```

### Code Generation

```sh
miniml fact.mml --codegen
miniml fact.mml --codegen -O2 # to run LLVM -O2
```

Prints LLVM IR to stdout.

```llvm
; ModuleID = 'fact.mml'
source_filename = "fact.mml"

%list = type { i64, ptr }
%closure = type { ptr, ptr }

@ARGS = internal global ptr null
@print.format = private unnamed_addr constant [6 x i8] c"%lld\0A\00", align 1

declare ptr @miniml_alloc(i64)

declare i32 @printf(ptr, ...)
...

define internal tailcc i64 @print(ptr %env) {
entry:
  ...
}

define i64 @main() {
entry:
  call void @miniml_init()
  %env.slot = alloca ptr, align 8
  ...
  ret i64 %unboxed
}

...
```

## Running the LLVM IR

The examples below expect the LLVM 21 tools (`lli`, `opt`, `llc`, `clang`). Older versions may not be able to read the
generated IR.

```sh
miniml fact.mml --codegen > fact.ll
```

The IR only declares `miniml_init` and `miniml_alloc`, so the runtime has to come along. Interpret it by compiling the
runtime to IR as well:

```sh
clang -S -emit-llvm runtime/runtime.c -o runtime.ll
lli --extra-module=runtime.ll fact.ll
```

```
120
- : Int = 120
```

Or compile it with the runtime:

```sh
clang fact.ll runtime/runtime.c -o fact
```

Note that the way the final value is displayed differs from the interpreter. In the interpreter, we have access to the entire term, so running `--interpret` on a program like this

```
fun => 1
```

will print the actual term

```
- : (∀α.α -> Int) = (λ.1)
```

while the compiled version will print

```
- : (∀α.α -> Int) = <fun>
```

## Future Work

- A proper garbage collector (memory is currently never freed)
- Strings
- Tuple types
- Algebraic data types
- Pattern matching

---

[^1]: _The Definition of Standard ML Revised_ "Types and Type Functions"
    ([source](https://github.com/SMLFamily/The-Definition-of-Standard-ML-Revised/blob/c4212f434cafd06669171619f7d332341a5b31f3/statcor.tex#L241))

[^2]: My previous ZINC WebAssembly compiler and explainer: https://github.com/jessejenks/ZINC

[^3]: The Inkwell crate: https://github.com/TheDan64/inkwell
