//! Based on [Jules Jacobs' 2021 notes on compiling pattern matching](https://julesjacobs.com/notes/patternmatching/patternmatching.pdf)

use miette::{Diagnostic, SourceSpan};
use std::collections::HashMap;
use std::fmt::Display;
use thiserror::Error;

use crate::{ast, typecheck};

#[derive(Debug, Error, Diagnostic)]
pub enum PatternMatchError {
    #[error("non-exhaustive match")]
    #[diagnostic(code(patternmatch::non_exhaustive))]
    NonExhaustive {
        #[label("missing case")]
        span: SourceSpan,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MatchVar(pub usize);

impl Display for MatchVar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchVar(id) => write!(f, "a_{id}"),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Arm(pub usize);

impl Display for Arm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Arm(index) = self;
        write!(f, "{index}")
    }
}

/// A variable or wildcard, numbered left to right within each pattern
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Binder(usize);

impl Display for Binder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Binder(index) = self;
        write!(f, "x_{index}")
    }
}

pub enum Branch {
    Literal(ast::Literal, Box<Tree>, Option<Box<Tree>>),
    Tuple(Vec<MatchVar>, Box<Tree>),
    Constructor(
        typecheck::TypeId,
        usize,
        Vec<MatchVar>,
        Box<Tree>,
        Option<Box<Tree>>,
    ),
}

impl Display for Branch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Branch::Literal(lit, tree, alternative) => {
                lit.fmt(f)?;
                write!(
                    f,
                    " =>{}{tree}",
                    if matches!(**tree, Tree::Branch(_, _)) {
                        "\n"
                    } else {
                        " "
                    }
                )?;
                if let Some(alternative) = alternative {
                    write!(f, ",\n_ => {alternative}")?;
                }
                Ok(())
            }
            Branch::Tuple(vars, tree) => {
                write!(f, "(")?;
                for (i, v) in vars.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, ") => {tree}")
            }
            Branch::Constructor(id, index, vars, tree, alternative) => {
                write!(f, "T_{}::[{index}]", id.0)?;
                for v in vars {
                    write!(f, " {v}")?;
                }
                write!(f, " => {tree}")?;
                if let Some(alternative) = alternative {
                    write!(f, ",\n_ => {alternative}")?;
                }
                Ok(())
            }
        }
    }
}

pub enum Tree {
    Err,
    Leaf(Arm, Vec<MatchVar>),
    Branch(MatchVar, Branch),
}

impl Display for Tree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tree::Err => write!(f, "ERR"),
            Tree::Leaf(arm, bindings) => {
                write!(f, "{arm} [")?;
                for (i, var) in bindings.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{var}")?;
                }
                write!(f, "]")
            }
            Tree::Branch(x, branch) => write!(f, "match# {x} {{\n{branch}}}"),
        }
    }
}

#[derive(Clone)]
enum Pattern {
    Bind(Binder),
    Literal(ast::Literal),
    Tuple(Vec<Pattern>),
    Constructor(typecheck::TypeId, usize, Vec<Pattern>),
}

impl Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pattern::Bind(binder) => binder.fmt(f),
            Pattern::Literal(lit) => lit.fmt(f),
            Pattern::Tuple(patterns) => {
                write!(f, "(")?;
                for (i, p) in patterns.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ")")
            }
            Pattern::Constructor(id, index, patterns) => {
                write!(f, "T_{}::[{index}]", id.0)?;
                for p in patterns {
                    match p {
                        Self::Constructor(_, _, args) if !args.is_empty() => write!(f, " ({p})")?,
                        _ => write!(f, " {p}")?,
                    }
                }
                Ok(())
            }
        }
    }
}

struct Test(MatchVar, Pattern);

impl Display for Test {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Test(var, pattern) => write!(f, "{var} is {pattern}"),
        }
    }
}

struct Clause {
    tests: Vec<Test>,
    bindings: Vec<(Binder, MatchVar)>,
    arm: Arm,
}

impl Display for Clause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, t) in self.tests.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{t}")?;
        }
        write!(f, " -> {}", self.arm)
    }
}

#[derive(Clone)]
enum Head {
    Literal(ast::Literal),
    Tuple(Vec<Pattern>),
    Constructor(typecheck::TypeId, usize, Vec<Pattern>),
}

impl Display for Head {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Head::Literal(lit) => lit.fmt(f),
            Head::Tuple(patterns) => {
                write!(f, "(")?;
                for (i, p) in patterns.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ")")
            }
            Head::Constructor(id, index, patterns) => {
                write!(f, "T_{}::[{index}]", id.0)?;
                for p in patterns {
                    match p {
                        Pattern::Constructor(_, _, args) if !args.is_empty() => {
                            write!(f, " ({p})")?;
                        }
                        _ => write!(f, " {p}")?,
                    }
                }
                Ok(())
            }
        }
    }
}

struct NormalizedTest(MatchVar, Head);

impl NormalizedTest {
    fn to_test(&self) -> Test {
        let NormalizedTest(var, head) = self;
        let pattern = match head {
            Head::Literal(lit) => Pattern::Literal(lit.clone()),
            Head::Tuple(patterns) => Pattern::Tuple(patterns.clone()),
            Head::Constructor(id, index, patterns) => {
                Pattern::Constructor(*id, *index, patterns.clone())
            }
        };
        Test(*var, pattern)
    }
}

impl Display for NormalizedTest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let NormalizedTest(var, head) = self;
        write!(f, "{var} is {head}")
    }
}

struct NormalizedClause {
    tests: Vec<NormalizedTest>,
    bindings: Vec<(Binder, MatchVar)>,
    arm: Arm,
}

impl NormalizedClause {
    fn to_clause(&self) -> Clause {
        Clause {
            tests: self.tests.iter().map(NormalizedTest::to_test).collect(),
            bindings: self.bindings.clone(),
            arm: self.arm,
        }
    }
}

impl Display for NormalizedClause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, t) in self.tests.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{t}")?;
        }
        write!(f, " -> {}", self.arm)
    }
}

fn siblings(lit: &ast::Literal) -> Option<Vec<ast::Literal>> {
    // finite vs infinite types
    match lit {
        ast::Literal::Unit => Some(vec![ast::Literal::Unit]),
        ast::Literal::Bool(_) => Some(vec![ast::Literal::Bool(false), ast::Literal::Bool(true)]),
        ast::Literal::Char(_) | ast::Literal::Int(_) | ast::Literal::String(_) => None,
    }
}

fn leaf(clause: &NormalizedClause) -> Tree {
    let mut bindings = clause.bindings.clone();
    bindings.sort_by_key(|(binder, _)| *binder);
    Tree::Leaf(
        clause.arm,
        bindings.into_iter().map(|(_, var)| var).collect(),
    )
}

fn number_binders(pattern: &ast::Pattern, first: Binder) -> (Pattern, Binder) {
    match &pattern.kind {
        ast::PatternKind::Wild | ast::PatternKind::Variable => {
            (Pattern::Bind(first), Binder(first.0 + 1))
        }
        ast::PatternKind::Literal(lit) => (Pattern::Literal(lit.clone()), first),
        ast::PatternKind::Tuple(patterns) => {
            let mut next = first;
            let mut numbered = Vec::with_capacity(patterns.len());
            for p in patterns {
                let (pattern, after) = number_binders(p, next);
                numbered.push(pattern);
                next = after;
            }
            (Pattern::Tuple(numbered), next)
        }
        ast::PatternKind::Constructor(id, index, patterns) => {
            let mut next = first;
            let mut numbered = Vec::with_capacity(patterns.len());
            for p in patterns {
                let (pattern, after) = number_binders(p, next);
                numbered.push(pattern);
                next = after;
            }
            (Pattern::Constructor(*id, *index, numbered), next)
        }
    }
}

fn clauses(arms: &[(ast::Pattern, ast::Expr)]) -> Vec<Clause> {
    arms.iter()
        .enumerate()
        .map(|(arm, (pattern, _))| {
            let (pattern, _) = number_binders(pattern, Binder(0));
            Clause {
                tests: vec![Test(MatchVar(0), pattern)],
                bindings: vec![],
                arm: Arm(arm),
            }
        })
        .collect()
}

#[derive(Clone, PartialEq)]
enum Ctor {
    Literal(ast::Literal),
    Adt(usize),
}

pub struct MatchCompiler<'a> {
    next_var: usize,
    fail_early: bool,
    constructor_counts: &'a [usize],
}

impl<'a> MatchCompiler<'a> {
    pub fn compile(
        span: ast::Span,
        arms: &[(ast::Pattern, ast::Expr)],
        fail_early: bool,
        constructor_counts: &'a [usize],
    ) -> Result<Tree, PatternMatchError> {
        // match variable 0 holds the scrutinee
        let mut compiler = MatchCompiler {
            next_var: 1,
            fail_early,
            constructor_counts,
        };
        compiler.build(span, &clauses(arms), &HashMap::new())
    }

    fn fresh_var(&mut self) -> MatchVar {
        let var = MatchVar(self.next_var);
        self.next_var += 1;
        var
    }

    fn build(
        &mut self,
        span: ast::Span,
        clauses: &[Clause],
        remaining: &HashMap<MatchVar, Vec<Ctor>>,
    ) -> Result<Tree, PatternMatchError> {
        // base cases
        if clauses.is_empty() {
            return if self.fail_early {
                Err(PatternMatchError::NonExhaustive { span: span.into() })
            } else {
                Ok(Tree::Err)
            };
        }

        // step 1, push bare variable tests to RHS
        let mut new_clauses = Vec::with_capacity(clauses.len());
        for clause in clauses {
            let mut new_tests = Vec::new();
            let mut bindings = clause.bindings.clone();
            for Test(var, pattern) in &clause.tests {
                match pattern {
                    Pattern::Bind(binder) => bindings.push((*binder, *var)),
                    Pattern::Literal(lit) => {
                        new_tests.push(NormalizedTest(*var, Head::Literal(lit.clone())));
                    }
                    Pattern::Tuple(patterns) => {
                        new_tests.push(NormalizedTest(*var, Head::Tuple(patterns.clone())));
                    }
                    Pattern::Constructor(id, index, patterns) => {
                        new_tests.push(NormalizedTest(
                            *var,
                            Head::Constructor(*id, *index, patterns.clone()),
                        ));
                    }
                }
            }
            let new_clause = NormalizedClause {
                tests: new_tests,
                bindings,
                arm: clause.arm,
            };
            new_clauses.push(new_clause);
        }

        if new_clauses[0].tests.is_empty() {
            return Ok(leaf(&new_clauses[0]));
        }

        // step 2, select a test
        // TODO heuristic
        let test = &new_clauses[0].tests[0];
        let var = test.0;

        // step 3, generate a pattern match
        // step 4, iterate over clauses
        match test.1.clone() {
            Head::Tuple(patterns) => {
                let vars: Vec<MatchVar> = patterns.iter().map(|_| self.fresh_var()).collect();
                let mut match_clauses = Vec::new();
                for clause in &new_clauses {
                    let Some(j) = clause.tests.iter().position(|t| t.0 == var) else {
                        // (c)
                        match_clauses.push(clause.to_clause());
                        continue;
                    };
                    // (a)
                    let Head::Tuple(patterns) = &clause.tests[j].1 else {
                        unreachable!("a tuple match variable is only tested against tuples");
                    };
                    let mut tests: Vec<Test> = clause.tests[..j]
                        .iter()
                        .map(NormalizedTest::to_test)
                        .collect();
                    tests.extend(vars.iter().zip(patterns).map(|(s, p)| Test(*s, p.clone())));
                    tests.extend(clause.tests[j + 1..].iter().map(NormalizedTest::to_test));
                    let new_clause = Clause {
                        tests,
                        bindings: clause.bindings.clone(),
                        arm: clause.arm,
                    };
                    match_clauses.push(new_clause);
                }
                // step 5, recurse on [A]
                let matched = self.build(span, &match_clauses, remaining)?;
                Ok(Tree::Branch(var, Branch::Tuple(vars, Box::new(matched))))
            }
            Head::Literal(lit) => {
                let mut match_clauses = Vec::new();
                let mut alternate_clauses = Vec::new();
                for clause in &new_clauses {
                    let Some(j) = clause.tests.iter().position(|t| t.0 == var) else {
                        // (c)
                        match_clauses.push(clause.to_clause());
                        alternate_clauses.push(clause.to_clause());
                        continue;
                    };
                    let Head::Literal(other) = &clause.tests[j].1 else {
                        unreachable!("a literal match variable is only tested against literals");
                    };
                    if *other == lit {
                        // (a)
                        let mut new_clause = clause.to_clause();
                        new_clause.tests.remove(j);
                        match_clauses.push(new_clause);
                    } else {
                        // (b)
                        alternate_clauses.push(clause.to_clause());
                    }
                }
                // step 5, recurse on [A] and [B]
                let matched = self.build(span, &match_clauses, remaining)?;

                let alternate = self.build_alternate(
                    span,
                    var,
                    &Ctor::Literal(lit.clone()),
                    siblings(&lit).map(|s| s.into_iter().map(Ctor::Literal).collect()),
                    &alternate_clauses,
                    remaining,
                )?;
                Ok(Tree::Branch(
                    var,
                    Branch::Literal(lit, Box::new(matched), alternate.map(Box::new)),
                ))
            }
            Head::Constructor(id, index, patterns) => {
                let vars: Vec<MatchVar> = patterns.iter().map(|_| self.fresh_var()).collect();
                let mut match_clauses = Vec::new();
                let mut alternate_clauses = Vec::new();
                for clause in &new_clauses {
                    let Some(j) = clause.tests.iter().position(|t| t.0 == var) else {
                        // (c)
                        match_clauses.push(clause.to_clause());
                        alternate_clauses.push(clause.to_clause());
                        continue;
                    };
                    let Head::Constructor(_, other_index, patterns) = &clause.tests[j].1 else {
                        unreachable!(
                            "a constructor match variable is only tested against constructors"
                        );
                    };
                    if index == *other_index {
                        // (a)
                        let mut tests: Vec<Test> = clause.tests[..j]
                            .iter()
                            .map(NormalizedTest::to_test)
                            .collect();
                        tests.extend(vars.iter().zip(patterns).map(|(s, p)| Test(*s, p.clone())));
                        tests.extend(clause.tests[j + 1..].iter().map(NormalizedTest::to_test));
                        let new_clause = Clause {
                            tests,
                            bindings: clause.bindings.clone(),
                            arm: clause.arm,
                        };
                        match_clauses.push(new_clause);
                    } else {
                        // (b)
                        alternate_clauses.push(clause.to_clause());
                    }
                }
                let matched = self.build(span, &match_clauses, remaining)?;

                let count = self.constructor_counts[id.0];
                let alternate = self.build_alternate(
                    span,
                    var,
                    &Ctor::Adt(index),
                    Some((0..count).map(Ctor::Adt).collect()),
                    &alternate_clauses,
                    remaining,
                )?;
                Ok(Tree::Branch(
                    var,
                    Branch::Constructor(
                        id,
                        index,
                        vars,
                        Box::new(matched),
                        alternate.map(Box::new),
                    ),
                ))
            }
        }
    }

    fn build_alternate(
        &mut self,
        span: ast::Span,
        var: MatchVar,
        tested: &Ctor,
        initial: Option<Vec<Ctor>>,
        clauses: &[Clause],
        remaining: &HashMap<MatchVar, Vec<Ctor>>,
    ) -> Result<Option<Tree>, PatternMatchError> {
        match remaining.get(&var).cloned().or(initial) {
            None => Ok(Some(self.build(span, clauses, remaining)?)),
            Some(possible) => {
                let others: Vec<Ctor> = possible.into_iter().filter(|c| c != tested).collect();
                if others.is_empty() {
                    Ok(None)
                } else {
                    let mut remaining = remaining.clone();
                    remaining.insert(var, others);
                    Ok(Some(self.build(span, clauses, &remaining)?))
                }
            }
        }
    }
}

pub fn check_patterns(
    expr: &ast::Expr,
    source: &str,
    debug: bool,
    constructor_counts: &[usize],
) -> Result<HashMap<ast::Span, Tree>, PatternMatchError> {
    let mut map = HashMap::new();
    capture_trees(expr, source, debug, constructor_counts, &mut map)?;
    Ok(map)
}

fn capture_trees(
    expr: &ast::Expr,
    source: &str,
    debug: bool,
    constructor_counts: &[usize],
    map: &mut HashMap<ast::Span, Tree>,
) -> Result<(), PatternMatchError> {
    match &expr.kind {
        ast::ExprKind::Literal(_)
        | ast::ExprKind::BoundVariable(_)
        | ast::ExprKind::FreeVariable(_) => Ok(()),
        ast::ExprKind::Application(left, right) | ast::ExprKind::Binary(_, left, right) => {
            capture_trees(left, source, debug, constructor_counts, map)?;
            capture_trees(right, source, debug, constructor_counts, map)
        }
        ast::ExprKind::Abstraction(_, expr) | ast::ExprKind::Unary(_, expr) => {
            capture_trees(expr, source, debug, constructor_counts, map)
        }
        ast::ExprKind::LetIn(_, _, expr, body) => {
            capture_trees(expr, source, debug, constructor_counts, map)?;
            capture_trees(body, source, debug, constructor_counts, map)
        }
        ast::ExprKind::IfElse(condition, consequence, alternative) => {
            capture_trees(condition, source, debug, constructor_counts, map)?;
            capture_trees(consequence, source, debug, constructor_counts, map)?;
            capture_trees(alternative, source, debug, constructor_counts, map)
        }
        ast::ExprKind::Tuple(elements) => {
            for element in elements {
                capture_trees(element, source, debug, constructor_counts, map)?;
            }
            Ok(())
        }
        ast::ExprKind::Constructor(_, _, args) => {
            for arg in args {
                capture_trees(arg, source, debug, constructor_counts, map)?;
            }
            Ok(())
        }
        ast::ExprKind::Match(scrutinee, arms) => {
            capture_trees(scrutinee, source, debug, constructor_counts, map)?;
            for (_, body) in arms {
                capture_trees(body, source, debug, constructor_counts, map)?;
            }
            let tree = MatchCompiler::compile(expr.span, arms, !debug, constructor_counts)?;
            if debug {
                println!("source:\n{}", source_text(source, expr.span));
                println!("\ntree:");
                debug_tree(&tree, arms, source, 0)?;
                println!();
            }
            if map.insert(expr.span, tree).is_some() {
                unreachable!("two matches share span {:?}", expr.span);
            }

            Ok(())
        }
    }
}

fn debug_tree(
    tree: &Tree,
    arms: &[(ast::Pattern, ast::Expr)],
    source: &str,
    depth: usize,
) -> Result<(), PatternMatchError> {
    let indent = "  ".repeat(depth);
    match tree {
        Tree::Err => {
            println!("{indent}ERR");
            Ok(())
        }
        Tree::Leaf(arm, vars) => {
            let (pattern, body) = &arms[arm.0];
            println!(
                "{indent}arm {arm}: {} => {}",
                source_text(source, pattern.span),
                source_text(source, body.span)
            );
            let mut binders = Vec::new();
            collect_binder_spans(&mut binders, pattern);
            let bindings: Vec<String> = binders
                .iter()
                .zip(vars)
                .map(|(span, var)| format!("{} = {var}", source_text(source, *span)))
                .collect();
            if !bindings.is_empty() {
                println!("{indent}  with {}", bindings.join(", "));
            }
            Ok(())
        }
        Tree::Branch(var, branch) => {
            println!("{indent}match# {var}");
            debug_branch(branch, arms, source, depth)
        }
    }
}

fn debug_branch(
    branch: &Branch,
    arms: &[(ast::Pattern, ast::Expr)],
    source: &str,
    depth: usize,
) -> Result<(), PatternMatchError> {
    let indent = "  ".repeat(depth);
    match branch {
        Branch::Literal(literal, matched, alternate) => {
            println!("{indent}| {literal} =>");
            debug_tree(matched, arms, source, depth + 1)?;
            if let Some(alternate) = alternate {
                println!("{indent}| _ =>");
                debug_tree(alternate, arms, source, depth + 1)?;
            }
            Ok(())
        }
        Branch::Tuple(vars, matched) => {
            let vars: Vec<String> = vars.iter().map(ToString::to_string).collect();
            println!("{indent}| ({}) =>", vars.join(", "));
            debug_tree(matched, arms, source, depth + 1)
        }
        Branch::Constructor(id, index, vars, matched, alternate) => {
            let vars: Vec<String> = vars.iter().map(ToString::to_string).collect();
            println!(
                "{indent}| T_{}::[{index}]{}{} =>",
                id.0,
                if vars.is_empty() { "" } else { " " },
                vars.join(" ")
            );
            debug_tree(matched, arms, source, depth + 1)?;
            if let Some(alternate) = alternate {
                println!("{indent}| _ =>");
                debug_tree(alternate, arms, source, depth + 1)?;
            }
            Ok(())
        }
    }
}

fn source_text(source: &str, span: ast::Span) -> &str {
    &source[span.start..span.end]
}

fn collect_binder_spans(spans: &mut Vec<ast::Span>, pattern: &ast::Pattern) {
    match &pattern.kind {
        ast::PatternKind::Literal(_) => {}
        ast::PatternKind::Wild | ast::PatternKind::Variable => spans.push(pattern.span),
        ast::PatternKind::Tuple(patterns) | ast::PatternKind::Constructor(_, _, patterns) => {
            for p in patterns {
                collect_binder_spans(spans, p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::front_end;

    fn compile_match(source: &str) -> Tree {
        let (program, decls) = front_end(source);
        let ast::ExprKind::Match(scrutinee, arms) = &program.kind else {
            panic!("expected a match: {source}");
        };
        let constructor_counts = decls
            .iter()
            .map(|d| d.constructors.len())
            .collect::<Vec<_>>();
        MatchCompiler::compile(scrutinee.span, arms, false, &constructor_counts).unwrap()
    }

    #[test]
    fn compiles_matches() {
        assert_eq!(
            compile_match(
                r#"match x {
                     ("A", _) => 0,
                     (_, "B") => 1,
                     (x, y) => 2,
                   }"#
            )
            .to_string(),
            "match# a_0 {
(a_1, a_2) => match# a_1 {
\"A\" => 0 [a_2],
_ => match# a_2 {
\"B\" => 1 [a_1],
_ => 2 [a_1, a_2]}}}"
        );
        assert_eq!(
            compile_match(
                "match x {
                   (1, _) => 0,
                   (_, 1) => 1,
                   _ => 2,
                 }"
            )
            .to_string(),
            "match# a_0 {
(a_1, a_2) => match# a_1 {
1 => 0 [a_2],
_ => match# a_2 {
1 => 1 [a_1],
_ => 2 [a_0]}}}"
        );
        assert_eq!(
            compile_match(
                "match t {
                   (x, y, 1) => x,
                   (x, y, 2) => y,
                   (_, _, 3) => \"default\"
                 }"
            )
            .to_string(),
            "match# a_0 {
(a_1, a_2, a_3) => match# a_3 {
1 => 0 [a_1, a_2],
_ => match# a_3 {
2 => 1 [a_1, a_2],
_ => match# a_3 {
3 => 2 [a_1, a_2],
_ => ERR}}}}"
        );
        assert_eq!(
            compile_match(
                "type List t = Nil | Cons t (List t);
                 match t {
                   List::Nil => 0,
                   List::Cons 1 _ => 1,
                 }"
            )
            .to_string(),
            "match# a_0 {
T_0::[0] => 0 [],
_ => match# a_0 {
T_0::[1] a_1 a_2 => match# a_1 {
1 => 1 [a_2],
_ => ERR}}}"
        );
        // every constructor covered, so the last test has no alternate
        assert_eq!(
            compile_match(
                "type List t = Nil | Cons t (List t);
                 match t {
                   List::Nil => 0,
                   List::Cons x xs => x 
                 }"
            )
            .to_string(),
            "match# a_0 {
T_0::[0] => 0 [],
_ => match# a_0 {
T_0::[1] a_1 a_2 => 1 [a_1, a_2]}}"
        );
        assert_eq!(
            compile_match(
                "type List t = Nil | Cons t (List t);
                 match t {
                   List::Cons a (List::Cons b _) => a - b,
                   List::Cons a _ => a,
                   List::Nil => 0,
                 }"
            )
            .to_string(),
            "match# a_0 {
T_0::[1] a_1 a_2 => match# a_2 {
T_0::[1] a_3 a_4 => 0 [a_1, a_3, a_4],
_ => 1 [a_1, a_2]},
_ => match# a_0 {
T_0::[0] => 2 []}}"
        );
        assert_eq!(
            compile_match(
                "type Option a = None | Some a;
                 match t {
                   (Option::Some a, b) => a - b,
                   (Option::None, c) => c
                 }"
            )
            .to_string(),
            "match# a_0 {
(a_1, a_2) => match# a_1 {
T_0::[1] a_3 => 0 [a_3, a_2],
_ => match# a_1 {
T_0::[0] => 1 [a_2]}}}"
        );
    }
}
