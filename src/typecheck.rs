use std::{
    cmp,
    collections::{HashMap, HashSet},
    fmt::Display,
    rc::Rc,
};

use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

use crate::ast;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeId(pub usize);

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Unit,
    Bool,
    Char,
    Int,
    String,
    Var(i64, bool), // is this an eq variable
    UnifVar(u32),
    Arrow(Rc<TypeExpr>, Rc<TypeExpr>),
    Named(TypeId, Rc<[TypeExpr]>),
    Product(Rc<[TypeExpr]>),
}

impl TypeExpr {
    fn greek_letter(n: u32) -> (char, u32) {
        let letter = char::from_u32(u32::from('α') + n % 25).expect("α..ω are valid chars");
        (letter, n / 25)
    }

    fn create_existential_variable_name(id: u32) -> String {
        const COMBINING_CIRCUMFLEX_ACCENT: char = '\u{0302}';
        match Self::greek_letter(id) {
            (letter, 0) => format!("{letter}{COMBINING_CIRCUMFLEX_ACCENT}"),
            (letter, c) => format!("{letter}{COMBINING_CIRCUMFLEX_ACCENT}{c}"),
        }
    }

    fn create_type_variable_name(level: i64) -> String {
        let level = u32::try_from(level).expect("type variable levels are non-negative");
        match Self::greek_letter(level) {
            (letter, 0) => letter.to_string(),
            (letter, c) => format!("{letter}{c}"),
        }
    }

    pub fn max_type_variable_level(tp: &TypeExpr) -> i64 {
        match tp {
            TypeExpr::Unit
            | TypeExpr::Bool
            | TypeExpr::Char
            | TypeExpr::Int
            | TypeExpr::String
            | TypeExpr::UnifVar(_) => -1,
            TypeExpr::Var(level, _) => *level,
            TypeExpr::Arrow(src, tgt) => cmp::max(
                Self::max_type_variable_level(src),
                Self::max_type_variable_level(tgt),
            ),
            TypeExpr::Named(_, elts) | TypeExpr::Product(elts) => elts
                .iter()
                .fold(-1, |acc, e| cmp::max(acc, Self::max_type_variable_level(e))),
        }
    }

    pub fn arrow(src: TypeExpr, tgt: TypeExpr) -> TypeExpr {
        Self::Arrow(Rc::new(src), Rc::new(tgt))
    }
}

impl Display for TypeExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeExpr::Unit => write!(f, "Unit"),
            TypeExpr::Bool => write!(f, "Bool"),
            TypeExpr::Char => write!(f, "Char"),
            TypeExpr::Int => write!(f, "Int"),
            TypeExpr::String => write!(f, "String"),
            TypeExpr::Var(level, eq) => {
                write!(
                    f,
                    "{}{}",
                    if *eq { "''" } else { "" },
                    Self::create_type_variable_name(*level)
                )
            }
            TypeExpr::UnifVar(id) => {
                write!(f, "{}", Self::create_existential_variable_name(*id))
            }
            TypeExpr::Arrow(src, tgt) => {
                if matches!(src.as_ref(), TypeExpr::Arrow(_, _)) {
                    write!(f, "({src}) -> {tgt}")
                } else {
                    write!(f, "{src} -> {tgt}")
                }
            }
            TypeExpr::Named(id, args) => {
                write!(f, "T_{}", id.0)?;
                for t in args.iter() {
                    match t {
                        TypeExpr::Arrow(_, _) => {
                            write!(f, " ({t})")?;
                        }
                        TypeExpr::Named(_, inner_args) if !inner_args.is_empty() => {
                            write!(f, " ({t})")?;
                        }
                        _ => {
                            write!(f, " {t}")?;
                        }
                    }
                }
                Ok(())
            }
            TypeExpr::Product(elts) => {
                write!(f, "(")?;
                for (i, e) in elts.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{e}")?;
                }
                write!(f, ")")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
    arity: u32,
    tp: TypeExpr,
}

impl Display for Scheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.arity == 0 {
            self.tp.fmt(f)
        } else {
            write!(f, "(")?;
            for level in 0..=TypeExpr::max_type_variable_level(&self.tp) {
                write!(f, "∀{}.", TypeExpr::create_type_variable_name(level))?;
            }
            write!(f, "{})", self.tp)
        }
    }
}

impl Scheme {
    pub fn as_monotype(tp: TypeExpr) -> Scheme {
        Scheme { arity: 0, tp }
    }

    pub fn tp(&self) -> &TypeExpr {
        &self.tp
    }
}

#[derive(Debug, Error, Diagnostic)]
pub enum TypeError {
    #[error("unbound variable `{name}`")]
    UnboundVariable {
        name: String,
        #[label("not defined")]
        span: SourceSpan,
    },
    #[error("variable #{index} is not in scope")]
    #[diagnostic(code(typecheck::unbound_index))]
    UnboundIndex {
        index: i64,
        #[label("not in scope")]
        span: SourceSpan,
    },
    #[error("could not unify {left} with {right}")]
    #[diagnostic(code(typecheck::unification))]
    Unification {
        left: String,
        right: String,
        #[label("this term")]
        span: SourceSpan,
    },
    #[error("occurs check failed for {tp}")]
    #[diagnostic(code(typecheck::occurs_check))]
    OccursCheck {
        tp: String,
        #[label("this term")]
        span: SourceSpan,
    },
    #[error("type cannot be compared with '==' {tp}")]
    #[diagnostic(code(typecheck::equality))]
    Equality {
        tp: String,
        #[label("this term")]
        span: SourceSpan,
    },
    #[error("cannot destructure in recursive let")]
    #[diagnostic(code(typecheck::destructured_let_rec))]
    DestructuredLetRec {
        #[label("cannot destructure here")]
        span: SourceSpan,
    },
    #[error("this pattern cannot contain literal values")]
    #[diagnostic(code(typecheck::pattern_contains_literals))]
    PatternContainsLiterals {
        #[label("cannot include literals here")]
        span: SourceSpan,
    },
    #[error("this pattern cannot contain constructors")]
    #[diagnostic(code(typecheck::pattern_contains_constructors))]
    PatternContainsConstructors {
        #[label("cannot include constructors here")]
        span: SourceSpan,
    },
    #[error("arms of match must have same type")]
    #[diagnostic(code(typecheck::match_arms))]
    MatchArms {
        #[label("this arm does not have the expected type")]
        span: SourceSpan,
    },
    #[error("unknown constructor")]
    #[diagnostic(code(typecheck::unknown_constructor))]
    UnknownConstructor {
        id: usize,
        index: usize,
        #[label("could not find constructor")]
        span: SourceSpan,
    },
    #[error("wrong arity for constructors")]
    #[diagnostic(code(typecheck::wrong_constructor_arity))]
    WrongConstructorArity {
        expected: usize,
        actual: usize,
        #[label("wrong number of arguments")]
        span: SourceSpan,
    },
}

type Result<T> = std::result::Result<T, TypeError>;

pub struct TypeChecker {
    env: HashMap<String, Scheme>,
    constructors: Vec<Vec<Scheme>>,
    fields: Vec<Vec<Vec<TypeExpr>>>,
    context: Vec<Scheme>,
    substitution: Vec<Option<TypeExpr>>,
    bindings: Vec<(ast::Span, Scheme)>,
    eq: Vec<bool>, // which type variables can be compared with ==
    levels: Vec<i64>,
    current_level: i64,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            env: HashMap::new(),
            constructors: Vec::new(),
            fields: Vec::new(),
            context: Vec::new(),
            substitution: Vec::new(),
            bindings: Vec::new(),
            eq: Vec::new(),
            levels: Vec::new(),
            current_level: 0,
        }
    }

    pub fn infer(&mut self, expr: &ast::Expr) -> Result<Scheme> {
        let tp = self.algorithm_j(expr)?;
        let resolved = self.resolve(tp);
        let scheme = self.generalize_at_level(resolved, -1);
        Ok(scheme)
    }

    pub fn set_env(&mut self, name: String, tp: Scheme) {
        self.env.insert(name, tp);
    }

    pub fn add_types(&mut self, decls: &[ast::TypeDecl]) {
        for (id, decl) in decls.iter().enumerate() {
            let arity = u32::try_from(decl.arity).unwrap();
            let params = (0..i64::from(arity)).map(|i| TypeExpr::Var(i, false));
            let result = TypeExpr::Named(TypeId(id), params.collect());
            let schemes = decl
                .constructors
                .iter()
                .map(|(_, fields)| Scheme {
                    arity,
                    tp: fields.iter().rev().fold(result.clone(), |acc, field| {
                        TypeExpr::arrow(field.clone(), acc)
                    }),
                })
                .collect();
            self.constructors.push(schemes);
            self.fields.push(
                decl.constructors
                    .iter()
                    .map(|(_, fields)| fields.clone())
                    .collect(),
            );
        }
    }

    pub fn bindings(&self) -> &[(ast::Span, Scheme)] {
        &self.bindings
    }

    fn algorithm_j(&mut self, expr: &ast::Expr) -> Result<TypeExpr> {
        match &expr.kind {
            ast::ExprKind::Literal(lit) => Ok(Self::rule_lit(lit)),
            ast::ExprKind::BoundVariable(index) => self.rule_var(expr.span, *index),
            ast::ExprKind::FreeVariable(name) => self.rule_free_var(expr.span, name.clone()),
            ast::ExprKind::Application(left, right) => self.rule_app(expr.span, left, right),
            ast::ExprKind::Abstraction(_, body) => self.rule_abs(body),
            ast::ExprKind::LetIn(false, pattern, e, body) => self.rule_let(pattern, e, body),
            ast::ExprKind::LetIn(true, pattern, e, body) => {
                self.rule_let_rec(expr.span, pattern, e, body)
            }
            ast::ExprKind::IfElse(condition, consequence, alternative) => {
                self.rule_if(expr.span, condition, consequence, alternative)
            }
            ast::ExprKind::Unary(op, e) => self.rule_unary(expr.span, *op, e),
            ast::ExprKind::Binary(op, left, right) => self.rule_binary(expr.span, *op, left, right),
            ast::ExprKind::Tuple(elements) => self.rule_tuple(elements),
            ast::ExprKind::Constructor(id, index, args) => {
                self.rule_constructor(expr.span, *id, *index, args)
            }
            ast::ExprKind::Match(scrutinee, arms) => self.rule_match(scrutinee, arms),
        }
    }

    fn rule_lit(lit: &ast::Literal) -> TypeExpr {
        match lit {
            ast::Literal::Unit => TypeExpr::Unit,
            ast::Literal::Bool(_) => TypeExpr::Bool,
            ast::Literal::Char(_) => TypeExpr::Char,
            ast::Literal::Int(_) => TypeExpr::Int,
            ast::Literal::String(_) => TypeExpr::String,
        }
    }

    fn rule_var(&mut self, span: ast::Span, index: i64) -> Result<TypeExpr> {
        let scheme = usize::try_from(index)
            .ok()
            .and_then(|i| self.context.len().checked_sub(i + 1))
            .map(|position| self.context[position].clone())
            .ok_or(TypeError::UnboundIndex {
                index,
                span: span.into(),
            })?;
        Ok(self.instantiate(&scheme))
    }

    fn rule_free_var(&mut self, span: ast::Span, name: String) -> Result<TypeExpr> {
        match self.env.get(&name).cloned() {
            Some(scheme) => Ok(self.instantiate(&scheme)),
            None => Err(TypeError::UnboundVariable {
                name,
                span: span.into(),
            }),
        }
    }

    fn rule_app(
        &mut self,
        span: ast::Span,
        left: &ast::Expr,
        right: &ast::Expr,
    ) -> Result<TypeExpr> {
        let t1 = self.algorithm_j(left)?;
        let t2 = self.algorithm_j(right)?;
        let t_prime = self.new_unification_variable(false);
        self.unify(
            span,
            t1,
            TypeExpr::Arrow(Rc::new(t2), Rc::new(t_prime.clone())),
        )?;
        Ok(t_prime)
    }

    fn rule_abs(&mut self, body: &ast::Expr) -> Result<TypeExpr> {
        let t = self.new_unification_variable(false);
        self.context.push(Scheme::as_monotype(t.clone()));
        let t_prime = self.algorithm_j(body)?;
        self.context.pop();
        Ok(TypeExpr::Arrow(Rc::new(t), Rc::new(t_prime)))
    }

    fn rule_let(
        &mut self,
        pattern: &ast::Pattern,
        expr: &ast::Expr,
        body: &ast::Expr,
    ) -> Result<TypeExpr> {
        self.current_level += 1;
        let t_prime = self.algorithm_j(expr)?;
        let mut leaves = Vec::new();
        self.unify_pattern(true, pattern, &t_prime, &mut leaves)?;
        self.current_level -= 1;
        let pop_count = leaves.len();
        for (span, t) in leaves {
            let t_gen = self.generalize(t);
            self.bindings.push((span, t_gen.clone()));
            self.context.push(t_gen);
        }
        let t = self.algorithm_j(body)?;
        for _ in 0..pop_count {
            self.context.pop();
        }
        Ok(t)
    }

    fn rule_let_rec(
        &mut self,
        span: ast::Span,
        pattern: &ast::Pattern,
        expr: &ast::Expr,
        body: &ast::Expr,
    ) -> Result<TypeExpr> {
        self.current_level += 1;
        let t_prime = self.new_unification_variable(false);
        self.context.push(Scheme::as_monotype(t_prime.clone()));
        let t1 = self.algorithm_j(expr)?;
        self.context.pop();
        self.unify(span, t_prime.clone(), t1)?;
        self.current_level -= 1;
        let t_prime_gen = self.generalize(t_prime);
        match &pattern.kind {
            ast::PatternKind::Wild => {}
            ast::PatternKind::Variable => {
                self.bindings.push((pattern.span, t_prime_gen.clone()));
            }
            ast::PatternKind::Literal(_) => {
                return Err(TypeError::PatternContainsLiterals {
                    span: pattern.span.into(),
                });
            }
            ast::PatternKind::Tuple(_) => {
                return Err(TypeError::DestructuredLetRec {
                    span: pattern.span.into(),
                });
            }
            ast::PatternKind::Constructor(_, _, _) => {
                return Err(TypeError::PatternContainsConstructors {
                    span: pattern.span.into(),
                });
            }
        }
        self.context.push(t_prime_gen);
        let t = self.algorithm_j(body)?;
        self.context.pop();
        Ok(t)
    }

    fn rule_if(
        &mut self,
        span: ast::Span,
        condition: &ast::Expr,
        consequence: &ast::Expr,
        alternative: &ast::Expr,
    ) -> Result<TypeExpr> {
        let condition_tp = self.algorithm_j(condition)?;
        self.unify(span, condition_tp, TypeExpr::Bool)?;
        let cons_tp = self.algorithm_j(consequence)?;
        let alt_tp = self.algorithm_j(alternative)?;
        self.unify(span, cons_tp.clone(), alt_tp)?;
        Ok(cons_tp)
    }

    fn rule_unary(
        &mut self,
        span: ast::Span,
        op: ast::UnaryOp,
        expr: &ast::Expr,
    ) -> Result<TypeExpr> {
        let tp = self.algorithm_j(expr)?;
        match op {
            ast::UnaryOp::Negate | ast::UnaryOp::BitNot => {
                self.unify(span, tp, TypeExpr::Int)?;
                Ok(TypeExpr::Int)
            }
            ast::UnaryOp::Not => {
                self.unify(span, tp, TypeExpr::Bool)?;
                Ok(TypeExpr::Bool)
            }
        }
    }

    fn rule_binary(
        &mut self,
        span: ast::Span,
        op: ast::BinaryOp,
        left: &ast::Expr,
        right: &ast::Expr,
    ) -> Result<TypeExpr> {
        let l_tp = self.algorithm_j(left)?;
        let r_tp = self.algorithm_j(right)?;
        match op {
            ast::BinaryOp::Add
            | ast::BinaryOp::Sub
            | ast::BinaryOp::Mul
            | ast::BinaryOp::Div
            | ast::BinaryOp::BitAnd
            | ast::BinaryOp::BitOr => {
                self.unify(span, l_tp, TypeExpr::Int)?;
                self.unify(span, r_tp, TypeExpr::Int)?;
                Ok(TypeExpr::Int)
            }
            ast::BinaryOp::Lt | ast::BinaryOp::Gt | ast::BinaryOp::Lte | ast::BinaryOp::Gte => {
                self.unify(span, l_tp, TypeExpr::Int)?;
                self.unify(span, r_tp, TypeExpr::Int)?;
                Ok(TypeExpr::Bool)
            }
            ast::BinaryOp::Concat => {
                self.unify(span, l_tp, TypeExpr::String)?;
                self.unify(span, r_tp, TypeExpr::String)?;
                Ok(TypeExpr::String)
            }
            ast::BinaryOp::And | ast::BinaryOp::Or => {
                self.unify(span, l_tp, TypeExpr::Bool)?;
                self.unify(span, r_tp, TypeExpr::Bool)?;
                Ok(TypeExpr::Bool)
            }
            ast::BinaryOp::Eq => {
                let a = self.new_unification_variable(true);
                self.unify(span, l_tp, a.clone())?;
                self.unify(span, r_tp, a)?;
                Ok(TypeExpr::Bool)
            }
        }
    }

    fn rule_tuple(&mut self, elements: &[ast::Expr]) -> Result<TypeExpr> {
        let mut elts = Vec::new();
        for elt in elements {
            elts.push(self.algorithm_j(elt)?);
        }
        Ok(TypeExpr::Product(elts.into()))
    }

    fn rule_match(
        &mut self,
        scrutinee: &ast::Expr,
        arms: &[(ast::Pattern, ast::Expr)],
    ) -> Result<TypeExpr> {
        let t = self.algorithm_j(scrutinee)?;
        let result = self.new_unification_variable(false);
        for (pattern, body) in arms {
            let mut leaves = Vec::new();
            self.unify_pattern(false, pattern, &t, &mut leaves)?;
            let pop_count = leaves.len();
            for (_, tp) in leaves {
                self.context.push(Scheme::as_monotype(tp));
            }
            let t_arm = self.algorithm_j(body)?;
            for _ in 0..pop_count {
                self.context.pop();
            }
            self.unify(body.span, result.clone(), t_arm)
                .map_err(|e| match e {
                    TypeError::Unification { span, .. } => TypeError::MatchArms { span },
                    e => e,
                })?;
        }
        Ok(result)
    }

    fn rule_constructor(
        &mut self,
        span: ast::Span,
        id: TypeId,
        index: usize,
        args: &[ast::Expr],
    ) -> Result<TypeExpr> {
        let scheme = self.get_constructor_scheme(span, id, index)?;
        let mut tp = self.instantiate(&scheme);
        for arg in args {
            // same as rule_app for each arg
            let t2 = self.algorithm_j(arg)?;
            let t_prime = self.new_unification_variable(false);
            self.unify(
                arg.span,
                tp,
                TypeExpr::Arrow(Rc::new(t2), Rc::new(t_prime.clone())),
            )?;
            tp = t_prime;
        }
        Ok(tp)
    }

    fn unify_pattern(
        &mut self,
        is_let: bool,
        pattern: &ast::Pattern,
        tp: &TypeExpr,
        out: &mut Vec<(ast::Span, TypeExpr)>,
    ) -> Result<()> {
        match (&pattern.kind, tp) {
            (ast::PatternKind::Literal(lit), t) => {
                if is_let {
                    Err(TypeError::PatternContainsLiterals {
                        span: pattern.span.into(),
                    })
                } else {
                    self.unify(pattern.span, Self::rule_lit(lit), (*t).clone())
                }
            }
            (ast::PatternKind::Wild | ast::PatternKind::Variable, t) => {
                out.push((pattern.span, (*t).clone()));
                Ok(())
            }
            (ast::PatternKind::Tuple(patterns), TypeExpr::Product(elts))
                if patterns.len() == elts.len() =>
            {
                for (p, t) in patterns.iter().zip(elts.iter()) {
                    self.unify_pattern(is_let, p, t, out)?;
                }
                Ok(())
            }
            (ast::PatternKind::Tuple(patterns), t) => {
                let fresh: Vec<TypeExpr> = patterns
                    .iter()
                    .map(|_| self.new_unification_variable(false))
                    .collect();
                self.unify(
                    pattern.span,
                    TypeExpr::Product(fresh.clone().into()),
                    t.clone(),
                )?;
                for (p, v) in patterns.iter().zip(&fresh) {
                    self.unify_pattern(is_let, p, v, out)?;
                }
                Ok(())
            }
            (ast::PatternKind::Constructor(id, index, patterns), t) => {
                if is_let {
                    return Err(TypeError::PatternContainsConstructors {
                        span: pattern.span.into(),
                    });
                }
                let mut tp =
                    self.instantiate(&self.get_constructor_scheme(pattern.span, *id, *index)?);
                let mut fields = Vec::new();
                for _ in patterns {
                    let TypeExpr::Arrow(field, rest) = tp else {
                        // too few
                        return Err(TypeError::WrongConstructorArity {
                            expected: fields.len(),
                            actual: patterns.len(),
                            span: pattern.span.into(),
                        });
                    };
                    fields.push(field);
                    tp = (*rest).clone();
                }
                if let TypeExpr::Arrow(_, _) = tp {
                    // too many
                    let mut remaining = 0;
                    let mut rest = &tp;
                    while let TypeExpr::Arrow(_, next) = rest {
                        remaining += 1;
                        rest = next;
                    }
                    return Err(TypeError::WrongConstructorArity {
                        expected: fields.len() + remaining,
                        actual: patterns.len(),
                        span: pattern.span.into(),
                    });
                }
                self.unify(pattern.span, tp, t.clone())?;
                for (p, field) in patterns.iter().zip(&fields) {
                    self.unify_pattern(is_let, p, field, out)?;
                }
                Ok(())
            }
        }
    }

    fn get_constructor_scheme(&self, span: ast::Span, id: TypeId, index: usize) -> Result<Scheme> {
        self.constructors
            .get(id.0)
            .and_then(|ctors| ctors.get(index))
            .cloned()
            .ok_or(TypeError::UnknownConstructor {
                id: id.0,
                index,
                span: span.into(),
            })
    }

    fn new_unification_variable(&mut self, is_eq: bool) -> TypeExpr {
        let id = u32::try_from(self.levels.len()).unwrap();
        self.eq.push(is_eq);
        self.levels.push(self.current_level);
        self.substitution.push(None);
        TypeExpr::UnifVar(id)
    }

    fn require_equality(&mut self, span: ast::Span, tp: TypeExpr) -> Result<()> {
        match self.resolve(tp) {
            TypeExpr::Unit | TypeExpr::Bool | TypeExpr::Char | TypeExpr::Int | TypeExpr::String => {
                Ok(())
            }
            TypeExpr::UnifVar(id) => {
                let idx = usize::try_from(id).unwrap();
                self.eq[idx] = true;
                Ok(())
            }
            TypeExpr::Named(id, elts) => {
                if !self.fields_allow_equality(id, &mut HashSet::new()) {
                    return Err(TypeError::Equality {
                        tp: TypeExpr::Named(id, elts).to_string(),
                        span: span.into(),
                    });
                }
                for e in elts.iter() {
                    self.require_equality(span, e.clone())?;
                }
                Ok(())
            }
            TypeExpr::Product(elts) => {
                for e in elts.iter() {
                    self.require_equality(span, e.clone())?;
                }
                Ok(())
            }
            other => Err(TypeError::Equality {
                tp: other.to_string(),
                span: span.into(),
            }),
        }
    }

    fn fields_allow_equality(&self, id: TypeId, visited: &mut HashSet<usize>) -> bool {
        if !visited.insert(id.0) {
            return true;
        }
        let Some(ctors) = self.fields.get(id.0) else {
            return false;
        };
        ctors
            .iter()
            .flatten()
            .all(|tp| self.field_allows_equality(tp, visited))
    }

    fn field_allows_equality(&self, tp: &TypeExpr, visited: &mut HashSet<usize>) -> bool {
        match tp {
            TypeExpr::Unit
            | TypeExpr::Bool
            | TypeExpr::Char
            | TypeExpr::Int
            | TypeExpr::String
            | TypeExpr::Var(_, _) => true,
            TypeExpr::Product(elts) => elts.iter().all(|e| self.field_allows_equality(e, visited)),
            TypeExpr::Named(id, elts) => {
                elts.iter().all(|e| self.field_allows_equality(e, visited))
                    && self.fields_allow_equality(*id, visited)
            }
            TypeExpr::Arrow(_, _) | TypeExpr::UnifVar(_) => false,
        }
    }

    fn unify(&mut self, span: ast::Span, left_init: TypeExpr, right_init: TypeExpr) -> Result<()> {
        let left = self.resolve(left_init);
        let right = self.resolve(right_init);
        match (left, right) {
            (TypeExpr::Unit, TypeExpr::Unit)
            | (TypeExpr::Bool, TypeExpr::Bool)
            | (TypeExpr::Char, TypeExpr::Char)
            | (TypeExpr::Int, TypeExpr::Int)
            | (TypeExpr::String, TypeExpr::String) => Ok(()),
            (TypeExpr::Arrow(ss, st), TypeExpr::Arrow(ts, tt)) => {
                self.unify(span, (*ss).clone(), (*ts).clone())?;
                self.unify(span, (*st).clone(), (*tt).clone())
            }
            (TypeExpr::Named(l_id, ls), TypeExpr::Named(r_id, rs))
                if l_id == r_id && ls.len() == rs.len() =>
            {
                for (l, r) in ls.iter().zip(rs.iter()) {
                    self.unify(span, (*l).clone(), (*r).clone())?;
                }
                Ok(())
            }
            (TypeExpr::Product(ls), TypeExpr::Product(rs)) if ls.len() == rs.len() => {
                for (l, r) in ls.iter().zip(rs.iter()) {
                    self.unify(span, (*l).clone(), (*r).clone())?;
                }
                Ok(())
            }
            (TypeExpr::UnifVar(l_id), TypeExpr::UnifVar(r_id)) => {
                if l_id == r_id {
                    return Ok(());
                }
                let (l_idx, r_idx) = (
                    usize::try_from(l_id).unwrap(),
                    usize::try_from(r_id).unwrap(),
                );
                if self.levels[l_idx] <= self.levels[r_idx] {
                    self.levels[r_idx] = self.levels[l_idx];
                    self.substitution[l_idx] = Some(TypeExpr::UnifVar(r_id));
                    self.eq[r_idx] |= self.eq[l_idx];
                } else {
                    self.levels[l_idx] = self.levels[r_idx];
                    self.substitution[r_idx] = Some(TypeExpr::UnifVar(l_id));
                    self.eq[l_idx] |= self.eq[r_idx];
                }
                Ok(())
            }
            (TypeExpr::UnifVar(id), tp) | (tp, TypeExpr::UnifVar(id)) => {
                if self.occurs(id, tp.clone()) {
                    Err(TypeError::OccursCheck {
                        tp: self.resolve(tp).to_string(),
                        span: span.into(),
                    })
                } else {
                    let idx = usize::try_from(id).unwrap();
                    if self.eq[idx] {
                        self.require_equality(span, tp.clone())?;
                    }
                    let level = self.levels[idx];
                    self.lower_levels(tp.clone(), level);
                    self.substitution[idx] = Some(tp);
                    Ok(())
                }
            }
            (l, r) if l == r => Ok(()),
            (l, r) => Err(TypeError::Unification {
                left: l.to_string(),
                right: r.to_string(),
                span: span.into(),
            }),
        }
    }

    fn resolve(&mut self, tp: TypeExpr) -> TypeExpr {
        match tp {
            TypeExpr::UnifVar(mut id) => {
                let index = |id: u32| usize::try_from(id).unwrap();
                let mut path = Vec::new();
                while let Some(TypeExpr::UnifVar(next)) = &self.substitution[index(id)] {
                    path.push(id);
                    id = *next;
                }
                let result = self.substitution[index(id)]
                    .clone()
                    .unwrap_or(TypeExpr::UnifVar(id));
                // path compress
                for var in path {
                    self.substitution[index(var)] = Some(result.clone());
                }
                result
            }
            x => x,
        }
    }

    // open
    fn instantiate(&mut self, scheme: &Scheme) -> TypeExpr {
        if scheme.arity == 0 {
            return scheme.tp.clone();
        }
        let fresh: Vec<TypeExpr> = (0..scheme.arity)
            .map(|_| self.new_unification_variable(false))
            .collect();
        self.instantiate_inner(&scheme.tp, &fresh)
    }

    fn instantiate_inner(&mut self, tp: &TypeExpr, fresh: &[TypeExpr]) -> TypeExpr {
        match tp {
            TypeExpr::Var(level, is_eq) => {
                let level_idx = usize::try_from(*level).expect("levels are non-negative");
                let var = fresh[level_idx].clone();
                if *is_eq && let TypeExpr::UnifVar(id) = var {
                    let idx = usize::try_from(id).unwrap();
                    self.eq[idx] = true;
                }
                var
            }
            TypeExpr::Arrow(src, tgt) => TypeExpr::Arrow(
                Rc::new(self.instantiate_inner(src, fresh)),
                Rc::new(self.instantiate_inner(tgt, fresh)),
            ),
            TypeExpr::Named(id, args) => TypeExpr::Named(
                *id,
                args.iter()
                    .map(|e| self.instantiate_inner(e, fresh))
                    .collect(),
            ),
            TypeExpr::Product(elts) => TypeExpr::Product(
                elts.iter()
                    .map(|e| self.instantiate_inner(e, fresh))
                    .collect(),
            ),
            TypeExpr::Unit
            | TypeExpr::Bool
            | TypeExpr::Char
            | TypeExpr::Int
            | TypeExpr::String
            | TypeExpr::UnifVar(_) => tp.clone(),
        }
    }

    // close
    fn generalize(&mut self, tp: TypeExpr) -> Scheme {
        self.generalize_at_level(tp, self.current_level)
    }

    fn generalize_at_level(&mut self, tp: TypeExpr, level: i64) -> Scheme {
        let mut generalized = HashMap::new();
        let tp = self.generalize_inner(tp, level, &mut generalized);
        Scheme {
            arity: u32::try_from(generalized.len()).unwrap(),
            tp,
        }
    }

    fn generalize_inner(
        &mut self,
        tp: TypeExpr,
        level: i64,
        generalized: &mut HashMap<u32, i64>,
    ) -> TypeExpr {
        match self.resolve(tp) {
            TypeExpr::UnifVar(id) => {
                let idx = usize::try_from(id).unwrap();
                if self.levels[idx] > level {
                    let next = i64::try_from(generalized.len()).unwrap();
                    TypeExpr::Var(*generalized.entry(id).or_insert(next), self.eq[idx])
                } else {
                    TypeExpr::UnifVar(id)
                }
            }
            TypeExpr::Arrow(src, tgt) => TypeExpr::Arrow(
                Rc::new(self.generalize_inner((*src).clone(), level, generalized)),
                Rc::new(self.generalize_inner((*tgt).clone(), level, generalized)),
            ),
            TypeExpr::Named(id, args) => TypeExpr::Named(
                id,
                args.iter()
                    .map(|e| self.generalize_inner(e.clone(), level, generalized))
                    .collect(),
            ),
            TypeExpr::Product(elts) => TypeExpr::Product(
                elts.iter()
                    .map(|e| self.generalize_inner(e.clone(), level, generalized))
                    .collect(),
            ),
            other => other,
        }
    }

    fn lower_levels(&mut self, tp: TypeExpr, max_level: i64) {
        match self.resolve(tp) {
            TypeExpr::UnifVar(id) => {
                let idx = usize::try_from(id).unwrap();
                let level = &mut self.levels[idx];
                *level = (*level).min(max_level);
            }
            TypeExpr::Arrow(src, tgt) => {
                self.lower_levels((*src).clone(), max_level);
                self.lower_levels((*tgt).clone(), max_level);
            }
            TypeExpr::Named(_, elts) | TypeExpr::Product(elts) => {
                elts.iter()
                    .for_each(|e| self.lower_levels((*e).clone(), max_level));
            }
            TypeExpr::Unit
            | TypeExpr::Bool
            | TypeExpr::Char
            | TypeExpr::Int
            | TypeExpr::String
            | TypeExpr::Var(_, _) => {}
        }
    }

    fn occurs(&mut self, id: u32, tp: TypeExpr) -> bool {
        match self.resolve(tp) {
            TypeExpr::Unit
            | TypeExpr::Bool
            | TypeExpr::Char
            | TypeExpr::Int
            | TypeExpr::String
            | TypeExpr::Var(_, _) => false,
            TypeExpr::UnifVar(tp_id) => id == tp_id,
            TypeExpr::Arrow(src, tgt) => {
                self.occurs(id, (*src).clone()) || self.occurs(id, (*tgt).clone())
            }
            TypeExpr::Named(_, elts) | TypeExpr::Product(elts) => {
                elts.iter().any(|e| self.occurs(id, (*e).clone()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::front_end;

    fn check(source: &str) -> Result<String> {
        let (program, decls) = front_end(source);
        let mut checker = TypeChecker::new();
        checker.add_types(&decls);
        checker.infer(&program).map(|scheme| scheme.to_string())
    }

    fn arrow(left: TypeExpr, right: TypeExpr) -> TypeExpr {
        TypeExpr::Arrow(Rc::new(left), Rc::new(right))
    }

    fn bound_variable(index: i64) -> ast::Expr {
        ast::Expr {
            kind: ast::ExprKind::BoundVariable(index),
            span: (0..1).into(),
        }
    }

    #[test]
    fn infers_expressions() {
        assert_eq!(check("()").unwrap(), "Unit");
        assert_eq!(check("true").unwrap(), "Bool");
        assert_eq!(check("1").unwrap(), "Int");
        assert_eq!(check("-1 + 2 * 3").unwrap(), "Int");
        assert_eq!(check("1 < 2 && !false").unwrap(), "Bool");
        assert_eq!(check("() == ()").unwrap(), "Bool");
        assert_eq!(check("true == true").unwrap(), "Bool");
        assert_eq!(check("'a' == 'a'").unwrap(), "Bool");
        assert_eq!(check("1 == 1").unwrap(), "Bool");
        assert_eq!(check(r#""abc" == "abc""#).unwrap(), "Bool");
        assert_eq!(check("1 == (fun _ => 2) ()").unwrap(), "Bool");
        assert_eq!(check("1; true").unwrap(), "Bool");
        assert_eq!(check("fun x => x").unwrap(), "(∀α.α -> α)");
        assert_eq!(check("fun x => fun y => x").unwrap(), "(∀α.∀β.α -> β -> α)");
        assert_eq!(check("(fun x => x) 1").unwrap(), "Int");
        assert_eq!(check("fun x => x + 1").unwrap(), "Int -> Int");
        assert_eq!(
            check("fun f => fun x => f (f x)").unwrap(),
            "(∀α.(α -> α) -> α -> α)"
        );
        assert_eq!(
            check("fun x => fun y => fun z => x z (y z)").unwrap(),
            "(∀α.∀β.∀γ.(α -> β -> γ) -> (α -> β) -> α -> γ)"
        );
        assert_eq!(check("fun b => if b then 1 else 2").unwrap(), "Bool -> Int");
        assert_eq!(check("let x = 1; x + x").unwrap(), "Int");
        assert_eq!(check("let id = fun x => x; id 1; id true").unwrap(), "Bool");
        assert_eq!(
            check("let k = fun x => fun y => x; k 1").unwrap(),
            "(∀α.α -> Int)"
        );
        // y should not generalize since x is outside of let
        assert_eq!(
            check("fun x => { let y = x; y + 1; y }").unwrap(),
            "Int -> Int"
        );
        assert_eq!(
            check("let rec f = fun n => if n < 2 then 1 else n * f (n - 1); f").unwrap(),
            "Int -> Int"
        );
        assert_eq!(
            check("let rec id = fun x => x; id 1; id true").unwrap(),
            "Bool"
        );
        assert_eq!(
            check("let rec loop = fun x => loop x; loop").unwrap(),
            "(∀α.∀β.α -> β)"
        );
        assert_eq!(
            check("fun x => fun y => x == y").unwrap(),
            "(∀α.''α -> ''α -> Bool)"
        );
        assert_eq!(check("(1, 2)").unwrap(), "(Int, Int)");
        assert_eq!(check("((), ((), ()))").unwrap(), "(Unit, (Unit, Unit))");
        assert_eq!(
            check("(fun x => x, fun y, z => (y, z))").unwrap(),
            "(∀α.∀β.∀γ.(α -> α, β -> γ -> (β, γ)))"
        );
        assert_eq!(
            check("let y = 1; let f = fun z => (y, z); f").unwrap(),
            "(∀α.α -> (Int, α))"
        );
        assert_eq!(check("(1, 2) == (3, 4)").unwrap(), "Bool");
        assert_eq!(
            check("let (x, y) = (1, 2); (x, y) == (3, 4)").unwrap(),
            "Bool"
        );
        assert_eq!(
            check("let (x, (y, z)) = (1, (2, 3)); x + y - z").unwrap(),
            "Int"
        );
        assert_eq!(
            check("let f = (fun x => let (y, z) = x; z); f").unwrap(),
            "(∀α.∀β.(α, β) -> β)"
        );
        assert_eq!(
            check("let f = fun x => x; (f 1, f \"a\")").unwrap(),
            "(Int, String)"
        );
        assert_eq!(
            check("let (f, n) = (fun x => x, 1); (f 1, f \"a\")").unwrap(),
            "(Int, String)"
        );
    }

    #[test]
    fn infers_constructors() {
        let check = |source| {
            check(&format!(
                "type Option a = None | Some a;
                 type List a = Nil | Cons a (List a);
                 type Pair = Pair Int Int;
                 {source}"
            ))
        };
        assert_eq!(check("Option::None").unwrap(), "(∀α.T_0 α)");
        assert_eq!(check("Option::Some").unwrap(), "(∀α.α -> T_0 α)");
        assert_eq!(check("Option::Some 1").unwrap(), "T_0 Int");
        assert_eq!(check("Pair::Pair 1").unwrap(), "Int -> T_2");
        assert_eq!(
            check("List::Cons 1 (List::Cons 2 List::Nil)").unwrap(),
            "T_1 Int"
        );
        assert_eq!(
            check("let none = Option::None; (Option::Some 1, none, Option::Some true, none)")
                .unwrap(),
            "(∀α.∀β.(T_0 Int, T_0 α, T_0 Bool, T_0 β))"
        );
        assert_eq!(
            check("Option::Some (Option::Some 1)").unwrap(),
            "T_0 (T_0 Int)"
        );
        assert_eq!(
            check("Option::Some (fun x => x)").unwrap(),
            "(∀α.T_0 (α -> α))"
        );

        for source in [
            "Pair::Pair true",
            "Option::Some 1 + 1",
            "List::Cons 1 (List::Cons true List::Nil)",
            "List::Cons 1 Option::None",
            "let f = fun x => x; (f List::Nil, f Option::None) ; List::Cons 1 Option::None",
        ] {
            assert!(
                matches!(check(source), Err(TypeError::Unification { .. })),
                "{source}"
            );
        }

        // equality
        assert_eq!(check("Option::None == Option::None").unwrap(), "Bool");
        assert_eq!(check("Option::Some 1 == Option::Some 2").unwrap(), "Bool");
        assert_eq!(
            // has to unify List Int with ∀α. (List α)
            check("List::Cons 1 List::Nil == List::Nil").unwrap(),
            "Bool"
        );
        // mutual recursion
        assert_eq!(
            check(
                "type A = A B | X;
                 type B = B A | Y;
                 A::X == A::X"
            )
            .unwrap(),
            "Bool"
        );

        for source in [
            "type F a = F (a -> a);
             let f = fun x => x;
             F::F f == F::F f",
            "type G = G (Option (Int -> Int));
             G::G Option::None == G::G Option::None",
            "type A = A B | X;
             type B = B A (Int -> Int);
             A::X == A::X",
        ] {
            assert!(
                matches!(check(source), Err(TypeError::Equality { .. })),
                "{source}"
            );
        }
    }

    #[test]
    fn pattern_matching() {
        assert_eq!(check("match 1 { 1 => true, _ => false }").unwrap(), "Bool");
        assert_eq!(
            check(r#"match (1, "a") { (x, y) => y }"#).unwrap(),
            "String"
        );
        assert_eq!(
            check("match (1, 2) { (0, y) => y, (x, _) => x }").unwrap(),
            "Int"
        );
        assert_eq!(check("match { 0 => 1, n => n * 2 }").unwrap(), "Int -> Int");
        assert_eq!(check("fun x => match x { y => y }").unwrap(), "(∀α.α -> α)");
        assert_eq!(
            check("match { (x, y) => (y, x) }").unwrap(),
            "(∀α.∀β.(α, β) -> (β, α))"
        );
        // pattern variable shadows the outer x
        assert_eq!(
            check(r#"let x = 1; match "a" { x => x }"#).unwrap(),
            "String"
        );
        // arm bindings are popped afterwards
        assert_eq!(
            check(r#"let x = 1; (match "a" { y => y }, x)"#).unwrap(),
            "(String, Int)"
        );
        assert_eq!(
            check(
                "type Either a b = Left a | Right b;
                 match Either::Left 1 { Either::Left n => n }"
            )
            .unwrap(),
            "Int"
        );
        assert_eq!(
            check("type A a = A a; fun x => match x { A::A x => x }").unwrap(),
            "(∀α.T_0 α -> α)"
        );
        assert_eq!(
            check(
                "type List t = Nil | Cons t (List t);
                 fun x => match x {
                    List::Cons a List::Nil => a,
                    List::Cons a (List::Cons b List::Nil) => a + b,
                    List::Cons a (List::Cons b (List::Cons c List::Nil)) => a + b + c,
                 }"
            )
            .unwrap(),
            "T_0 Int -> Int"
        );
        assert_eq!(
            check(
                "type List t = Nil | Cons t (List t);
                 fun x => match x {
                    List::Cons (a, b, c) (List::Cons (d, e, f) List::Nil) => a + e,
                 }"
            )
            .unwrap(),
            "(∀α.T_0 (Int, Int, α) -> Int)"
        );
    }

    #[test]
    fn type_errors() {
        assert!(matches!(
            check("1 + true"),
            Err(TypeError::Unification { .. })
        ));
        assert!(matches!(check("!1"), Err(TypeError::Unification { .. })));
        assert!(matches!(check("1 2"), Err(TypeError::Unification { .. })));
        assert!(matches!(
            check("if 1 then 2 else 3"),
            Err(TypeError::Unification { .. })
        ));
        assert!(matches!(
            check("if true then 1 else false"),
            Err(TypeError::Unification { .. })
        ));
        // lambda-bound variables are monomorphic
        assert!(matches!(
            check("(fun id => { id 1; id true }) (fun x => x)"),
            Err(TypeError::Unification { .. })
        ));
        // recursive calls are monomorphic
        assert!(matches!(
            check("let rec f = fun x => { f 1; f true; x }; f"),
            Err(TypeError::Unification { .. })
        ));
        assert!(matches!(
            check("fun x => x x"),
            Err(TypeError::OccursCheck { .. })
        ));
        assert!(matches!(
            check("let rec f = fun x => f; f"),
            Err(TypeError::OccursCheck { .. })
        ));
        assert!(matches!(
            check("(fun x => x) == (fun y => y)"),
            Err(TypeError::Equality { .. })
        ));
        // infers f as arrow type, so equality fails
        assert!(matches!(
            check("fun f => fun x => { f x; f == f }"),
            Err(TypeError::Equality { .. })
        ));
        assert!(matches!(
            check("(1, fun x => x) == (1, fun y => y)"),
            Err(TypeError::Equality { .. })
        ));
        assert!(matches!(
            check("match 1 { 1 => true, _ => 2 }"),
            Err(TypeError::MatchArms { .. })
        ));
        // pattern doesn't fit the scrutinee
        assert!(matches!(
            check(r#"match 1 { "a" => 1 }"#),
            Err(TypeError::Unification { .. })
        ));
        assert!(matches!(
            check("match (1, 2) { (x, y, z) => x }"),
            Err(TypeError::Unification { .. })
        ));
        // match-bound variables are monomorphic
        assert!(matches!(
            check("match (fun x => x) { f => (f 1, f true) }"),
            Err(TypeError::Unification { .. })
        ));
    }

    #[test]
    fn resolves_chains() {
        let mut checker = TypeChecker::new();
        let a = checker.new_unification_variable(false);
        let b = checker.new_unification_variable(false);
        let c = checker.new_unification_variable(false);
        checker.substitution[0] = Some(b.clone());
        checker.substitution[1] = Some(c.clone());
        checker.substitution[2] = Some(arrow(TypeExpr::Int, TypeExpr::Bool));

        assert_eq!(checker.resolve(a).to_string(), "Int -> Bool");
        assert_eq!(checker.substitution[0], checker.substitution[2]);
        assert_eq!(checker.substitution[1], checker.substitution[2]);

        let unbound = checker.new_unification_variable(false);
        assert_eq!(checker.resolve(unbound.clone()), unbound);
        assert_eq!(checker.resolve(TypeExpr::Int), TypeExpr::Int);
    }

    #[test]
    fn unbound_variables() {
        assert!(matches!(
            check("foo"),
            Err(TypeError::UnboundVariable { name, .. }) if name == "foo"
        ));
        assert!(matches!(
            TypeChecker::new().algorithm_j(&bound_variable(0)),
            Err(TypeError::UnboundIndex { index: 0, .. })
        ));
        assert!(matches!(
            TypeChecker::new().algorithm_j(&bound_variable(-1)),
            Err(TypeError::UnboundIndex { index: -1, .. })
        ));
    }

    #[test]
    fn bad_constructors() {
        let mut checker = TypeChecker::new();
        checker.add_types(&[ast::TypeDecl {
            name: "Either".to_string(),
            arity: 2,
            constructors: vec![
                ("Left".to_string(), vec![TypeExpr::Var(0, false)]),
                ("Right".to_string(), vec![TypeExpr::Var(1, false)]),
            ],
        }]);

        assert!(matches!(
            checker.algorithm_j(&ast::Expr {
                kind: ast::ExprKind::Constructor(TypeId(1), 0, vec![]),
                span: (0..0).into(),
            }),
            Err(TypeError::UnknownConstructor { .. })
        ));
        assert!(matches!(
            checker.algorithm_j(&ast::Expr {
                kind: ast::ExprKind::Constructor(TypeId(0), 2, vec![]),
                span: (0..0).into(),
            }),
            Err(TypeError::UnknownConstructor { .. })
        ));
        // too many arguments
        assert!(matches!(
            checker.algorithm_j(&ast::Expr {
                kind: ast::ExprKind::Constructor(
                    TypeId(0),
                    0,
                    vec![
                        ast::Expr {
                            kind: ast::ExprKind::Literal(ast::Literal::Int(0)),
                            span: (0..0).into()
                        },
                        ast::Expr {
                            kind: ast::ExprKind::Literal(ast::Literal::Int(1)),
                            span: (0..0).into()
                        }
                    ]
                ),
                span: (0..0).into(),
            }),
            Err(TypeError::Unification { .. })
        ));
        // too few arguments is not an error, but should not actually happen in practice
        let tp = checker
            .algorithm_j(&ast::Expr {
                kind: ast::ExprKind::Constructor(TypeId(0), 0, vec![]),
                span: (0..0).into(),
            })
            .unwrap();
        let tp = checker.resolve(tp);
        assert_eq!(
            checker.generalize_at_level(tp, -1).to_string(),
            "(∀α.∀β.α -> T_0 α β)"
        );
    }

    #[test]
    fn bad_patterns() {
        assert!(matches!(
            check(
                "type Option a = None | Some a;
                 type List t = Nil | Cons t (List t);
                 match Option::Some 1 { List::Cons x _ => x, _ => 0 }"
            ),
            Err(TypeError::Unification { .. })
        ));

        // the elaborator rejects the rest, so build them directly
        let mut checker = TypeChecker::new();
        checker.add_types(&[ast::TypeDecl {
            name: "Option".to_string(),
            arity: 1,
            constructors: vec![
                ("None".to_string(), vec![]),
                ("Some".to_string(), vec![TypeExpr::Var(0, false)]),
            ],
        }]);
        let pattern = |kind| ast::Pattern {
            kind,
            span: (0..1).into(),
        };
        let expr = |kind| ast::Expr {
            kind,
            span: (0..1).into(),
        };
        let one = || expr(ast::ExprKind::Literal(ast::Literal::Int(1)));
        let some_one = || expr(ast::ExprKind::Constructor(TypeId(0), 1, vec![one()]));
        let some = |patterns| pattern(ast::PatternKind::Constructor(TypeId(0), 1, patterns));
        let none = |patterns| pattern(ast::PatternKind::Constructor(TypeId(0), 0, patterns));
        let variable = || pattern(ast::PatternKind::Variable);

        // let Option::Some x = Option::Some 1; x
        assert!(matches!(
            checker.algorithm_j(&expr(ast::ExprKind::LetIn(
                false,
                some(vec![variable()]),
                Box::new(some_one()),
                Box::new(bound_variable(0)),
            ))),
            Err(TypeError::PatternContainsConstructors { .. })
        ));
        // let (Option::Some x, y) = (Option::Some 1, 1); x
        assert!(matches!(
            checker.algorithm_j(&expr(ast::ExprKind::LetIn(
                false,
                pattern(ast::PatternKind::Tuple(vec![
                    some(vec![variable()]),
                    variable()
                ])),
                Box::new(expr(ast::ExprKind::Tuple(vec![some_one(), one()]))),
                Box::new(bound_variable(1)),
            ))),
            Err(TypeError::PatternContainsConstructors { .. })
        ));
        // let rec Option::Some x = Option::Some 1; x
        assert!(matches!(
            checker.algorithm_j(&expr(ast::ExprKind::LetIn(
                true,
                some(vec![variable()]),
                Box::new(some_one()),
                Box::new(bound_variable(0)),
            ))),
            Err(TypeError::PatternContainsConstructors { .. })
        ));

        // match Option::Some 1 { <pattern> => 1 }
        let arity = |checker: &mut TypeChecker, p| match checker.algorithm_j(&expr(
            ast::ExprKind::Match(Box::new(some_one()), vec![(p, one())]),
        )) {
            Err(TypeError::WrongConstructorArity {
                expected, actual, ..
            }) => Some((expected, actual)),
            _ => None,
        };
        // Option::Some
        assert_eq!(arity(&mut checker, some(vec![])), Some((1, 0)));
        // Option::Some x y
        assert_eq!(
            arity(&mut checker, some(vec![variable(), variable()])),
            Some((1, 2))
        );
        // Option::None x
        assert_eq!(arity(&mut checker, none(vec![variable()])), Some((0, 1)));
    }
}
