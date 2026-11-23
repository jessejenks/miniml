# miniml

![Tests](https://github.com/jessejenks/miniml/actions/workflows/test.yml/badge.svg)

A small ML-style language compiled to LLVM using the ZINC abstract machine's scheme.

```sml
# fibonacci with accumulator
let rec fib = fun n, acc => {
    let (a, b) = acc;
    match n {
        0 => a,
        _ => fib (n - 1) (b, a + b)
    }
}

let result = fib 10 (0, 1);

print ("fib 10 = " ++ intToStr result);

result
```

## Highlights

This is a simple functional language inspired by CAML that compiles to native executables through LLVM using the
`inkwell`[^1] crate.

The compiler uses the ZINC[^2] abstract machine's compilation scheme, and is based on my previous ZINC project[^3] and
[blog series](https://jessejenks.github.io/blog/).

It features:

- A Hindley-Milner type checker with SML style equality types[^4] for `==` comparison
- Proper tail calls with `tailcc` and `musttail`
- A [small C runtime](runtime/runtime.c) with a non-moving mark-sweep garbage collector that scans the native stack
  conservatively and the argument stack precisely
- A reference call-by-value interpreter that the compiled code is tested against
- Algebraic data types, which can be polymorphic and recursive
- Pattern matching on literals, tuples, and constructors, compiled to decision trees using Jules Jacobs' algorithm[^5],
  with non-exhaustive matches reported as compile errors

Compared to the initial runtime (`5707e42`), I've already improved performance of benchmark programs at `-O0` to

- make ~64% fewer allocations,
- use ~97% less (peak) memory, and
- run ~4.9x faster.

See [the benchmarks section](#benchmarks) below for more details.

## Documentation

- [The language](docs/language.md): a short tour of the syntax and features, with examples

## Usage

### Creating an Executable

```sh
miniml fib.mml
./fib
```

prints `fib 10 = 55` to stdout and then displays the final computed value.

```
fib 10 = 55
- : Int = 55
```

### Options

All options can be seen with `--help`, but they mostly match `clang` and `gcc`.

```sh
miniml fib.mml --target x86_64-pc-windows-msvc -c -O2
```

will write a COFF object file at `fib.o`.

Object files (`-c`) and assembly (`-S`) don't include the runtime, so link it yourself.

```sh
miniml fib.mml -c
$CC fib.o runtime/runtime.c -o fib
```

### Allocation Stats

Set `MINIML_STATS` to print allocation counts to stderr when the program exits.

```sh
MINIML_STATS=1 ./fib
```

```
fib 10 = 55
- : Int = 55
allocations: 76
allocated bytes: 1816
peak heap: 1816
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
miniml fib.mml --lex
```

Just prints each token and its location in the file

```
Keyword(Let) (29, 32) # span of token
Keyword(Rec) (33, 36)
Identifier("fib") (37, 40)
Equal (41, 42)
Keyword(Fun) (43, 46)
Identifier("n") (47, 48)
Comma (48, 49)
Identifier("acc") (50, 53)
WideArrow (54, 56)
LBrace (57, 58)
... etc
```

### Parser

```sh
miniml fib.mml --parse
```

outputs fully parenthesized expression.

```
(let rec fib = (fun n, acc => (let (a, b) = acc; (match n { 0 => a, _ => ((fib (n - 1)) (b, (a + b))) }))); (let result = ((fib 10) (0, 1)); ((print ("fib 10 = " ++ (intToStr result))); result)))
```

### Elaborator

```sh
miniml fib.mml --elab
```

Elaborates to a core syntax with de Bruijn indices, and printed in s-expression like format.

```
(let rec (λ.(λ.(let #0 in (match #3 { 0 => #1, _ => ((#5 (- #4 1)) (#1, (+ #2 #1))) })))) in (let ((#0 10) (0, 1)) in ((λ.#1) (print (++ "fib 10 = " (intToStr #0))))))
```

Multi-parameter functions are also elaborated to curried functions, so `fun x, y, z => z` is equivalent to
`fun x => fun y => fun z => z`, and both elaborate to `(λ.(λ.(λ.#0)))`.

A function with no arguments like `fun => 1` is equivalent to `fun _ => 1`, which elaborates to `(λ.1)`.

Sequences are derived forms, and are elaborated to lambdas.

`1; 2` elaborates to `((λ.2) 1)`.

### Type Checker

```sh
miniml fib.mml --types
miniml fib.mml --interface
miniml fib.mml -i
```

Prints inferred top-level declarations and type of entire program.

```
fib : Int -> (Int, Int) -> Int
result : Int
- : Int
```

Uses Hindley-Milner type checking.

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

### Pattern Matching

```sh
miniml fib.mml --patterns
```

Prints the decision tree compiled from each `match` expression.

```
source:
match n {
        0 => a,
        _ => fib (n - 1) (b, a + b)
    }

tree:
match# a_0
| 0 =>
  arm 0: 0 => a
| _ =>
  arm 1: _ => fib (n - 1) (b, a + b)
    with _ = a_0
```

This output will always show a tree, but when compiling, a `match` that doesn't cover every case is a compile error.

For example the following program will type check, but the match is not exhaustive.

```
source:
match {
    (x, y, 1) => x + y,
    (x, y, 2) => x - y,
}

tree:
match# a_0
| (a_1, a_2, a_3) =>
  match# a_3
  | 1 =>
    arm 0: (x, y, 1) => x + y
      with x = a_1, y = a_2
  | _ =>
    match# a_3
    | 2 =>
      arm 1: (x, y, 2) => x - y
        with x = a_1, y = a_2
    | _ =>
      ERR
```

### Interpreter

This project includes a reference call-by-value interpreter.

```sh
miniml fib.mml --interpret
```

```
fib 10 = 55
- : Int = 55
```

### Code Generation

```sh
miniml fib.mml --codegen
miniml fib.mml --codegen -O2 # to run LLVM -O2
```

Prints LLVM IR to stdout.

It should look something like this

```llvm
; ModuleID = 'fib.mml'
source_filename = "fib.mml"

%list = type { i64, ptr }
%closure = type { ptr, ptr }

@miniml_args = external global ptr
; ...

declare ptr @miniml_alloc(i64, i64)

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

## Differences Between Interpreted and Compiled Programs

### Stuck Terms

The interpreter tries to be a true call-by-value interpreter, so there are no runtime errors. Stuck terms simply stop
being evaluated.

This is also true of exhaustiveness checking in pattern matching. If a pattern doesn't match, the match expression never
reduces.

Neither of these behaviors is visible with the basic pipeline since the the type and pattern checks are run first. But
you can see this in the tests.

### Printing terms

```
fun => 1
```

The interpreter will print the actual term

```
- : (∀α.α -> Int) = (λ.1)
```

while the compiled output just shows

```
- : (∀α.α -> Int) = <fun>
```

## Running the LLVM IR

If you want to compile from the generated LLVM directly, you'll need to include the runtime as well.

```sh
miniml fib.mml --codegen > fib.ll
```

You can compile the runtime to LLVM and use `lli`

```sh
$CC -S -emit-llvm runtime/runtime.c -o runtime.ll
lli --extra-module=runtime.ll fib.ll
```

Or directly compile them together

```sh
$CC fib.ll runtime/runtime.c -o fib
./fib
```

## Internals

Values are 64-bit words with 63-bit tagged integers or pointers to heap-allocated objects. Following the ZINC paper and
OCaml, each object has a header word.

Like my existing WebAssembly version[^3], the environment is a heap-allocated list, and closures are pairs of code
pointers and environments. But this project includes several optimizations, and manages its own garbage collection.

### Benchmarks

In section 3.3.2 of The ZINC Experiment[^2], Leroy discusses how using multiple application instead of binary
application to collect arguments will prevent unnecessary closures from being created. The linked-list argument stack
with a bump-pointer array dropped the number of allocations significantly. This reduces the number of allocations
required at all, but to make more efficient use of memory requires memory management.

All values are percent reduction relative to `5707e42`. A `-` indicates no change in allocations.

Benchmark numbers come from statistics captured in the runtime and from hyperfine[^6] version 2.0.0. This table only
compares programs compiled with `-O0`.

| file            | commit    | allocations | peak memory | wall time |
| --------------- | --------- | ----------: | ----------: | --------: |
| `closures.mml`  | `fc1e20b` |      20.02% |      19.88% |    17.17% |
|                 | `ad2056d` |      60.01% |      59.53% |    54.26% |
|                 | `5520f05` |           - |      60.23% |    82.07% |
|                 | `c348bb2` |           - |      60.04% |    81.91% |
|                 | `b8044f3` |           - |      98.66% |    82.29% |
|                 | `8cedc21` |           - |      98.66% |    77.05% |
|                 |           |             |             |           |
| `curried.mml`   | `fc1e20b` |      35.29% |      35.17% |    36.14% |
|                 | `ad2056d` |      70.59% |      70.31% |    69.03% |
|                 | `5520f05` |           - |      70.80% |    88.76% |
|                 | `c348bb2` |           - |      70.61% |    89.17% |
|                 | `b8044f3` |           - |      99.24% |    89.54% |
|                 | `8cedc21` |           - |      99.24% |    86.34% |
|                 |           |             |             |           |
| `fib.mml`       | `fc1e20b` |       0.00% |      -0.33% |   -16.17% |
|                 | `ad2056d` |      66.67% |      64.26% |    62.03% |
|                 | `5520f05` |           - |      64.92% |    80.45% |
|                 | `c348bb2` |           - |      64.59% |    80.08% |
|                 | `b8044f3` |           - |      93.44% |    81.20% |
|                 | `8cedc21` |           - |      93.44% |    78.20% |
|                 |           |             |             |           |
| `garbage.mml`   | `fc1e20b` |      18.18% |      18.07% |    19.31% |
|                 | `ad2056d` |      54.55% |      54.20% |    52.43% |
|                 | `5520f05` |           - |      54.95% |    85.69% |
|                 | `c348bb2` |           - |      54.66% |    84.92% |
|                 | `b8044f3` |           - |      98.83% |    86.19% |
|                 | `8cedc21` |           - |      98.83% |    79.70% |
|                 |           |             |             |           |
| `recursion.mml` | `fc1e20b` |       0.01% |       0.00% |     0.44% |
|                 | `ad2056d` |      66.67% |      64.18% |    54.07% |
|                 | `5520f05` |           - |      64.60% |    73.85% |
|                 | `c348bb2` |           - |      64.39% |    73.41% |
|                 | `b8044f3` |           - |      94.00% |    72.09% |
|                 | `8cedc21` |           - |      94.00% |    70.11% |

Notes:

- `8cedc21` is slower because objects now have headers. This takes up more memory and makes gc more complex.

## Future Work

- [x] Garbage collection
- [x] Strings and characters
- [x] Tuple types
- [x] Algebraic data types
- Pattern matching
  - [x] Tuples
  - [x] ADTs
- [ ] Nicer type errors

---

[^1]: The Inkwell crate: https://github.com/TheDan64/inkwell

[^2]: _The Zinc Experiment_ ([source](https://xavierleroy.org/publi/ZINC.pdf))

[^3]: My previous ZINC WebAssembly compiler and explainer: https://github.com/jessejenks/ZINC

[^4]: _The Definition of Standard ML Revised_ "Types and Type Functions"
    ([source](https://github.com/SMLFamily/The-Definition-of-Standard-ML-Revised/blob/c4212f434cafd06669171619f7d332341a5b31f3/statcor.tex#L241))

[^5]: _How to compile pattern matching_, Jules Jacobs (2021)
    ([source](https://julesjacobs.com/notes/patternmatching/patternmatching.pdf))

[^6]: Hyperfine benchmarking tool: https://github.com/sharkdp/hyperfine
