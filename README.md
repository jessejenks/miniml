# miniml

![Tests](https://github.com/jessejenks/miniml/actions/workflows/test.yml/badge.svg)

A small ML-style language compiled with the ZINC abstract machine's scheme and LLVM.

```sml
# factorial function
let rec fact = fun n => {
    if n < 2
        then 1
        else n * fact (n - 1)
}

let result = fact 5;

print result;

result
```

## Highlights

This is a simple functional language inspired by CAML that compiles to native executables through LLVM using
`inkwell`[^1].

The compiler uses the ZINC[^2] abstract machine's compilation scheme, and is based on my previous ZINC project[^3].

It features:

- A Hindley-Milner type checker with SML style equality types[^4] for `==` comparison
- Proper tails calls with `tailcc` and `musttail`
- A [small C runtime](runtime/runtime.c) with a non-moving mark-sweep garbage collector

Compared to the initial runtime (`5707e42`), I've already improved performance of benchmark programs at `-O0` to

- make ~64% fewer allocations,
- use ~97% less (peak) memory, and
- run ~6.3x faster.

See [the benchmarks section](#benchmarks) below for more details.

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
$CC fact.o runtime/runtime.c -o fact
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
peak heap: 192
collections: 0
max live bytes: 0
final live bytes: 0
gc seconds: 0.000
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

It should look something like this

```llvm
; ModuleID = 'fact.mml'
source_filename = "fact.mml"

%list = type { i64, ptr }
%closure = type { ptr, ptr }

@miniml_args = external global ptr
; ...

declare ptr @miniml_alloc(i64)

declare i32 @printf(ptr, ...)
; ...

define internal tailcc i64 @print(ptr %env) {
entry:
  ; ...
}

define i64 @miniml_main() {
entry:
  ; ...
  ret i64 0
}

; ...
```

## Running the LLVM IR

If you want to compile from the generated LLVM directly, you'll need to include the runtime as well.

```sh
miniml fact.mml --codegen > fact.ll
```

You can compile the runtime to LLVM and use `lli`

```sh
$CC -S -emit-llvm runtime/runtime.c -o runtime.ll
lli --extra-module=runtime.ll fact.ll
```

Or directly compile them together

```sh
$CC fact.ll runtime/runtime.c -o fact
./fact
```

## Internals

Values are 64-bit words with 63-bit tagged integers or pointers to closures.

Like my existing WebAssembly version[^3], the environment is a heap-allocated list, and closures are pairs of code
pointers and environments. But this project includes several optimizations, and manages its own garbage collection. The
WASM version relies on the GC extension.

In section 3.3.2 of The ZINC Experiment[^2], Leroy discusses how using multiple application instead of binary
application to collect arguments will prevent unnecessary closures from being created.

### Benchmarks

The linked-list argument stack with a bump-pointer array dropped the number of allocations significantly. This reduces
the number of allocations required at all, but to make more efficient use of memory requires memory management.

All values are percent reduction relative to `5707e42`. A `-` indicates no change in allocations.

Benchmark numbers come from statistics captured in the runtime and from hyperfine[^5] version 2.0.0. This table only
compares programs compiled with `-O0`.

| file            | commit    | allocations | peak memory | wall time |
| --------------- | --------- | ----------: | ----------: | --------: |
| `closures.mml`  | `fc1e20b` |      20.02% |      19.88% |    17.17% |
|                 | `ad2056d` |      60.01% |      59.53% |    54.26% |
|                 | `5520f05` |           - |      60.23% |    82.07% |
|                 | `c348bb2` |           - |      60.04% |    81.91% |
|                 | `b8044f3` |           - |      98.66% |    82.29% |
|                 |           |             |             |           |
| `curried.mml`   | `fc1e20b` |      35.29% |      35.17% |    36.14% |
|                 | `ad2056d` |      70.59% |      70.31% |    69.03% |
|                 | `5520f05` |           - |      70.80% |    88.76% |
|                 | `c348bb2` |           - |      70.61% |    89.17% |
|                 | `b8044f3` |           - |      99.24% |    89.54% |
|                 |           |             |             |           |
| `fib.mml`       | `fc1e20b` |       0.00% |      -0.33% |   -16.17% |
|                 | `ad2056d` |      66.67% |      64.26% |    62.03% |
|                 | `5520f05` |           - |      64.92% |    80.45% |
|                 | `c348bb2` |           - |      64.59% |    80.08% |
|                 | `b8044f3` |           - |      93.44% |    81.20% |
|                 |           |             |             |           |
| `garbage.mml`   | `fc1e20b` |      18.18% |      18.07% |    19.31% |
|                 | `ad2056d` |      54.55% |      54.20% |    52.43% |
|                 | `5520f05` |           - |      54.95% |    85.69% |
|                 | `c348bb2` |           - |      54.66% |    84.92% |
|                 | `b8044f3` |           - |      98.83% |    86.19% |
|                 |           |             |             |           |
| `recursion.mml` | `fc1e20b` |       0.01% |       0.00% |     0.44% |
|                 | `ad2056d` |      66.67% |      64.18% |    54.07% |
|                 | `5520f05` |           - |      64.60% |    73.85% |
|                 | `c348bb2` |           - |      64.39% |    73.41% |
|                 | `b8044f3` |           - |      94.00% |    72.09% |

## Future Work

- [x] garbage collector
- [ ] Strings
- [ ] Tuple types
- [ ] Algebraic data types
- [ ] Pattern matching

---

[^1]: The Inkwell crate: https://github.com/TheDan64/inkwell

[^2]: _The Zinc Experiment_ ([source](https://xavierleroy.org/publi/ZINC.pdf))

[^3]: My previous ZINC WebAssembly compiler and explainer: https://github.com/jessejenks/ZINC

[^4]: _The Definition of Standard ML Revised_ "Types and Type Functions"
    ([source](https://github.com/SMLFamily/The-Definition-of-Standard-ML-Revised/blob/c4212f434cafd06669171619f7d332341a5b31f3/statcor.tex#L241))

[^5]: Hyperfine benchmarking tool: https://github.com/sharkdp/hyperfine
