# The miniml language

A short tour of the language. Every example here can be saved to a file and run with `miniml file.mml`.

A program is a single expression, optionally preceded by type declarations. Its value is displayed when the program
finishes.

## Comments

Line comments start with `#`

```
# this is a comment
1 + 2 # so is this
```

## Values and operators

| Type     | Examples             |
| -------- | -------------------- |
| `Unit`   | `()`                 |
| `Bool`   | `true`, `false`      |
| `Char`   | `'a'`, `'ç'`, `'\n'` |
| `Int`    | `0`, `42`, `-7`      |
| `String` | `"hello"`            |

Integers are 63-bit.

| Operators              | Types                     |
| ---------------------- | ------------------------- |
| `+` `-` `*` `/`        | `Int`                     |
| `&` `\|` `~` (bitwise) | `Int`                     |
| `<` `>` `<=` `>=`      | `Int -> Int -> Bool`      |
| `==` `!=`              | see [equality](#equality) |
| `&&` `\|\|` `!`        | `Bool`                    |
| `++` (concatenation)   | `String`                  |

`&&` and `||` short-circuit.

## Let bindings and sequences

`let` binds a name for the rest of the expression after the `;`.

```sml
let x = 1;
let y = x + 1;
x + y
```

`a; b` evaluates `a`, then `b`, and has the value of `b`. Braces group a sequence into a single expression.

```sml
let z = {
    let a = 2;
    a * a
};
z
```

A `;` is optional after a closing `}`.

## Functions

Functions are written with `fun`, and applied by juxtaposition.

```sml
let add = fun x, y => x + y;
add 1 2
```

`fun x, y => ...` is the same as `fun x => fun y => ...`, so functions can be partially applied.

```sml
let add = fun x, y => x + y;
let inc = add 1;
inc 41
```

A function with no parameters, `fun => ...`, is the same as `fun _ => ...`, and is called with `()`.

```sml
let greet = fun => "hello";
greet ()
```

`|>` and `<|` apply a function, so `x |> f` and `f <| x` are both `f x`.

```sml
let double = fun n => n * 2;
3 |> double |> intToStr
```

Recursive functions use `let rec`.

```sml
let rec factorial = fun n => if n == 0 then 1 else n * factorial (n - 1);
factorial 10
```

## Tuples

```sml
let pair = (1, "one");
let (n, s) = pair;
s ++ intToStr n
```

`let` can destructure tuples, but can't contain literals or constructors, since those could fail to match. Use
[`match`](#pattern-matching) instead.

## Equality

`==` and `!=` compare values structurally. Like SML's equality types, they're only allowed on types that can be
compared, which excludes functions.

```sml
(1, "a") == (1, "a")
```

```sml
# type error: functions can't be compared
(fun x => x) == (fun x => x)
```

## Algebraic data types

Types are declared before the program's expression. Constructors are written with their type's name, and take their
fields as arguments.

```sml
type Shape
    = Circle Int
    | Rect Int Int;

let area = fun s => match s {
    Shape::Circle r => 3 * r * r,
    Shape::Rect w h => w * h,
};

area (Shape::Rect 3 4)
```

Types can take parameters, and can be recursive.

```sml
type List a
    = Nil
    | Cons a (List a);

List::Cons 1 (List::Cons 2 List::Nil)
```

displays

```
- : List Int = (List::Cons 1 (List::Cons 2 List::Nil))
```

Constructors are functions, so they can be partially applied, like `List::Cons 1`.

## Pattern matching

`match` compares a value against a list of patterns, and evaluates the first arm that matches. Arms are separated by
commas, and a trailing comma is optional.

```sml
let describe = fun n => match n {
    0 => "zero",
    1 => "one",
    _ => "many",
};
describe 1
```

Patterns can be:

- literals, like `0`, `"a"`, or `true`
- a wildcard `_`
- a variable
- tuples of patterns, like `(x, 0)`
- constructors applied to patterns, like `List::Cons x rest`

```sml
type List a
    = Nil
    | Cons a (List a);

let rec sumPairs = fun xs => match xs {
    List::Cons a (List::Cons b rest) => a + b + sumPairs rest,
    List::Cons a List::Nil => a,
    List::Nil => 0,
};

sumPairs (List::Cons 1 (List::Cons 2 (List::Cons 3 List::Nil)))
```

`match` without a value is a function of one argument, so `match { ... }` is the same as `fun x => match x { ... }`.

```sml
type Tree a
    = Leaf
    | Node (Tree a) a (Tree a);

let rec insert = fun x => match {
    Tree::Leaf => Tree::Node Tree::Leaf x Tree::Leaf,
    Tree::Node left v right =>
        if x < v then Tree::Node (insert x left) v right
        else Tree::Node left v (insert x right),
}

insert 5 (insert 2 (insert 8 Tree::Leaf))
```

displays

```
- : Tree Int = (Tree::Node (Tree::Node Tree::Leaf 2 (Tree::Node Tree::Leaf 5 Tree::Leaf)) 8 Tree::Leaf)
```

A `match` that doesn't cover every value is a compile error.

```sml
match 1 { 0 => "zero" }
```

```
Error: patternmatch::non_exhaustive

  × non-exhaustive match
   ╭─[example.mml:1:1]
 1 │ match 1 { 0 => "zero" }
   · ───────────┬───────────
   ·            ╰── missing case
 2 │ 
   ╰────
```

## Built-in functions

| Function    | Type             |
| ----------- | ---------------- |
| `print`     | `String -> Unit` |
| `len`       | `String -> Int`  |
| `unitToStr` | `Unit -> String` |
| `boolToStr` | `Bool -> String` |
| `charToStr` | `Char -> String` |
| `intToStr`  | `Int -> String`  |
