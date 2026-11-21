//! Based on [Jules Jacobs' 2021 notes on compiling pattern matching](https://julesjacobs.com/notes/patternmatching/patternmatching.pdf)

use miette::{Diagnostic, SourceSpan};
use std::collections::HashMap;
use std::fmt::Display;
use thiserror::Error;

use crate::ast;

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
            Branch::Tuple(patterns, tree) => {
                write!(f, "(")?;
                for (i, p) in patterns.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ") => {tree}")
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

pub struct MatchCompiler {
    next_var: usize,
    fail_early: bool,
}

impl MatchCompiler {
    pub fn compile(
        span: ast::Span,
        arms: &[(ast::Pattern, ast::Expr)],
        fail_early: bool,
    ) -> Result<Tree, PatternMatchError> {
        // match variable 0 holds the scrutinee
        let mut compiler = MatchCompiler {
            next_var: 1,
            fail_early,
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
        remaining: &HashMap<MatchVar, Vec<ast::Literal>>,
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

                let alternate = match remaining.get(&var).cloned().or_else(|| siblings(&lit)) {
                    None => Some(self.build(span, &alternate_clauses, remaining)?),
                    Some(possible) => {
                        let others: Vec<ast::Literal> =
                            possible.into_iter().filter(|l| *l != lit).collect();
                        if others.is_empty() {
                            None
                        } else {
                            let mut remaining = remaining.clone();
                            remaining.insert(var, others);
                            Some(self.build(span, &alternate_clauses, &remaining)?)
                        }
                    }
                };
                Ok(Tree::Branch(
                    var,
                    Branch::Literal(lit, Box::new(matched), alternate.map(Box::new)),
                ))
            }
        }
    }
}

pub fn check_patterns(
    expr: &ast::Expr,
    source: &str,
    debug: bool,
) -> Result<HashMap<ast::Span, Tree>, PatternMatchError> {
    let mut map = HashMap::new();
    capture_trees(expr, source, debug, &mut map)?;
    Ok(map)
}

fn capture_trees(
    expr: &ast::Expr,
    source: &str,
    debug: bool,
    map: &mut HashMap<ast::Span, Tree>,
) -> Result<(), PatternMatchError> {
    match &expr.kind {
        ast::ExprKind::Literal(_)
        | ast::ExprKind::BoundVariable(_)
        | ast::ExprKind::FreeVariable(_)
        | ast::ExprKind::Constructor(_, _) => Ok(()), // TODO constructors
        ast::ExprKind::Application(left, right) | ast::ExprKind::Binary(_, left, right) => {
            capture_trees(left, source, debug, map)?;
            capture_trees(right, source, debug, map)
        }
        ast::ExprKind::Abstraction(_, expr) | ast::ExprKind::Unary(_, expr) => {
            capture_trees(expr, source, debug, map)
        }
        ast::ExprKind::LetIn(_, _, expr, body) => {
            capture_trees(expr, source, debug, map)?;
            capture_trees(body, source, debug, map)
        }
        ast::ExprKind::IfElse(condition, consequence, alternative) => {
            capture_trees(condition, source, debug, map)?;
            capture_trees(consequence, source, debug, map)?;
            capture_trees(alternative, source, debug, map)
        }
        ast::ExprKind::Tuple(elements) => {
            for element in elements {
                capture_trees(element, source, debug, map)?;
            }
            Ok(())
        }
        ast::ExprKind::Match(scrutinee, arms) => {
            capture_trees(scrutinee, source, debug, map)?;
            for (_, body) in arms {
                capture_trees(body, source, debug, map)?;
            }
            let tree = MatchCompiler::compile(expr.span, arms, !debug)?;
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
    }
}

fn source_text(source: &str, span: ast::Span) -> &str {
    &source[span.start..span.end]
}

fn collect_binder_spans(spans: &mut Vec<ast::Span>, pattern: &ast::Pattern) {
    match &pattern.kind {
        ast::PatternKind::Literal(_) => {}
        ast::PatternKind::Wild | ast::PatternKind::Variable => spans.push(pattern.span),
        ast::PatternKind::Tuple(patterns) => {
            for p in patterns {
                collect_binder_spans(spans, p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{elaborate::elaborate, parser::Parser};

    fn compile_match(source: &str) -> Tree {
        let (program, _) = elaborate(Parser::parse(source).unwrap().0, &[]).unwrap();
        let ast::ExprKind::Match(scrutinee, arms) = &program.kind else {
            panic!("expected a match: {source}");
        };
        MatchCompiler::compile(scrutinee.span, arms, false).unwrap()
    }

    #[test]
    fn compiles_matches() {
        assert_eq!(
            compile_match(r#"match x { ("A", _) => 0, (_, "B") => 1, (x, y) => 2 }"#).to_string(),
            "match# a_0 {\n(a_1, a_2) => match# a_1 {\n\"A\" => 0 [a_2],\n_ => match# a_2 {\n\"B\" => 1 [a_1],\n_ => 2 [a_1, a_2]}}}"
        );
        assert_eq!(
            compile_match("match x { (1, _) => 0, (_, 1) => 1, _ => 2 }").to_string(),
            "match# a_0 {\n(a_1, a_2) => match# a_1 {\n1 => 0 [a_2],\n_ => match# a_2 {\n1 => 1 [a_1],\n_ => 2 [a_0]}}}"
        );
        assert_eq!(
            compile_match("match _ { (x, y, 1) => x, (x, y, 2) => y, (_, _, 3) => \"default\" }")
                .to_string(),
            "match# a_0 {\n(a_1, a_2, a_3) => match# a_3 {\n1 => 0 [a_1, a_2],\n_ => match# a_3 {\n2 => 1 [a_1, a_2],\n_ => match# a_3 {\n3 => 2 [a_1, a_2],\n_ => ERR}}}}"
        );
    }
}
