# miniml

![Tests](https://github.com/jessejenks/miniml/actions/workflows/test.yml/badge.svg)

A small ML-style language compiled with the ZINC abstract machine's scheme and LLVM.

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
types[^1] for `==` comparison. The compiler uses the ZINC[^2] abstract machine's compilation scheme, and is based on my
previous ZINC project[^3]. It uses `inkwell`[^4] to write LLVM.

Because the ZINC compilation scheme already tracks tail position, the compiler can use proper `tailcc` with `musttail`.

Values are 64-bit words with 63-bit tagged integers or pointers to closures.

Like my existing WebAssembly version, the environment is a heap-allocated list, and closures are pairs of code pointers
and environments.

Generated code allocates through a small C runtime ([`runtime/runtime.c`](runtime/runtime.c)), which is linked into
every executable. It sets up the argument stack with guard pages, and wraps `malloc` to counts allocations. See
[stats section below](#allocation-stats).

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
allocations: 12
allocated bytes: 192
```

## Building From Source

This project can be built like any normal Rust project

```sh
cargo build
```

A C compiler is also required, since `build.rs` compiles the runtime for the tests.

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

```
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

```
120
- : Int = 120
```

Note that one difference between the interpreter and the compiled version is that

```
fun => 1
```

will print the actual term

```
- : (∀α.α -> Int) = (λ.1)
```

instead of

```
- : (∀α.α -> Int) = <fun>
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

@miniml_args = external global ptr
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
  ret i64 0
}

...
```

## Running the LLVM IR

```sh
miniml fact.mml --codegen > fact.ll
```

Then compile the runtime to IR as well:

```sh
clang -S -emit-llvm runtime/runtime.c -o runtime.ll
lli --extra-module=runtime.ll fact.ll
```

```
120
- : Int = 120
```

Or just compile it with the runtime:

```sh
clang fact.ll runtime/runtime.c -o fact
```

## Future Work

- A proper garbage collector (memory is currently never freed)
- Strings
- Tuple types
- Algebraic data types
- Pattern matching

## Benchmarks

I'm still working on implementing GC. However, I have started on optimizations already.

In section 3.3.2 of The ZINC Experiment[^2], Leroy discusses how using multiple application instead of binary
application to collect arguments will prevent unnecessary closures from being created. This change, along with replacing
the linked-list argument stack with a bump-pointer array, dropped the number of allocations in the benchmark files by
55%-70% compared to `5707e42`.

| file                  | allocations removed |
| --------------------- | ------------------- |
| `bench/closures.mml`  | 60.01%              |
| `bench/curried.mml`   | 70.59%              |
| `bench/fib.mml`       | 66.67%              |
| `bench/garbage.mml`   | 54.55%              |
| `bench/recursion.mml` | 66.67%              |

---

[^1]: _The Definition of Standard ML Revised_ "Types and Type Functions"
    ([source](https://github.com/SMLFamily/The-Definition-of-Standard-ML-Revised/blob/c4212f434cafd06669171619f7d332341a5b31f3/statcor.tex#L241))

[^2]: _The Zinc Experiment_ ([source](https://xavierleroy.org/publi/ZINC.pdf))

[^3]: My previous ZINC WebAssembly compiler and explainer: https://github.com/jessejenks/ZINC

[^4]: The Inkwell crate: https://github.com/TheDan64/inkwell
