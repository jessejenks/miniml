# miniml

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

This is a combination of a variety of previous projects, primarily my [ZINC compiler](https://github.com/jessejenks/ZINC).

## Building

The backend uses [inkwell](https://github.com/TheDan64/inkwell) 0.11, which supports LLVM 12 through 23. This project assumes LLVM 21.1.

```sh
cargo build
```

If LLVM 21 isn't the `llvm-config` on your `PATH`, use the `LLVM_SYS_211_PREFIX` variable.

```sh
LLVM_SYS_211_PREFIX=/path/to/llvm cargo build
```

## Stages

### Lexer

Debugging the lexer

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

Multi-parameter functions are also elaborated to curried functions, so `fun x, y, z => z` is equivalent to `fun x => fun y => fun z => z`, and both elaborate to `(λ.(λ.(λ.#0)))`.

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
120
```

### Code Generation

```sh
miniml fact.mml --codegen
```

Prints LLVM IR to stdout.

```llvm
; ModuleID = 'fact.mml'
source_filename = "fact.mml"

%list = type { i64, ptr }
%closure = type { ptr, ptr }

@ARGS = internal global ptr null
@print.format = private unnamed_addr constant [6 x i8] c"%lld\0A\00", align 1

declare ptr @malloc(i64)

declare i32 @printf(ptr, ...)
...

define internal tailcc i64 @print(ptr %env) {
entry:
  ...
}

define i64 @main() {
entry:
  %env.slot = alloca ptr, align 8
  ...
  ret i64 %unboxed
}

...
```


## Running the LLVM IR

The examples below expect the LLVM 21 tools (`lli`, `opt`, `llc`, `clang`). Older versions may not be able to read the generated IR.

```sh
miniml fact.mml --codegen > fact.ll
```

Interpret it:

```sh
lli fact.ll
echo $?
```

Note that this is a key difference from the interpreter: the result of the entire program is always expected to be an integer, so for program like this:

```
fun => 1
```

`--interpret` will print a nice string like `(λ.1)`, while `lli` will output nothing, and `echo $?` will display a random looking number that comes from unboxing the address of the closure.
